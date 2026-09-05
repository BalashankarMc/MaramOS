//! Virtual memory manager

use alloc::vec::Vec;
use x86_64::{PhysAddr, VirtAddr, registers::control::Cr3, structures::paging::{FrameAllocator, FrameDeallocator, MappedPageTable, Mapper, Page, PageSize, PageTable, PageTableFlags, PhysFrame, Size1GiB, Size2MiB, Size4KiB, mapper::{CleanUp, PageTableFrameMapping, UnmapError}, page_table::PageTableEntry}};

use crate::{InterruptMutex, LateInit, allocators::RangeAllocator, errors::MemoryError};

use super::{HHDM_OFFSET, phys_to_virt, physical::{alloc_pages, free_frames}, virt_to_phys};

const SIZE_L1: u64 = 1 << 12; // 4 KiB
const SIZE_L2: u64 = 1 << 21; // 2 MiB
const SIZE_L3: u64 = 1 << 30; // 1 GiB
const SIZE_L4: u64 = 1 << 39; // 512 GiB

pub static KERNEL_ALLOCATOR: LateInit<InterruptMutex<VMemAllocator>> = LateInit::new();

pub struct PMMFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for PMMFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        unsafe { Some(PhysFrame::containing_address(virt_to_phys(alloc_pages(1)?))) }
    }
}

pub struct PMMFrameDeallocator;

// Safety: frames handed to us are empty page tables, returned once
impl FrameDeallocator<Size4KiB> for PMMFrameDeallocator {
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        free_frames(frame.start_address(), 1);
    }
}

pub struct PhysOffset(pub u64);

