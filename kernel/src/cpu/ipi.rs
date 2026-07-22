//! Inter-Processor Interrupt (IPI) management.
//!
//! Provides routines for sending IPIs (wake, TLB shootdown, function call, halt)
//! to individual or groups of CPUs via the local APIC (xAPIC or x2APIC).
//!
//! Also manages per-CPU IPI data and the idle-CPU tracking used by the
//! scheduler for load balancing.
//!
//! # Safety
//! IPI delivery relies on MMIO (xAPIC) or MSR (x2APIC) register access.
//! Handler functions run in interrupt context and must not re-enter the
//! scheduler or allocate memory.

use crate::acpi::lapic_regs as regs;
use crate::acpi::{ICR_HIGH_REG, ICR_LOW_REG, X2APIC_ICR_MSR, is_x2apic};
use crate::boot::requests::MP_DATA;
use crate::descriptors::interrupts::add_idt_entry;
use crate::library::{LateInit, Time};
use crate::halt_loop;

use alloc::boxed::Box;
use core::arch::{asm, naked_asm};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use x86_64::VirtAddr;
use x86_64::structures::idt::InterruptStackFrame;

const IPI_WAKE: u8 = 32;
const IPI_TLB: u8 = 33;
const IPI_FUNC: u8 = 34;
const IPI_HALT: u8 = 35;

/// Bitmask tracking which CPUs are currently idle.
/// Each bit `1 << cpu_id` indicates that CPU is idle.
pub static IDLE_CPUS: AtomicU64 = AtomicU64::new(0);

/// Timestamp (nanos) of the last load-balance operation.
pub static LAST_REBALANCE: AtomicU64 = AtomicU64::new(0);

/// Delivery mode for the ICR (Interrupt Command Register).
#[derive(Clone, Copy)]
#[repr(u8)]
#[allow(dead_code)]
pub enum IcrDeliveryMode {
    Fixed = 0b000,
    Init = 0b101,
}

/// Shorthand for the ICR destination field.
#[derive(Clone, Copy)]
#[repr(u8)]
#[allow(dead_code)]
pub enum IcrDestinationShorthand {
    NoShorthand = 0b00,
    AllIncludingSelf = 0b10,
    AllExcludingSelf = 0b11,
}

/// Send an IPI to one or more CPUs via the local APIC ICR.
///
/// Supports both xAPIC (MMIO) and x2APIC (MSR) delivery.
pub fn send_ipi(vector: u8, dest_lapic_id: Option<u32>, shorthand: IcrDestinationShorthand, mode: IcrDeliveryMode) {
    if is_x2apic() {
        let mut icr = vector as u64 | ((mode as u64) << 8) | ((shorthand as u64) << 18);
        if let Some(id) = dest_lapic_id { icr |= (id as u64) << 32 }
        unsafe { asm!("wrmsr", in("ecx") X2APIC_ICR_MSR, in("eax") icr as u32, in("edx") (icr >> 32) as u32) }
    } else {
        let base = regs();
        let icr_high = match shorthand {
            IcrDestinationShorthand::NoShorthand => dest_lapic_id.unwrap_or(0) << 24,
            _ => 0,
        };

        unsafe { (base + ICR_HIGH_REG).as_mut_ptr::<u32>().write_volatile(icr_high) }

        let icr_low = vector as u32 | ((mode as u32) << 8) | ((shorthand as u32) << 18);

        unsafe { (base + ICR_LOW_REG).as_mut_ptr::<u32>().write_volatile(icr_low) }
    }
}

/// Spin until the xAPIC ICR indicates the previous IPI has been delivered.
fn wait_for_idle() {
    if !is_x2apic() {
        let base = regs();
        while unsafe { (base + ICR_LOW_REG).as_ptr::<u32>().read_volatile() & (1 << 12) != 0 } {
            core::hint::spin_loop();
        }
    }
}

/// Per-CPU state for IPI handlers.
#[derive(Debug)]
struct IpiCpuData {
    func: Option<fn()>,
    tlb_addr: u64,
    ack: AtomicBool,
}

static IPI_DATA: LateInit<Box<[IpiCpuData]>> = LateInit::new();

/// Returns the CPU index for a given LAPIC ID.
fn position(lapic_id: u32) -> usize {
    for (i, cpu) in MP_DATA.cpus().iter().enumerate() {
        if cpu.lapic_id == lapic_id { return i }
    }
    panic!("IPI data position not found for LAPIC {}", lapic_id);
}

/// Returns a mutable pointer to the IPI data for a given LAPIC ID.
fn data_mut(lapic_id: u32) -> *mut IpiCpuData {
    &mut IPI_DATA.get_mut()[position(lapic_id)]
}

/// Returns a mutable pointer to the calling CPU's IPI data.
fn my_data() -> *mut IpiCpuData {
    data_mut(crate::acpi::lapic_id())
}

/// Convert a logical CPU ID (0 = BSP, 1..N = APs) to a LAPIC ID.
fn cpu_lapic_id(cpu_id: u64) -> u32 {
    if cpu_id == 0 { return crate::acpi::lapic_id() }

    let my = crate::acpi::lapic_id();
    let mut idx = 1u64;
    for cpu in MP_DATA.cpus().iter() {
        if cpu.lapic_id == my { continue }
        if idx == cpu_id { return cpu.lapic_id }
        idx += 1;
    }
    panic!("CPU {} not found in MP data", cpu_id);
}

