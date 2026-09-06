//! Memory management for user-space

use x86_64::{structures::paging::{PageTable, PageTableFlags}};

use crate::{KResult, errors::MemoryError};
use super::{AddressSpace, PhysPage, VMemAllocator, VirtualRegion, physical::active_l4_table};

pub fn new_user_allocator() -> KResult<(VMemAllocator, PhysPage)> {
    let page = PhysPage::new(1)?;
    let mut table = PageTable::new();

    // Safety: Not used mutably. Safe
    let curr_table = &*unsafe { active_l4_table() };
    for i in 256..512 { // Copy higher half
        let mut entry = curr_table[i].clone();
        let mut flags = entry.flags();
        flags.remove(PageTableFlags::USER_ACCESSIBLE);
        entry.set_flags(flags);

        table[i] = entry;
    }

    page.write_data(0, table)?;
    
    // Safety: Deref of a safe pointer. Safe
    let page_table = unsafe { &mut *page.address().as_mut_ptr::<PageTable>() };

    // Safety: We just created an L4 Table. Safe
    let allocator = unsafe { VMemAllocator::new(page_table, AddressSpace::User) };

    Ok((allocator, page))
}

pub fn map_page(addr_space: &mut VMemAllocator, page: &PhysPage, flags:PageTableFlags) -> KResult<VirtualRegion> {
    let phys = page.phys();
    let virt = addr_space.alloc(page.count).ok_or(MemoryError::OutOfMemory)?;
    addr_space.map(&virt, phys, flags | PageTableFlags::USER_ACCESSIBLE)?;

    Ok(virt)
}