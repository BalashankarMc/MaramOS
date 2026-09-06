//! Kernel Memory Module

use x86_64::{PhysAddr, VirtAddr, registers::control::Cr3, structures::paging::{PageSize, PhysFrame, Size4KiB}};
use crate::{HHDM_RESPONSE, KResult, LateInit};

mod physical;
mod wrappers;
mod heap;
mod virt;
mod user;

pub const PAGE_SIZE: usize = Size4KiB::SIZE as usize;

pub static KERNEL_L4: LateInit<PhysFrame> = LateInit::new();
pub static HHDM_OFFSET: LateInit<u64> = LateInit::new();

pub use wrappers::{PhysPage, Stack, MMIORegister, MMIORegion};
pub use virt::{VMemAllocator, VirtualRegion, AddressSpace};
pub use user::{map_page, new_user_allocator};

pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr { VirtAddr::new(phys.as_u64() + *HHDM_OFFSET) }
unsafe fn virt_to_phys(virt: VirtAddr) -> PhysAddr { PhysAddr::new(virt.as_u64() - *HHDM_OFFSET) }


pub fn init() -> KResult<()> {
    HHDM_OFFSET.init(HHDM_RESPONSE.offset);

    physical::init();
    heap::init()?;
    virt::init();

    KERNEL_L4.init(Cr3::read().0);

    Ok(())
}