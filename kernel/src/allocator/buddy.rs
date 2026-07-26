//! Generic buddy allocator.
//!
//! Manages power-of-two memory blocks using free lists per order. Supports
//! splitting larger blocks on allocation and merging buddies on free. An
//! optional slab sub-allocator handles non-power-of-two counts via
//! [`alloc_range`](BuddyAllocator::alloc_range).

use super::slab::PageSlab;

use x86_64::VirtAddr;

pub struct BuddyAllocator
    <const MIN_ORDER: usize, const ORDER_COUNT: usize, const SLAB_BLOCKS: usize, const HAS_SLAB: bool = false>
{
    pub(crate) heads: [u64; ORDER_COUNT],
    pub(crate) offset: u64,
    slab: PageSlab<SLAB_BLOCKS>,
}

impl<const MIN_ORDER: usize, const ORDER_COUNT: usize, const SLAB_BLOCKS: usize, const HAS_SLAB: bool>
    BuddyAllocator<MIN_ORDER, ORDER_COUNT, SLAB_BLOCKS, HAS_SLAB>
{
    pub const fn new() -> Self {
        Self {
            heads: [0u64; ORDER_COUNT],
            offset: 0,
            slab: PageSlab::new(1 << MIN_ORDER),
        }
    }

    pub fn set_offset(&mut self, offset: u64) {
        self.offset = offset;
        self.slab.set_offset(offset);
    }

    pub fn block_size(order: usize) -> usize { 1 << (order + MIN_ORDER) }

    fn max_order(&self) -> usize { ORDER_COUNT - 1 }

    fn ptr(&self, addr: u64) -> *mut u64 { VirtAddr::new(addr + self.offset).as_mut_ptr::<u8>() as *mut u64 }

    pub fn buddy_of(addr: u64, order: usize) -> u64 { addr ^ Self::block_size(order) as u64 }

    pub unsafe fn push(&mut self, addr: u64, order: usize) {
        unsafe { self.ptr(addr).write(self.heads[order]) }
        self.heads[order] = addr;
    }

    pub unsafe fn pop(&mut self, order: usize) -> Option<u64> {
        let head = self.heads[order];
        if head == 0 {
            return None;
        }
        let next = unsafe { self.ptr(head).read() };
        self.heads[order] = next;
        Some(head)
    }

    pub unsafe fn remove(&mut self, addr: u64, order: usize) -> bool {
        let mut prev_ptr: *mut u64 = &mut self.heads[order];
        let mut current = self.heads[order];

        while current != 0 {
            if current == addr {
                let next = unsafe { self.ptr(current).read() };
                unsafe { prev_ptr.write(next) }
                return true;
            }
            prev_ptr = self.ptr(current);
            current = unsafe { self.ptr(current).read() };
        }
        false
    }

    pub fn alloc(&mut self, order: usize) -> Option<u64> {
        if order > self.max_order() { return None }

        let mut found = order;
        loop {
            if found > self.max_order() { return None }
            if self.heads[found] != 0 { break }
            found += 1;
        }

        let block = unsafe { self.pop(found) }?;

        while found > order {
            found -= 1;
            let buddy = block + Self::block_size(found) as u64;
            unsafe { self.push(buddy, found) }
        }

        Some(block)
    }

    pub fn free(&mut self, addr: u64, order: usize) {
        self.free_with(addr, order, |_, _| true)
    }

    pub fn free_with(&mut self, addr: u64, order: usize, can_merge: impl Fn(u64, usize) -> bool) {
        if order > self.max_order() {
            return;
        }
        let mut addr = addr;
        let mut order = order;

        loop {
            if order == self.max_order() {
                break;
            }
            let buddy = Self::buddy_of(addr, order);
            if !can_merge(buddy, order) {
                break;
            }
            if !unsafe { self.remove(buddy, order) } {
                break;
            }
            addr = addr.min(buddy);
            order += 1;
        }

        unsafe { self.push(addr, order) }
    }

    /// Allocate exactly `count` contiguous blocks (each of size `1 << MIN_ORDER`).
    /// Power-of-two counts always go through the buddy allocator.
    /// Non-power-of-two counts optionally use the slab sub-allocator.
    pub fn alloc_range(&mut self, count: usize) -> Option<u64> {
        if HAS_SLAB && !count.is_power_of_two() {
            if let Some(addr) = self.slab.allocate(count) {
                return Some(addr);
            }

            let order = count.next_power_of_two().trailing_zeros() as usize;
            let block = self.alloc(order)?;
            let total = 1u32 << order;
            self.slab.add_block(block, total);
            return self.slab.allocate(count);
        }

        let order = count.next_power_of_two().trailing_zeros() as usize;
        self.alloc(order)
    }

    /// Free exactly `count` contiguous blocks at `addr`.
    /// Routes through the slab sub-allocator if the address is slab-managed.
    pub fn free_range(&mut self, addr: u64, count: usize) {
        if count == 0 {
            return;
        }
        if HAS_SLAB && self.slab.owns(addr) {
            self.slab.free(addr, count);
            return;
        }
        let order = count.next_power_of_two().trailing_zeros() as usize;
        self.free(addr, order);
    }
}
