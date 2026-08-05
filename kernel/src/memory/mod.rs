//! Kernel Memory Module

use x86_64::{PhysAddr, VirtAddr, structures::paging::{PageSize, Size4KiB}};
use crate::{helpers::LateInit, requests::HHDM_REQUEST};

mod physical;
mod wrappers;
mod heap;
mod virt;

pub const PAGE_SIZE: usize = Size4KiB::SIZE as usize;

pub static HHDM_OFFSET: LateInit<u64> = LateInit::new();

fn phys_to_virt(phys: PhysAddr) -> VirtAddr { VirtAddr::new(phys.as_u64() + *HHDM_OFFSET) }
unsafe fn virt_to_phys(virt: VirtAddr) -> PhysAddr { PhysAddr::new(virt.as_u64() - *HHDM_OFFSET) }

#[derive(Debug)]
pub enum MemoryError {
    InvalidRequestResponse,
    OutOfMemory
}

pub fn init() -> Result<(), MemoryError> {
    let hhdm_offset = HHDM_REQUEST.response().ok_or(MemoryError::InvalidRequestResponse)?.offset;
    HHDM_OFFSET.init(hhdm_offset);

    physical::init()?;
    heap::init()?;
    virt::init();

    Ok(())
}

pub use wrappers::*;