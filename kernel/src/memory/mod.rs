//! Kernel Memory Module

use x86_64::{PhysAddr, VirtAddr, registers::control::Cr3, structures::paging::{PageSize, PhysFrame, Size4KiB}};
use crate::{HHDM_RESPONSE, KernelResult, LateInit};

mod physical;
mod wrappers;
mod heap;
mod virt;
mod user;

pub const PAGE_SIZE: usize = Size4KiB::SIZE as usize;

pub static KERNEL_L4: LateInit<PhysFrame> = LateInit::new();
pub static HHDM_OFFSET: LateInit<u64> = LateInit::new();

pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr { VirtAddr::new(phys.as_u64() + *HHDM_OFFSET) }
unsafe fn virt_to_phys(virt: VirtAddr) -> PhysAddr { PhysAddr::new(virt.as_u64() - *HHDM_OFFSET) }


pub fn init() -> KernelResult<()> {
    HHDM_OFFSET.init(HHDM_RESPONSE.offset);

    physical::init();
    heap::init()?;
    virt::init();

    KERNEL_L4.init(Cr3::read().0);

    Ok(())
}

pub use wrappers::*;