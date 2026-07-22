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
use crate::library::InterruptMutex as Mutex;

use limine::{memmap::MEMMAP_USABLE, request::MemmapRespData};
use x86_64::{PhysAddr, VirtAddr, structures::paging::{PageSize, PageTable, PageTableFlags, Size1GiB, Size2MiB, Size4KiB}};

const ALLOC_MIN_ORDER: usize = 12;
const ALLOC_ORDER_COUNT: usize = 19;

static ALLOCATOR: Mutex<BuddyAllocator
    <ALLOC_MIN_ORDER, ALLOC_ORDER_COUNT, 64, true>> = Mutex::new(BuddyAllocator::new());

fn block_size(order: usize) -> usize {
    BuddyAllocator::<ALLOC_MIN_ORDER, ALLOC_ORDER_COUNT, 64, true>::block_size(order)
}

/// Zero N consecutive 4 KiB pages starting at `virt`.
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

pub fn alloc_page_range(count: usize) -> PhysAddr {
    let mut alloc = ALLOCATOR.lock();
    let block = alloc.alloc_range(count).expect("Out of Memory");
    let virt = phys_to_virt(PhysAddr::new(block));
    unsafe { zero_pages(virt, count) }
    let phys = PhysAddr::new(block);
    drop(alloc);
    let order = count.next_power_of_two().trailing_zeros() as usize;
    try_upgrade_hhdm(phys, order);
    phys
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

pub fn alloc_frames(order: usize) -> PhysAddr {
    let mut alloc = ALLOCATOR.lock();
    let block = alloc.alloc(order).expect("Out of Memory");
    let pages = 1 << order;
    let virt = phys_to_virt(PhysAddr::new(block));
    unsafe { zero_pages(virt, pages) }
    let phys = PhysAddr::new(block);
    drop(alloc);
    try_upgrade_hhdm(phys, order);
    phys
}

pub fn free_frames(addr: PhysAddr, order: usize) {
    ALLOCATOR.lock().free(addr.as_u64(), order);
}

pub fn try_upgrade_hhdm(phys: PhysAddr, order: usize) {
    let virt = phys_to_virt(phys).as_u64();
    let l4 = unsafe { super::page_table::active_l4_table() };

    let l4_i = (virt >> 39) as usize & 0x1FF;
    let l3_i = (virt >> 30) as usize & 0x1FF;
    let l2_i = (virt >> 21) as usize & 0x1FF;

    // 1 GiB upgrade
    if order >= 18 && phys.is_aligned(Size1GiB::SIZE) {
        let l3 = unsafe { &mut *phys_to_virt(l4[l4_i].addr()).as_mut_ptr::<PageTable>() };
        let l3e = &mut l3[l3_i];
        if l3e.flags().contains(PageTableFlags::HUGE_PAGE) { return } // already upgraded

        let l2_table_phys = l3e.addr();
        let l2 = unsafe { &*phys_to_virt(l2_table_phys).as_ptr::<PageTable>() };
        // Free all L1 tables and their frames under this L2 table
        for l2_entry in l2.iter() {
            if l2_entry.is_unused() { continue }
            if l2_entry.flags().contains(PageTableFlags::HUGE_PAGE) { free_frames(l2_entry.addr(), 9) }
            else { free_frames(l2_entry.addr(), 0) }
        }
        free_frames(l2_table_phys, 0); // free L2 table itself
        l3e.set_addr(phys, PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::GLOBAL | PageTableFlags::HUGE_PAGE);
        crate::ipi::tlb_shootdown(VirtAddr::new(virt))
    }
    
    if order >= 9 && phys.is_aligned(Size2MiB::SIZE) {
        let l3 = unsafe { &mut *phys_to_virt(l4[l4_i].addr()).as_mut_ptr::<PageTable>() };
        let l2 = unsafe { &mut *phys_to_virt(l3[l3_i].addr()).as_mut_ptr::<PageTable>() };
        let l2e = &mut l2[l2_i];
        if l2e.flags().contains(PageTableFlags::HUGE_PAGE) { return } // already upgraded

        let l1_table_phys = l2e.addr();
        l2e.set_addr(phys, PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::GLOBAL | PageTableFlags::HUGE_PAGE);
        free_frames(l1_table_phys, 0); // free the now-orphaned L1 table
        crate::ipi::tlb_shootdown(VirtAddr::new(virt))
    }
}