unsafe impl PageTableFrameMapping for PhysOffset {
    fn frame_to_pointer(&self, frame: PhysFrame) -> *mut PageTable {
        let virt = frame.start_address().as_u64() + self.0;
        virt as *mut PageTable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpace {
    /// The higher-half address space
    Kernel,
    /// The lower-half address space
    User
}

/// A representation of a range of virtual memory
#[derive(Debug)]
pub struct VirtualRegion {
    pub start: VirtAddr,
    pub size: u64
}

/// An allocator to hand out virtual memory from a L4 Page Table
pub struct VMemAllocator {
    page_table: &'static mut PageTable,
    allocator: RangeAllocator
}

impl VMemAllocator {
    /// Create a new `VMemAllocator` for the given L4 Table
    /// 
    /// # Safety
    /// The provided table must be L4
    pub unsafe fn new(page_table: &'static mut PageTable, space: AddressSpace) -> Self {

        // Safety: We already assume `page_table` is L4
        let free = unsafe { find_free_ranges(page_table, space) };
        let mut allocator = RangeAllocator::new();

        for range in free {
            let start = range.start.as_u64() as usize;
            let end = start + range.size as usize;
            allocator.add_range(start, end);
        }

        Self { page_table, allocator }
    }

    /// Create a new `VMemAllocator` from the current L4 Table
    pub fn from_current_l4(space: AddressSpace) -> Self {
        let frame = Cr3::read().0;
        let phys = frame.start_address();
        let virt = phys_to_virt(phys);

        // Safety: Deref of a safe pointer. Safe
        let page_table = unsafe { &mut *virt.as_mut_ptr::<PageTable>() };

        // Safety: We know that the page_table obtained is an L4 Table. Safe
        unsafe { Self::new(page_table, space) }
    }

    pub fn map(&mut self, region: &VirtualRegion, phys: PhysAddr, mut flags: PageTableFlags) -> Result<(), MemoryError> {
        flags |= PageTableFlags::PRESENT;

        // Safety: We assume that the Page Table is L4
        let mut mapper = unsafe { new_mapper(self.page_table) };
        let mut allocator = PMMFrameAllocator;

        let mut rem = region.size;
        let mut curr_virt = region.start;
        let mut curr_phys = phys;

        if rem >= Size1GiB::SIZE && curr_phys.is_aligned(Size1GiB::SIZE) && curr_virt.is_aligned(Size1GiB::SIZE) {
            let count = rem / Size1GiB::SIZE;
            for _ in 0..count {
                let page = Page::<Size1GiB>::containing_address(curr_virt);
                let frame = PhysFrame::<Size1GiB>::containing_address(curr_phys);

                // Safety: Just mapping to a page table
                unsafe { mapper.map_to(page, frame, flags, &mut allocator).map_err(|_| MemoryError::InvalidMapping)?.flush(); };

                curr_virt += Size1GiB::SIZE;
                curr_phys += Size1GiB::SIZE;
            }
            rem -= count * Size1GiB::SIZE;
        }
        if rem >= Size2MiB::SIZE && curr_phys.is_aligned(Size2MiB::SIZE) && curr_virt.is_aligned(Size2MiB::SIZE) {
            let count = rem / Size2MiB::SIZE;
            for _ in 0..count {
                let page = Page::<Size2MiB>::containing_address(curr_virt);
                let frame = PhysFrame::<Size2MiB>::containing_address(curr_phys);

                // Safety: Same as last
                unsafe { mapper.map_to(page, frame, flags, &mut allocator).map_err(|_| MemoryError::InvalidMapping)?.flush(); }

                curr_virt += Size2MiB::SIZE;
                curr_phys += Size2MiB::SIZE;
            }
            rem -= count * Size2MiB::SIZE;
        }

        let count = rem.div_ceil(Size4KiB::SIZE);
        for _ in 0..count {
            let page = Page::<Size4KiB>::containing_address(curr_virt);
            let frame = PhysFrame::<Size4KiB>::containing_address(curr_phys);

            // Safety: Same as last
            unsafe { mapper.map_to(page, frame, flags, &mut allocator).map_err(|_| MemoryError::InvalidMapping)?.flush() };

            curr_virt += Size4KiB::SIZE;
            curr_phys += Size4KiB::SIZE;
        }

        Ok(())
    }

    pub fn unmap(&mut self, region: &VirtualRegion) -> Result<(), MemoryError> {
        // Safety: We already assume that the `PageTable` is L4
        let mut mapper = unsafe { new_mapper(self.page_table) };
        let mut dealloc = PMMFrameDeallocator;
        let mut curr = region.start;
        let end = region.start + region.size;

        while curr < end {
            let rem = end - curr;
            if rem >= Size1GiB::SIZE && curr.is_aligned(Size1GiB::SIZE) {
                let page = Page::<Size1GiB>::containing_address(curr);
                if let Ok((_, f)) = mapper.unmap(page) {
                    f.flush();
                    curr += Size1GiB::SIZE;
                    continue
                }
            }

            if rem >= Size2MiB::SIZE && curr.is_aligned(Size2MiB::SIZE) {
                let page = Page::<Size2MiB>::containing_address(curr);
                if let Ok((_, f)) = mapper.unmap(page) {
                    f.flush();
                    curr += Size2MiB::SIZE;
                    continue
                }
            }

            let page = Page::<Size4KiB>::containing_address(curr);
            match mapper.unmap(page) {
                Ok((_, f)) => f.flush(),
                Err(UnmapError::PageNotMapped) => {},
                Err(_) => return Err(MemoryError::InvalidMapping)
            }

            curr += Size4KiB::SIZE;
        }

        let range = Page::<Size4KiB>::range_inclusive(
            Page::containing_address(region.start),
            Page::containing_address(region.start + region.size - 1)
        );

        // Safety: The region was already unmapped. Safe
        unsafe { mapper.clean_up_addr_range(range, &mut dealloc) }

        Ok(())
    }

    pub fn alloc(&mut self, count: usize) -> Option<VirtualRegion> {
        if count == 0 { return None; }

        let align = if count.is_multiple_of(0x40_000) { Size1GiB::SIZE as usize }
            else if count.is_multiple_of(0x200) { Size2MiB::SIZE as usize }
            else { Size4KiB::SIZE as usize};

        let size_bytes = count * Size4KiB::SIZE as usize;
        let start = self.allocator.allocate(size_bytes, align)?;

        Some(VirtualRegion { start: VirtAddr::new(start as u64), size: size_bytes as u64 })
    }

    #[allow(clippy::needless_pass_by_value)] // Consume VirtualRegion so it cannot be used
    pub fn free(&mut self, region: VirtualRegion) -> Result<(), MemoryError> {
        self.unmap(&region)?;
        self.allocator.free(region.start.as_u64() as usize, region.size as usize);
        Ok(())
    }
}

/// Find free Entries for an L4 Table
/// 
/// Safety
/// The provided Table must be L4
unsafe fn find_free_ranges(page_table: &PageTable, space: AddressSpace) -> Vec<VirtualRegion> {
    let mut res = Vec::new();

    let range = match space {
        AddressSpace::Kernel => 256..512,
        AddressSpace::User => 0..256
    };

    for l4_index in range {
        let l4_entry = &page_table[l4_index];
        if l4_entry.is_unused() { // Free L4 Entry
            let region = VirtualRegion { start: index_to_addr(l4_index, 0, 0, 0), size: SIZE_L4 };
            res.push(region);
            continue
        }

        // Not free, parse inner table
        // Safety: All used L4 entries point to an L3 Table. Safe
        let l3_table = unsafe { get_pagetable(l4_entry) };

        for (l3_index, l3_entry) in l3_table.iter().enumerate() {
            if l3_entry.is_unused() { // Free L3 Entry
                let region = VirtualRegion { start: index_to_addr(l4_index, l3_index, 0, 0), size: SIZE_L3 };
                res.push(region);
                continue
            }

            if l3_entry.flags().contains(PageTableFlags::HUGE_PAGE) { continue } // Used

            // Not free, parse inner table
            // Safety: All used, non-huge L3 entries point to an L2 Table. Safe
            let l2_table = unsafe { get_pagetable(l3_entry) };

            for (l2_index, l2_entry) in l2_table.iter().enumerate() {
                if l2_entry.is_unused() { // Free L2 Entry
                    let region = VirtualRegion { start: index_to_addr(l4_index, l3_index, l2_index, 0), size: SIZE_L2 };
                    res.push(region);
                    continue
                }

                if l2_entry.flags().contains(PageTableFlags::HUGE_PAGE) { continue } // Used

                // Not free, parse L1 Table
                // Safety: All used, non-huge L2 entries point to an L1 Table. Safe
                let l1_table = unsafe { get_pagetable(l2_entry) };

                for (l1_index, l1_entry) in l1_table.iter().enumerate() {
                    if l1_entry.is_unused() {
                        let region = VirtualRegion {
                            start: index_to_addr(l4_index, l3_index, l2_index, l1_index),
                            size: SIZE_L1
                        };

                        res.push(region);
                    }
                }
            }

        }
    }

    res
}

/// Gets the `PageTable` pointed to by `entry`.
/// 
/// Safety
/// `entry` must point to a valid `PageTable` and not a mapped page
unsafe fn get_pagetable(entry: &PageTableEntry) -> &PageTable {
    unsafe { &* phys_to_virt(entry.addr()).as_ptr() }
}

const fn index_to_addr(l4: usize, l3: usize, l2: usize, l1: usize) -> VirtAddr {
    let mut addr = (l4 << 39) | (l3 << 30) | (l2 << 21) | (l1 << 12);
    let sign_extension_needed = (addr >> 47) & 1 == 1;
    if sign_extension_needed { addr |= 0xFFFF_0000_0000_0000 }

    VirtAddr::new(addr as u64)
}

/// The provided `PageTable` must be L4
unsafe fn new_mapper(pt: &mut PageTable) -> MappedPageTable<'_, PhysOffset> {
    unsafe { MappedPageTable::new(pt, PhysOffset(*HHDM_OFFSET)) }
}

pub fn init() {
    let kernel_allocator = VMemAllocator::from_current_l4(AddressSpace::Kernel);
    KERNEL_ALLOCATOR.init(InterruptMutex::new(kernel_allocator));
}