/// IPI wake handler — immediately jumps into the scheduler so the AP can pick
/// up a newly migrated task.
#[unsafe(naked)]
extern "x86-interrupt" fn ipiwake_handler(_sf: InterruptStackFrame) {
    naked_asm!("jmp {}", sym crate::scheduling::schedule)
}

/// IPI TLB shootdown handler — invalidates a single page on this CPU.
extern "x86-interrupt" fn ipitlb_handler(_sf: InterruptStackFrame) {
    let d = my_data();
    let addr = unsafe { (*d).tlb_addr };
    unsafe {
        asm!("invlpg [{}]", in(reg) addr, options(nostack, preserves_flags));
        (*d).ack.store(true, Ordering::SeqCst);
    }
    crate::acpi::lapic_eoi();
}

/// IPI function-call handler — runs a pre-registered closure on this CPU.
extern "x86-interrupt" fn ipifunc_handler(_sf: InterruptStackFrame) {
    let d = my_data();
    if let Some(f) = unsafe { (*d).func.take() } { f() }
    unsafe { (*d).ack.store(true, Ordering::SeqCst) }
    crate::acpi::lapic_eoi();
}

/// IPI halt handler — puts this CPU into an infinite HLT loop.
extern "x86-interrupt" fn ipihalt_handler(_sf: InterruptStackFrame) {
    crate::acpi::lapic_eoi();
    halt_loop()
}

/// Initialise the IPI subsystem.
///
/// Allocates per-CPU data, maps the local APIC registers, and installs
/// the four IPI IDT entries (wake, TLB, func, halt).
pub fn init() {
    let cpu_count = MP_DATA.cpus().len();

    let data: Box<[IpiCpuData]> = (0..cpu_count)
        .map(|_| IpiCpuData {
            func: None,
            tlb_addr: 0,
            ack: AtomicBool::new(false),
        })
        .collect::<alloc::vec::Vec<_>>()
        .into_boxed_slice();

    IPI_DATA.init(data);

    add_idt_entry(ipiwake_handler, IPI_WAKE);
    add_idt_entry(ipitlb_handler, IPI_TLB);
    add_idt_entry(ipifunc_handler, IPI_FUNC);
    add_idt_entry(ipihalt_handler, IPI_HALT);
}

/// Send a reschedule IPI to a specific CPU (0 = BSP, 1..N = APs)
pub fn sched_kick(cpu_id: u64) {
    send_ipi(
        IPI_WAKE,
        Some(cpu_lapic_id(cpu_id)),
        IcrDestinationShorthand::NoShorthand,
        IcrDeliveryMode::Fixed,
    );
}

/// Flush a virtual address from all other CPUs' TLBs
pub fn tlb_shootdown(addr: VirtAddr) {
    let cpu_count = MP_DATA.cpus().len();
    if cpu_count <= 1 { return }
    if IPI_DATA.try_get().is_none() { return }

    let my = crate::acpi::lapic_id();

    // 1. Set up data for each target CPU
    for cpu in MP_DATA.cpus().iter() {
        if cpu.lapic_id == my { continue }
        let d = data_mut(cpu.lapic_id);
        unsafe {
            (*d).tlb_addr = addr.as_u64();
            (*d).ack.store(false, Ordering::SeqCst);
        }
    }

    // 2. Send IPI
    send_ipi(
        IPI_TLB,
        None,
        IcrDestinationShorthand::AllExcludingSelf,
        IcrDeliveryMode::Fixed,
    );
    wait_for_idle();

    // 3. Wait for acks with timeout
    for cpu in MP_DATA.cpus().iter() {
        if cpu.lapic_id == my { continue }
        let d = data_mut(cpu.lapic_id);
        if !wait_ack(unsafe { &(*d).ack }) {
            warn!(
                "TLB shootdown: CPU {} (LAPIC {}) did not ack",
                position(cpu.lapic_id),
                cpu.lapic_id
            );
        }
    }
}

/// Halt all other CPUs (for panic propagation)
pub fn halt_other_cpus() {
    if MP_DATA.cpus().len() <= 1 { return }
    if IPI_DATA.try_get().is_none() { return}

    send_ipi(
        IPI_HALT,
        None,
        IcrDestinationShorthand::AllExcludingSelf,
        IcrDeliveryMode::Fixed,
    );
}

/// Set this CPU's idle bit and notify the BSP
pub fn notify_bsp_idle() {
    let id = crate::cpu::cpu_id();
    let prev = IDLE_CPUS.fetch_or(1 << id, Ordering::SeqCst);
    if prev & (1 << id) == 0 {
        sched_kick(0); // only IPI if first time entering idle
    }
}

/// Spin-wait on an `ack` flag with a ~100 ms timeout.
/// Returns `true` if the ack was received, `false` on timeout.
fn wait_ack(ack: &AtomicBool) -> bool {
    let deadline = crate::acpi::passed_nanos() + Time::Milliseconds(100).to_nanos();
    while crate::acpi::passed_nanos() < deadline {
        if ack.load(Ordering::SeqCst) {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}
