use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use alloc::boxed::Box;
use alloc::vec::Vec;
use x86_64::VirtAddr;
use x86_64::registers::model_specific::{GsBase, Msr};
use x86_64::structures::gdt::GlobalDescriptorTable;
use x86_64::structures::tss::TaskStateSegment;

use crate::descriptors::Selectors;
use crate::helpers::{Time, wait_timeout};
use crate::memory::Stack;
use crate::{InterruptMutex, KResult, LateInit, MP_RESPONSE};
use crate::scheduling::{Scheduler, Task};

mod init;
pub mod ipi;

pub static CPUS: LateInit<Vec<CPUData>> = LateInit::new();

#[derive(Debug)]
pub struct CPUData {
    pub id: u64,
    pub lapic_id: u64,
    pub gdt: Box<GlobalDescriptorTable<64>>,
    pub tss: Box<TaskStateSegment>,
    pub selectors: Selectors,
    pub scheduler: InterruptMutex<Scheduler>,
    pub idle: Task,
    pub priv_stack: Stack,
    pub ipi_ack: AtomicBool,
    pub tlb_addresses: InterruptMutex<Vec<VirtAddr>>,
    pub syscall_stack: VirtAddr
}

static ONLINE_APS: AtomicU64 = AtomicU64::new(0);

/// Fetch the number of CPUs
/// 
/// # Panics
/// Panics if the static `CPUS` has not been initialized
pub fn cpu_count() -> usize { CPUS.len() }

/// Fetch the data of the current CPU
/// 
/// # Panics
/// Panics if the static `CPUS` has not been initialized
/// or the current CPU was unaccounted for
pub fn current_cpu() -> &'static CPUData {
    let ptr = GsBase::read();
    if ptr.is_null() {
        // Safety: Just an MSR read
        let id = unsafe { Msr::new(0x802).read() };

        return CPUS.iter().find(|c| c.lapic_id == id).expect("Invalid CPU Data!")
    }

    // Safety: Deref of a validated pointer. Safe
    unsafe { &*ptr.as_ptr() }
}

pub fn set_rsp0(sp: VirtAddr) {
    let ptr = &raw const *current_cpu() as usize as *mut CPUData;

    // Safety: Dereference of a safe pointer. Safe
    let reference = unsafe { &mut *ptr };
    reference.tss.privilege_stack_table[0] = sp;
}

pub fn set_syscall_stack(sp: VirtAddr) {
    let ptr = &raw const *current_cpu() as usize as *mut CPUData;

    let reference = unsafe { &mut *ptr };
    reference.syscall_stack = sp;
}

pub fn init() -> KResult<()> {
    init::init_bsp()
}

pub fn boot() {
    for cpu in MP_RESPONSE.cpus() {
        if cpu.lapic_id == MP_RESPONSE.bsp_lapic_id { continue } // Skip BSP

        let idx = CPUS.iter().position(|c| c.lapic_id == u64::from(cpu.lapic_id)).expect("Invalid CPU Data!");
        let addr = &raw const CPUS[idx] as u64;

        cpu.bootstrap(init::ap_entry, addr);
    }

    let total_aps = (CPUS.len() - 1) as u64;
    let timeout = Time::Seconds(5);

    let status = wait_timeout(|| ONLINE_APS.load(Ordering::SeqCst) < total_aps, &timeout);

    assert!(status, "Failed to initialize APs in 5 seconds!");
}