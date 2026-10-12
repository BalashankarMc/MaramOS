use alloc::boxed::Box;
use alloc::vec::Vec;
use x86_64::VirtAddr;
use x86_64::registers::model_specific::Msr;
use x86_64::structures::gdt::GlobalDescriptorTable;
use x86_64::structures::tss::TaskStateSegment;

use crate::descriptors::Selectors;
use crate::{InterruptMutex, KResult, LateInit, MP_RESPONSE};
use crate::scheduling::{Scheduler, Task};

mod init;

pub static CPUS: LateInit<Vec<CPUData>> = LateInit::new();

#[derive(Debug,Clone)]
pub struct CPUData {
    pub id: u64,
    pub lapic_id: u64,
    pub gdt: Box<GlobalDescriptorTable<64>>,
    pub tss: Box<TaskStateSegment>,
    pub selectors: Selectors,
    pub scheduler: InterruptMutex<Scheduler>,
    pub idle: Task
}

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
    // Safety: Just a MSR read
    let id = unsafe { Msr::new(0x802).read() };

    CPUS.iter().find(|d| d.lapic_id == id).unwrap()
}

pub fn set_rsp0(sp: VirtAddr) {
    let ptr = &raw const *current_cpu() as usize as *mut CPUData;

    // Safety: Dereference of a safe pointer. Safe
    let reference = unsafe { &mut *ptr };
    reference.tss.privilege_stack_table[0] = sp;
}

pub fn init() -> KResult<()> {
    init::init_bsp()
}

pub fn boot() {
    for cpu in MP_RESPONSE.cpus() {
        if cpu.lapic_id == MP_RESPONSE.bsp_lapic_id { continue } // Skip BSP
        cpu.bootstrap(init::ap_entry, 0);
    }
}