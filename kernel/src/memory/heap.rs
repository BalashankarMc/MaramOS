//! The kernel heap allocator

use core::alloc::GlobalAlloc;

use crate::{KernelResult, allocators::BuddyAllocator, helpers::{InterruptMutex, LateInit}, memory::{PAGE_SIZE, PhysPage}};

type Buddy = BuddyAllocator<4, 17, 64, true>;

static ALLOCATOR: InterruptMutex<Buddy> = InterruptMutex::new(BuddyAllocator::new());
static HEAP_PAGES: LateInit<PhysPage> = LateInit::new();

const HEAP_SIZE: usize = 1024 * 1024;

pub fn init() -> KernelResult<()> {

    let pages = PhysPage::new(HEAP_SIZE.div_ceil(PAGE_SIZE))?;

    HEAP_PAGES.init(pages);

    let addr = HEAP_PAGES.address().as_u64();
    let order = (HEAP_PAGES.count * PAGE_SIZE).next_power_of_two().trailing_zeros() as usize - 4;

    // Safety: We just allocated a block from the physical allocator. This is safe.
    unsafe { ALLOCATOR.lock().push(addr, order) };

    Ok(())
}

fn layout_to_size(layout: core::alloc::Layout) -> usize {
    let align_units = layout.align().div_ceil(16);
    let units = layout.size().max(16).div_ceil(16);
    if align_units == 1 { return units; }
    units.max(align_units).next_power_of_two()
}

struct KernelHeap;

#[global_allocator]
static HEAP_ALLOCATOR: KernelHeap = KernelHeap;

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        ALLOCATOR.lock().alloc_range(layout_to_size(layout)).unwrap_or(0) as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        ALLOCATOR.lock().free_range(ptr as u64, layout_to_size(layout));
    }
}