//! RAII wrappers for physical pages, DMA buffers, and virtual memory areas.
//!
//! [`PhysPage`] represents a physically-contiguous set of pages with
//! automatic deallocation on drop. [`DMABuffer`] provides DMA-safe
//! allocations. [`VirtualMemoryArea`] describes demand-paged user
//! mappings with backing store information.

use super::{paging::{alloc_page_range, free_page_range}, phys_to_virt};

use x86_64::{PhysAddr, VirtAddr, structures::paging::{PageSize, PageTableFlags, Size4KiB}};

/// A Struct representing a physically contiguous set of memory pages.
/// Implements Drop.
#[derive(Debug)]
pub struct PhysPage {
    start: PhysAddr,
    pub page_count: usize,
}

impl Drop for PhysPage {
    fn drop(&mut self) {
        if self.page_count == 0 { return }
        free_page_range(self.start, self.page_count);
    }
}

impl PhysPage {
    /// DO NOT USE MANUALLY
    pub(crate) fn new(start: PhysAddr, pages: usize) -> Self {
        Self {
            start,
            page_count: pages,
        }
    }

    /// Obtain the Physical Address of the start of the page segment
    pub fn get_phys_address(&self) -> PhysAddr { self.start }

    /// Obtain the Virtual Address of the start of the page segment
    pub fn get_virt_addr(&self) -> VirtAddr { phys_to_virt(self.start) }

    /// Write data <T> to the given offset from the start of the page segment
    ///
    /// # Safety
    /// Ensure the offset is not outside the range of the segment
    pub fn write_data<T>(&mut self, offset: usize, data: T) {
        let base = self.get_virt_addr().as_mut_ptr::<u8>();

        unsafe {
            let address = base.add(offset) as *mut T;
            address.write_volatile(data);
        }
    }

    /// Read data <T> from the given offset from the start of the page segment
    /// Returns a stack-copy of the data
    ///
    /// # Safety
    /// Ensure the offset is not outside the range of the segment
    pub fn read_data<T: Copy>(&self, offset: usize) -> T {
        let base = self.get_virt_addr().as_mut_ptr::<u8>();

        unsafe {
            let address = base.add(offset) as *const T;
            address.read_volatile()
        }
    }

    /// Returns the number of pages in the segment
    pub fn size(&self) -> usize { self.page_count }

    /// Leak the page segment to prevent it from being dropped
    pub fn leak(self) -> (PhysAddr, usize) {
        let data = (self.start, self.page_count);
        core::mem::forget(self);
        data
    }

    /// Create a dummy PhysPage with zero address (for test scaffolding).
    /// # Safety
    /// The resulting PhysPage has no valid backing; it must not be read/written.
    #[cfg(feature = "integration-test")]
    pub fn dummy() -> Self {
        Self { start: PhysAddr::new(0), page_count: 0 }
    }
}

pub enum VMABacking {
    Anonymous,
    File {
        cache: PhysPage,
        data_offset: u64,
        file_size: u64,
    },
}

pub struct VirtualMemoryArea {
    pub start: u64,
    pub end: u64,
    pub perms: PageTableFlags,
    pub backing: VMABacking,
}

/// A physically‑contiguous DMA buffer backed by exactly the requested
/// number of pages (no power‑of‑two rounding waste).
#[derive(Debug)]
pub struct DMABuffer {
    phys: PhysAddr,
    pages: usize,
}

#[allow(dead_code)]
impl DMABuffer {
    pub fn new(size_bytes: usize) -> Self {
        let pages = size_bytes.div_ceil(Size4KiB::SIZE as usize);
        let phys = alloc_page_range(pages);

        Self { phys, pages }
    }

    pub fn phys(&self) -> PhysAddr { self.phys }

    pub fn virt(&self) -> VirtAddr { phys_to_virt(self.phys) }

    pub fn size(&self) -> usize { self.pages * (Size4KiB::SIZE as usize) }
}

impl Drop for DMABuffer {
    fn drop(&mut self) { free_page_range(self.phys, self.pages); }
}
