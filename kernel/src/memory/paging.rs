//! Physical page frame allocator.
//!
//! Provides `alloc_frames` / `alloc_page_range` and `free_page_range`.
//! Power-of-two page counts go through the buddy allocator.
//! Non-power-of-two counts are served via the slab sub-allocator
//! to avoid rounding waste.
//!
//! Initialisation (`init`) consumes the Limine memory map's `MEMMAP_USABLE`
//! regions and populates both allocators.

use super::phys_to_virt;

use crate::allocator::BuddyAllocator;
use crate::helpers::InterruptMutex as Mutex;

use limine::{memmap::MEMMAP_USABLE, request::MemmapRespData};
use x86_64::{PhysAddr, VirtAddr, structures::paging::{PageSize, Size4KiB}};

const ALLOC_MIN_ORDER: usize = 12;
const ALLOC_ORDER_COUNT: usize = 19;

static ALLOCATOR: Mutex<BuddyAllocator
    <ALLOC_MIN_ORDER, ALLOC_ORDER_COUNT, 64, true>> = Mutex::new(BuddyAllocator::new());

fn block_size(order: usize) -> usize {
    BuddyAllocator::<ALLOC_MIN_ORDER, ALLOC_ORDER_COUNT, 64, true>::block_size(order)
}

/// Zero N consecutive 4 KiB pages starting at `virt`.
///
/// # Safety
///
/// - `virt` must point to `count` consecutive writable 4 KiB pages.
unsafe fn zero_pages(virt: VirtAddr, count: usize) {
    let ptr = virt.as_mut_ptr::<u8>();
    let qwords = count * (Size4KiB::SIZE as usize / 8);
    unsafe { core::arch::asm!(
        "rep stosq",
        inout("rcx") qwords => _,
        inout("rdi") ptr => _,
        in("rax") 0u64,
        options(nostack, preserves_flags),
    ) };
}

pub fn alloc_page_range(count: usize) -> Option<PhysAddr> {
    let mut alloc = ALLOCATOR.lock();
    let block = alloc.alloc_range(count)?;
    let virt = phys_to_virt(PhysAddr::new(block));
    unsafe { zero_pages(virt, count) }
    let phys = PhysAddr::new(block);
    drop(alloc);
    let order = count.next_power_of_two().trailing_zeros() as usize;
    try_upgrade_hhdm(phys, order);
    Some(phys)
}

pub fn free_page_range(addr: PhysAddr, count: usize) {
    if count == 0 { return }
    ALLOCATOR.lock().free_range(addr.as_u64(), count);
}

pub fn init(memory_map: &MemmapRespData, phys_offset: u64) -> usize {
    let mut alloc = ALLOCATOR.lock();
    alloc.set_offset(phys_offset);

    let mut total_frames = 0usize;

    for entry in memory_map.entries().iter() {
        if entry.type_ != MEMMAP_USABLE {
            continue;
        }

        let mut addr = entry.base as usize;
        let end = (entry.base + entry.length) as usize;
        addr = (addr + Size4KiB::SIZE as usize - 1) & !(Size4KiB::SIZE as usize - 1);

        while addr + Size4KiB::SIZE as usize <= end {
            let remaining = end - addr;
            let mut order = ALLOC_ORDER_COUNT - 1;
            loop {
                let size = block_size(order);
                if order == 0 || (remaining >= size && addr.is_multiple_of(size)) { break }
                order -= 1;
            }

            let size = block_size(order);
            unsafe { alloc.push(addr as u64, order); }
            total_frames += 1 << order;
            addr += size;
        }
    }

    total_frames * (Size4KiB::SIZE as usize)
}

pub fn alloc_frames(order: usize) -> Option<PhysAddr> {
    let mut alloc = ALLOCATOR.lock();
    let block = alloc.alloc(order)?;
    let pages = 1 << order;
    let virt = phys_to_virt(PhysAddr::new(block));
    unsafe { zero_pages(virt, pages) }
    let phys = PhysAddr::new(block);
    drop(alloc);
    try_upgrade_hhdm(phys, order);
    Some(phys)
}

pub fn free_frames(addr: PhysAddr, order: usize) {
    ALLOCATOR.lock().free(addr.as_u64(), order);
}

pub fn try_upgrade_hhdm(phys: PhysAddr, order: usize) {
    super::page_table::try_upgrade_hhdm(phys, order)
}
