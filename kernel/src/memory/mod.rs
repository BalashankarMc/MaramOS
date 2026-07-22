//! The Memory module. Manages the Heap, Page allocation, HHDM and provides DMA Buffers.
//!
//! Most Page manipulation is done through the PhysPage Struct (RAII), which represents a physically-contiguous
//! cluster of 4KiB Pages.
//! HHDM functionality is provided through the phys_to_virt() function at the module root.
//! All other memory is managed by the KMemory Struct.
//!
//! Page allocation uses a buddy allocator for power‑of‑two page counts
//! and a slab sub‑allocator for non‑power‑of‑two counts,
//! eliminating rounding waste.
//!
//! # Safety
//! This module can be used in unsafe ways and it is the user's responsibility to use it well.

use crate::boot::requests::{HHDM_OFFSET, MMAP};
use crate::library::LateInit;

use x86_64::{
    PhysAddr, VirtAddr,
    structures::{
        idt::PageFaultErrorCode,
        paging::{PageSize, PageTableFlags, PhysFrame, Size4KiB},
    },
};

mod heap;
mod page_table;
mod paging;
mod wrappers;

/// Number of bytes in a single page (4 KiB)
pub const PAGE_SIZE: usize = Size4KiB::SIZE as usize;

static PHYS_OFFSET: LateInit<u64> = LateInit::new();

/// Translate a Physical Address to a Virtual Address using the HHDM offset.
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr { VirtAddr::new(phys.as_u64() + *PHYS_OFFSET) }

/// Initialize the memory module
/// Takes a Limine MemmapRequest and the HHDM offset.
pub fn init() {
    PHYS_OFFSET.init(*HHDM_OFFSET);
    page_table::init();
    let _pages = paging::init(&MMAP, *HHDM_OFFSET);
    heap::init(1024 * 1024);
}

pub use wrappers::{DMABuffer, PhysPage, VMABacking, VirtualMemoryArea};

/// The main memory manager struct.
/// Provides page allocation (auto-deallocated via PhysPage's Drop)
/// and MMIO mapping/unmapping.
pub struct KMemory;

impl KMemory {
    /// Returns a single zeroed Page (4KiB)
    pub fn alloc_page() -> PhysPage {
        let start = paging::alloc_frames(0);
        PhysPage::new(start, 1)
    }

    /// Returns a zeroed PhysPage containing exactly `count` pages.
    /// Non‑power‑of‑two counts are served via the page‑slab sub‑allocator
    /// to avoid rounding waste.
    pub fn alloc_pages(count: usize) -> PhysPage {
        let start = paging::alloc_page_range(count);
        PhysPage::new(start, count)
    }

    /// Map a Physical Address for Memory-mapped I/O
    pub fn map_mmio(addr: PhysAddr, pages: usize) -> VirtAddr { unsafe { page_table::map_mmio(addr, pages) } }

    /// Unmap a MMIO Page from its Virtual Address
    pub fn unmap_mmio(addr: VirtAddr, pages: usize) { unsafe { page_table::unmap_mmio(addr, pages) } }

    /// Get a kernel Page Table
    pub fn kernel_l4() -> &'static PhysFrame { page_table::get_kernel_l4() }

    pub fn map_user_page(l4: PhysAddr, virt: VirtAddr, phys: &PhysPage, flags: PageTableFlags) {
        unsafe { page_table::map_user_page(l4, virt, phys, flags) }
    }

    /// Check if the given page is mapped or not
    pub fn is_user_page_mapped(addr: u64) -> bool { page_table::is_user_page_mapped(addr) }

    /// Create a new user Page Table
    pub fn new_user_page_table() -> PhysAddr { page_table::new_user_page_table() }

    /// Remove and deallocate a user Page Table
    pub fn unmap_user_page_table(pt: PhysAddr) { page_table::free_user_address_space(pt); }
}

pub fn resolve_user_demand_page(vmas: &[VirtualMemoryArea], page_table: PhysAddr, addr: u64, error_code: PageFaultErrorCode) -> bool {
    let page_addr = addr & !0xFFF;

    let vma = match vmas.iter().find(|v| addr >= v.start && addr < v.end) {
        Some(v) => v,
        None => return false,
    };

    if error_code.contains(PageFaultErrorCode::CAUSED_BY_WRITE) && !vma.perms.contains(PageTableFlags::WRITABLE) { return false }
    if error_code.contains(PageFaultErrorCode::INSTRUCTION_FETCH) && vma.perms.contains(PageTableFlags::NO_EXECUTE) { return false }

    let phys = paging::alloc_frames(0);
    let page_virt = phys_to_virt(phys);

    match &vma.backing {
        VMABacking::Anonymous => {}
        VMABacking::File {
            cache,
            data_offset,
            file_size,
        } => {
            let offset = page_addr - vma.start;
            if offset >= *data_offset && offset < *data_offset + file_size {
                let cache_virt = cache.get_virt_addr();
                let src = unsafe { cache_virt.as_ptr::<u8>().add(offset as usize) };
                let len = core::cmp::min(Size4KiB::SIZE, *data_offset + *file_size - offset) as usize;
                unsafe { core::ptr::copy_nonoverlapping(src, page_virt.as_mut_ptr::<u8>(), len) };
            }
        }
    }

    let flags = PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::PRESENT
        | (vma.perms & (PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE));

    let page = PhysPage::new(phys, 1);

    unsafe { page_table::map_user_page(
        page_table,
        VirtAddr::new(page_addr),
        &page,
        flags) }

    page.leak();

    true
}
