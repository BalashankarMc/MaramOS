//! Virtual memory manager

use x86_64::{PhysAddr, VirtAddr, structures::paging::{PageSize, PageTable, PageTableFlags, Size1GiB, Size2MiB, page_table::PageTableEntry}};

use crate::{allocator::RangeAllocator, helpers::InterruptMutex, memory::PAGE_SIZE};
use super::physical::{active_l4_table, get_pagetable};

static ALLOCATOR: InterruptMutex<RangeAllocator> = InterruptMutex::new(RangeAllocator::new());

/// A struct representing a contigous region of Virtual Memory
#[derive(Debug)]
pub struct VirtualRegion {
    start: VirtAddr,
    length: usize,
    is_mapped: bool
}

impl VirtualRegion {
    /// Allocate a new region of virtual memory
    /// of size `count` 4KiB Pages
    pub fn new(count: usize) -> Option<Self> {
        Some(Self {
            start: VirtAddr::new(ALLOCATOR.lock().allocate(count * PAGE_SIZE)? as u64),
            length: count,
            is_mapped: false
        })
    }

    pub fn map(&mut self, mut phys: PhysAddr, mut flags: PageTableFlags) -> Result<(), ()> {
        if self.is_mapped { return Err(()) }
        self.is_mapped = true;

        let mut rem = self.length;
        let mut virt = self.start;

        // Safety: Single non 'static use. Safe
        let l4 = unsafe { active_l4_table() };

        flags |= PageTableFlags::WRITABLE | PageTableFlags::PRESENT;

        if phys.is_aligned(Size1GiB::SIZE) { while rem >= Size1GiB::SIZE as usize / PAGE_SIZE {
            let l3 = ensure_table(&mut l4[virt.p4_index()]);
            l3[virt.p3_index()].set_addr(phys, flags | PageTableFlags::HUGE_PAGE);

            virt += Size1GiB::SIZE;
            phys += Size1GiB::SIZE;

            rem = rem.saturating_sub(Size1GiB::SIZE as usize / PAGE_SIZE);
        } }

        if phys.is_aligned(Size2MiB::SIZE) { while rem >= Size2MiB::SIZE as usize / PAGE_SIZE {
            let l3 = ensure_table(&mut l4[virt.p4_index()]);
            let l2 = ensure_table(&mut l3[virt.p3_index()]);
            l2[virt.p2_index()].set_addr(phys, flags | PageTableFlags::HUGE_PAGE);

            virt += Size2MiB::SIZE;
            phys += Size2MiB::SIZE;

            rem = rem.saturating_sub(Size2MiB::SIZE as usize / PAGE_SIZE);
        } }
        
        while rem > 0 {
            let l3 = ensure_table(&mut l4[virt.p4_index()]);
            let l2 = ensure_table(&mut l3[virt.p3_index()]);
            let l1 = ensure_table(&mut l2[virt.p2_index()]);

            l1[virt.p1_index()].set_addr(phys, flags);

            virt += PAGE_SIZE as u64;
            phys += PAGE_SIZE as u64;

            rem = rem.saturating_sub(1);
        }

        Ok(())
    }

    /// Returns the start of the region
    pub const fn address(&self) -> VirtAddr { self.start }
    
    /// Returns the size the region in bytes
    pub const fn length(&self) -> usize { self.length * PAGE_SIZE }
}

impl Drop for VirtualRegion {
    fn drop(&mut self) {
        ALLOCATOR.lock().free(self.start.as_u64() as usize, self.length);
    }
}

/// Initialize the vmem allocator
pub fn init() {
    // Safety: Single-shot non 'static use. Safe
    let l4 = unsafe { active_l4_table() };
    let mut alloc = ALLOCATOR.lock();

    for (l4_index, l4_entry) in l4.iter().enumerate().skip(256) {
        if l4_entry.is_unused() {
            // Unused - Push to allocator
            let start = (l4_index << 39) | (0xFFFF << 48);
            let end = start + (512 * 1024_usize.pow(3)); // Each L4 Entry owns 512GiB
            alloc.add_range(start, end);
            continue
        }

        let l3 = get_pagetable(l4_entry);
        for (l3_index, l3_entry) in l3.iter().enumerate() {
            if l3_entry.is_unused() {
                let start = (l4_index << 39) | (l3_index << 30) | (0xFFFF << 48);
                let end = start + 1024_usize.pow(3); // Each L3 Entry owns 1GiB

                alloc.add_range(start, end);
            }
        }
    }
}

fn ensure_table(entry: &mut PageTableEntry) -> &mut PageTable {
    if entry.is_unused() {
        let frame = unsafe { super::physical::alloc_pages_raw(1).unwrap() };
        unsafe { super::physical::zero_pages(super::phys_to_virt(frame), 1) };
        entry.set_addr(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
    }
    get_pagetable(entry)
}