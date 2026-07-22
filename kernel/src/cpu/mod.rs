//! Per-CPU state management.
//!
//! Provides the [`CPUInfo`] struct stored at `GsBase` for each CPU,
//! along with helper functions to read the current CPU ID and initialise
//! the BSP's CPU-info area.

use crate::{
    boot::requests::MP_DATA,
    descriptors::gdt::Selectors,
    library::LateInit,
    memory::{KMemory, PAGE_SIZE, PhysPage},
};

use x86_64::{
    VirtAddr,
    registers::model_specific::GsBase,
    structures::{gdt::GlobalDescriptorTable, tss::TaskStateSegment},
};

pub mod ipi;
pub mod smp;

static CPU_PAGE: LateInit<PhysPage> = LateInit::new();

/// Per-CPU data block mapped at `GsBase`.
///
/// Fields at specific offsets (0x00, 0x08, …) are accessed via `gs:`-relative
/// addressing in assembly trampolines.
#[repr(C)]
#[derive(Clone)]
pub struct CPUInfo {
    pub cr3_save: u64,   // gs:0x00
    pub kernel_rsp: u64, // gs:0x08
    pub id: u64,         // gs:0x10
    pub lapic_id: u32,   // gs:0x18
    _pad: u32,
    pub fpu_owner_id: Option<u64>,
    pub fpu_owner_active: bool,
    pub gdt: GlobalDescriptorTable<32>,
    pub selectors: Selectors,
    pub tss: TaskStateSegment,
}

/// Returns the ID of the currently executing CPU (0 = BSP).
///
/// # Safety
/// Reads `GsBase` and dereferences the `CPUInfo` pointer stored there.
/// This is safe once [`init_bsp_data`] (and `cpu::smp::init_aps` for APs)
/// have been called.
pub fn cpu_id() -> u64 {
    unsafe { (*GsBase::read().as_ptr::<CPUInfo>()).id }
}

/// Returns a pointer to the data of the currently executing CPU (0 = BSP).
///
/// # Safety
/// Reads `GsBase` and dereferences the `CPUInfo` pointer stored there.
/// This is safe once [`init_bsp_data`] (and `cpu::smp::init_aps` for APs)
/// have been called.
pub fn this_cpu() -> *mut CPUInfo {
    GsBase::read().as_mut_ptr::<CPUInfo>()
}

/// Initialise the BSP's CPU-info block.
///
/// Allocates pages for [`MP_DATA.cpus().len()`] `CPUInfo` entries, writes the
/// BSP's LAPIC ID and CPU ID (0), then sets `GsBase` to point at it.
pub fn init_bsp_data() {
    let cpus = MP_DATA.cpus().len();
    let pages = (cpus * core::mem::size_of::<CPUInfo>()).div_ceil(PAGE_SIZE);

    let page = KMemory::alloc_pages(pages);
    let cpu = unsafe { &mut *page.get_virt_addr().as_mut_ptr::<CPUInfo>() };
    cpu.lapic_id = crate::acpi::lapic_id();
    cpu.cr3_save = KMemory::kernel_l4().start_address().as_u64();
    cpu.id = 0;

    let virt = VirtAddr::new(cpu as *const _ as u64);
    GsBase::write(virt);

    CPU_PAGE.init(page);
    crate::scheduling::init_scheduler();
}
