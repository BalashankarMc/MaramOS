//! Physical Memory management

use x86_64::{PhysAddr, VirtAddr, registers::control::Cr3, structures::paging::{PageSize, PageTable, PageTableFlags, Size1GiB, Size2MiB, page_table::PageTableEntry}};

use crate::{MMAP_RESPONSE, allocators::BuddyAllocator, helpers::InterruptMutex, memory::{HHDM_OFFSET, PAGE_SIZE, phys_to_virt}};

type Buddy = BuddyAllocator<12, 19, 64, true>;

static ALLOCATOR: InterruptMutex<Buddy> = InterruptMutex::new(BuddyAllocator::new());

/// Initialize the physical page allocator
pub fn init() {
    let mut alloc = ALLOCATOR.lock();
    let mmap = MMAP_RESPONSE.entries();

    alloc.set_offset(*HHDM_OFFSET);

    for entry in mmap {
        if entry.type_ != limine::memmap::MEMMAP_USABLE { continue }

        let start = entry.base;
        let end = entry.length + start;

        let mut addr = start.next_multiple_of(PAGE_SIZE as u64);

        while addr + PAGE_SIZE as u64 <= end {
            let rem = end - addr;

            let mut order = Buddy::max_order();
            let mut block = Buddy::block_size(order) as u64;
            
            while order != 0 && !(rem >= block && addr.is_multiple_of(block)) {
                order -= 1;
                block = Buddy::block_size(order) as u64;
            }

            // Safety: An entry from the MMAP is safe to push to the allocator
            unsafe { alloc.push(addr, order) };

            addr += block;
        }
    }
}

/// Zero `count` pages starting at `start`
pub unsafe fn zero_pages(start: VirtAddr, count: usize) {
    let ptr = start.as_mut_ptr::<u64>();
    let word_count = count * PAGE_SIZE / 8;
    unsafe { core::arch::asm!(
        "rep stosq",
        in("rax") 0,
        inout("rcx") word_count => _,
        inout("rdi") ptr => _
    ); }
}

/// Allocate a run of `count` pages and return the start address (Physical)
/// or None if OOM
/// 
/// Safety: Does not zero the allocated memory or merge the pages
pub unsafe fn alloc_pages_raw(count: usize) -> Option<PhysAddr> {
    ALLOCATOR.lock().alloc_range(count).map(PhysAddr::new)
}

/// Allocate a run of `count` pages and return the start address (Virtual)
pub fn alloc_pages(count: usize) -> Option<VirtAddr> {
    // Safety: We properly initialize the memory. This is safe.
    let start = unsafe { alloc_pages_raw(count) }?;

    let virt = phys_to_virt(start);

    // Safety: We just allocated the memory. This is safe
    unsafe { zero_pages(virt, count) };
    try_upgrade_hhdm(start, count);

    Some(virt)
}

pub fn free_frames(start: PhysAddr, count: usize) {
    ALLOCATOR.lock().free_range(start.as_u64(), count);
}

/// # Safety
///  Safe only for non-const uses
pub unsafe fn active_l4_table<'a>() -> &'a mut PageTable {
    unsafe { &mut *(phys_to_virt(Cr3::read().0.start_address()).as_mut_ptr()) }
}

#[allow(clippy::mut_from_ref)]
pub fn get_pagetable(e: &PageTableEntry) -> &mut PageTable {
    unsafe { &mut *phys_to_virt(e.addr()).as_mut_ptr::<PageTable>() }
}

fn try_upgrade_hhdm(phys: PhysAddr, count: usize) {
    const PAGES_2MIB: usize = (Size2MiB::SIZE as usize) / PAGE_SIZE; // 512
    const PAGES_1GIB: usize = (Size1GiB::SIZE as usize) / PAGE_SIZE; // 262144

    // Safety: Not a 'static. Safe
    let l4 = unsafe { active_l4_table() };

    let virt = phys_to_virt(phys);

    let l4_index = virt.p4_index();
    let l3_index = virt.p3_index();
    let l2_index = virt.p2_index();

    if l4[l4_index].is_unused() || l4[l4_index].flags().contains(PageTableFlags::HUGE_PAGE) { return }

    let l3 = get_pagetable(&l4[l4_index]);
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::GLOBAL | PageTableFlags::HUGE_PAGE;

    if count == PAGES_1GIB && phys.is_aligned(Size1GiB::SIZE) {
        let l3_entry = &mut l3[l3_index];

        // Already a huge page, or not an L2 page-table frame.
        if l3_entry.is_unused() || l3_entry.flags().contains(PageTableFlags::HUGE_PAGE) { return }

        let l2 = get_pagetable(l3_entry);
        for l2_entry in l2.iter() {
            if l2_entry.is_unused() || l2_entry.flags().contains(PageTableFlags::HUGE_PAGE) { continue }
            free_frames(l2_entry.addr(), 1);
        }

        free_frames(l3_entry.addr(), 1);
        l3_entry.set_addr(phys, flags);
        flush_range(virt, PAGES_1GIB);

    } else if count == PAGES_2MIB && phys.is_aligned(Size2MiB::SIZE) {
        let l3_entry = &l3[l3_index];

        // Must be an L2 page table, not a huge page / unused.
        if l3_entry.is_unused() || l3_entry.flags().contains(PageTableFlags::HUGE_PAGE) { return }

        let l2 = get_pagetable(l3_entry);
        let l2_entry = &mut l2[l2_index];

        // Already a huge page — nothing to free.
        if l2_entry.is_unused() || l2_entry.flags().contains(PageTableFlags::HUGE_PAGE) { return }

        free_frames(l2_entry.addr(), 1);
        l2_entry.set_addr(phys, flags);
        flush_range(virt, PAGES_2MIB);
    }
}

fn flush_range(start: VirtAddr, pages: usize) {
    for i in 0..pages {
        x86_64::instructions::tlb::flush(start + (i as u64) * PAGE_SIZE as u64);
    }
}