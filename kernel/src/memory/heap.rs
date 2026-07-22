//! A global heap allocator for kernel memory.
//!
//! Uses the buddy-based [`BuddyAllocator`](crate::allocator::BuddyAllocator)
//! as the backing allocator, wrapped in an InterruptMutex for interrupt-safety.
//!
//! # Initialisation
//!
//! [`init`] pre-populates the buddy allocator with 1 MiB of page-backed
//! memory. Each 4 KiB page is individually allocated from the physical
//! page allocator and mapped into the heap's virtual address region.

use super::{paging, page_table};

use crate::allocator::BuddyAllocator;

use crate::library::InterruptMutex;

use core::alloc::{GlobalAlloc, Layout};
use x86_64::{PhysAddr, VirtAddr, structures::paging::{Page, PageSize, PageTableFlags, PhysFrame, Size4KiB}};

const HEAP_MIN_ORDER: usize = 4;
const HEAP_ORDER_COUNT: usize = 17;
const MIN_BLOCK: usize = 1 << HEAP_MIN_ORDER;

pub const HEAP_START: usize = 0xFFFF_C000_0000_0000;

struct BuddyHeap {
    alloc: BuddyAllocator<HEAP_MIN_ORDER, HEAP_ORDER_COUNT, 0>,
    size: usize,
    initialized: bool,
}

impl BuddyHeap {
    const fn new(size: usize) -> Self {
        Self {
            alloc: BuddyAllocator::new(),
            size,
            initialized: false,
        }
    }

    fn block_size(order: usize) -> usize { BuddyAllocator::<HEAP_MIN_ORDER, HEAP_ORDER_COUNT, 0>::block_size(order) }

    fn order_for(layout: Layout) -> usize {
        let size = layout.size().max(layout.align()).max(MIN_BLOCK);
        let mut order = 0;
        while Self::block_size(order) < size { order += 1 }
        order
    }

    fn alloc(&mut self, layout: Layout) -> *mut u8 {
        if !self.initialized { return core::ptr::null_mut() }
        self.alloc.alloc(Self::order_for(layout))
            .map(|a| a as *mut u8)
            .unwrap_or(core::ptr::null_mut())
    }

    fn free(&mut self, ptr: *mut u8, layout: Layout) {
        if !self.initialized || ptr.is_null() { return }
        let order = Self::order_for(layout);
        let heap_end = HEAP_START + self.size;
        self.alloc.free_with(ptr as u64, order, |buddy, _| {
            buddy >= HEAP_START as u64 && buddy < heap_end as u64
        });
    }
}

struct LockedBuddyHeap(InterruptMutex<Option<BuddyHeap>>);

unsafe impl GlobalAlloc for LockedBuddyHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 { self.0.lock().as_mut().expect("Failed to get mutable lock on allocator").alloc(layout) }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { self.0.lock().as_mut().expect("Failed to get mutable lock on allocator").free(ptr, layout) }
}

#[global_allocator]
static ALLOCATOR: LockedBuddyHeap = LockedBuddyHeap(InterruptMutex::new(None));

pub fn init(size: usize) {
    let mut lock = ALLOCATOR.0.lock();
    *lock = Some(BuddyHeap::new(size));
    let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    let pages = size / (Size4KiB::SIZE as usize);

    let phys_block = paging::alloc_page_range(pages);
    for i in 0..pages {
        let virt = VirtAddr::new(HEAP_START as u64 + (i as u64) * Size4KiB::SIZE);
        let page = Page::<Size4KiB>::containing_address(virt);
        let phys = PhysAddr::new(phys_block.as_u64() + (i as u64) * Size4KiB::SIZE);
        let frame = PhysFrame::containing_address(phys);
        unsafe { page_table::map_sized_page(page_table::PageMapping::Size4K(page, frame), flags) }
    }

    let alloc = lock.as_mut().unwrap();
    alloc.initialized = true;

    let mut addr = HEAP_START;
    let mut remaining = size;

    while remaining >= MIN_BLOCK {
        let mut order = HEAP_ORDER_COUNT - 1;
        loop {
            let size = BuddyHeap::block_size(order);
            let aligned = (addr - HEAP_START).is_multiple_of(size);
            if order == 0 || (remaining >= size && aligned) { break }
            order -= 1;
        }
        let size = BuddyHeap::block_size(order);
        unsafe { alloc.alloc.push(addr as u64, order) };
        addr += size;
        remaining -= size;
    }
}
