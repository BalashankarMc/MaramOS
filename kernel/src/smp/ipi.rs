//! Inter-Processor Interrupt (IPI) management.
//!
//! Provides routines for sending IPIs (TLB shootdown, halt)
//! to individual or groups of CPUs via the local APIC (x2APIC).
//!
//! # Safety
//! IPI delivery relies on MSR (x2APIC) register access.
//! Handler functions run in interrupt context and must not re-enter the
//! scheduler or allocate memory.

use core::sync::atomic::Ordering;
use x86_64::{VirtAddr, registers::{control::{Cr4, Cr4Flags}, model_specific::Msr}, structures::idt::InterruptStackFrame};

use crate::{
    InterruptMutex, KResult,
    acpi::{lapic_eoi, lapic_id},
    descriptors::add_idt_entry,
    helpers::{Time, wait_timeout},
    log_warn
};

use self::IPITarget::AllExcludingSelf;
use super::{CPUS, ONLINE_APS, current_cpu};

// MSRs
const SELF_IPI_REGISTER          : u32 = 0x83F;
const ERROR_STATUS_REGISTER      : u32 = 0x828;
const SPURIOUS_VECTOR_REGISTER   : u32 = 0x80F;
const INTERRUPT_CONTROL_REGISTER : u32 = 0x830;

// IPI Lock (Ensures only one CPU can send IPIs)
static IPI_LOCK: InterruptMutex<()> = InterruptMutex::new(());

/// All IDT vectors used by the IPI subsystem
#[repr(u8)]
enum IPIVectors {
    TLBShootdown = 0xFF - 2,
    Halt,
    Spurious
}

#[repr(u8)]
enum DeliveryMode {
    Fixed   = 0b000,
    Nmi     = 0b100 // Non maskable
}

#[derive(Debug, Clone, Copy)]
enum IPITarget {
    Cpu(u32),
    AllExcludingSelf,
    All,
    SelfOnly
}

impl IPITarget {
    const fn icr_fields(self) -> (u64, u64) {
        match self {
            Self::Cpu(id) => (0, id as u64),
            Self::AllExcludingSelf => (0b11, 0),
            Self::SelfOnly => (0b01, 0),
            Self::All => (0b10, 0)
        }
    }
}

pub fn init() -> KResult<()> {
    add_idt_entry(ipi_halt_handler, IPIVectors::Halt as u8)?;
    add_idt_entry(ipi_tlb_handler, IPIVectors::TLBShootdown as u8)?;
    add_idt_entry(ipi_spurious_handler, IPIVectors::Spurious as u8)
}

pub fn tlb_shootdown(addresses: &[VirtAddr]) {
    let _lock = IPI_LOCK.lock();

    for &addr in addresses {
        // Safety: Safe assembly invocation
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) addr.as_u64(), options(nostack, preserves_flags));
        }
    }

    if CPUS.try_get().is_none() || CPUS.len() <= 1 { return }
    if addresses.is_empty() || ONLINE_APS.load(Ordering::Acquire) != CPUS.len() as u64 - 1 { return }

    let self_id = crate::acpi::lapic_id();

    for cpu in &*CPUS {
        if cpu.lapic_id == self_id { continue }

        let mut addrs = cpu.tlb_addresses.lock();
        addrs.clear();
        addrs.extend_from_slice(addresses);

        cpu.ipi_ack.store(false, Ordering::Release);
    }

    core::sync::atomic::fence(Ordering::SeqCst);

    send_ipi(IPIVectors::TLBShootdown, IPITarget::AllExcludingSelf, DeliveryMode::Fixed);
    wait_acks(self_id);
}

pub fn tlb_shoot_all() {
    let _lock = IPI_LOCK.lock();

    unsafe {
        Cr4::update(|f| f.remove(Cr4Flags::PAGE_GLOBAL));
        Cr4::update(|f| f.insert(Cr4Flags::PAGE_GLOBAL));
    }

    if CPUS.try_get().is_none() || CPUS.len() <= 1 { return }
    if ONLINE_APS.load(Ordering::Acquire) != CPUS.len() as u64 - 1 { return }

    let self_id = crate::acpi::lapic_id();

    for cpu in &*CPUS {
        if cpu.lapic_id == self_id { continue }

        cpu.tlb_addresses.lock().clear();
        cpu.ipi_ack.store(false, Ordering::Release);
    }

    core::sync::atomic::fence(Ordering::SeqCst);

    send_ipi(IPIVectors::TLBShootdown, IPITarget::AllExcludingSelf, DeliveryMode::Fixed);
    wait_acks(self_id);
}

pub fn halt_other_cpus() {
    let _lock = IPI_LOCK.lock();

    if CPUS.try_get().is_none() || CPUS.len() <= 1 { return }
    if ONLINE_APS.load(Ordering::Acquire) != CPUS.len() as u64 - 1 { return }

    let self_id = lapic_id();

    for cpu in &*CPUS {
        if cpu.lapic_id == self_id { continue }

        cpu.ipi_ack.store(false, Ordering::Release);
    }

    send_ipi(IPIVectors::Halt, AllExcludingSelf, DeliveryMode::Fixed);
    wait_acks(self_id);
}

fn send_ipi(vector: IPIVectors, destination: IPITarget, mode: DeliveryMode) {
    let fields = destination.icr_fields();
    let icr = vector as u64 | ((mode as u64) << 8) | (fields.0 << 18) | (fields.1 << 32);
    
    unsafe { Msr::new(INTERRUPT_CONTROL_REGISTER).write(icr) }
}

fn wait_acks(initiator_id: u64) {
    for cpu in &*CPUS {
        if cpu.lapic_id == initiator_id { continue }

        if !wait_timeout(|| !cpu.ipi_ack.load(Ordering::Acquire), &Time::Milliseconds(100)) {
            log_warn!("CPU {} did not ack!", cpu.id);
        }
    }
}

extern "x86-interrupt" fn ipi_halt_handler(_stack_frame: InterruptStackFrame) {
    lapic_eoi();
    current_cpu().ipi_ack.store(true, Ordering::Release);
    crate::halt_loop()
}

extern "x86-interrupt" fn ipi_tlb_handler(_stack_frame: InterruptStackFrame) {

    let addresses = &*current_cpu().tlb_addresses.lock();
    if addresses.is_empty() { // Flush all
        unsafe {
            Cr4::update(|f| f.remove(Cr4Flags::PAGE_GLOBAL));
            Cr4::update(|f| f.insert(Cr4Flags::PAGE_GLOBAL));
        }
    }

    for &addr in addresses {
        // Safety: Safe assembly invocation
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) addr.as_u64(), options(nostack, preserves_flags));
        }
    }
    current_cpu().ipi_ack.store(true, Ordering::Release);
    lapic_eoi();
}

#[allow(clippy::missing_const_for_fn)]
extern "x86-interrupt" fn ipi_spurious_handler(_stack_frame: InterruptStackFrame) {}