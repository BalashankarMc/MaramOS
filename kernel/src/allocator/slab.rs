//! Page slab sub-allocator for non-power-of-two allocations.
//!
//! Carves exact-size contiguous ranges from buddy-provided blocks. Each
//! block maintains a linked free list of [`FreeRange`] entries; splits
//! occur when a range is larger than requested.

use x86_64::VirtAddr;

#[repr(C)]
struct FreeRange {
    next: u64,
    count: u32,
}

#[derive(Clone, Copy)]
struct BlockInfo {
    phys_base: u64,
    total_blocks: u32,
    allocated_blocks: u32,
    free_list: u64,
    used: bool,
}

pub(crate) struct PageSlab<const MAX_BLOCKS: usize> {
    blocks: [BlockInfo; MAX_BLOCKS],
    block_count: usize,
    offset: u64,
    block_size: usize,
}

impl<const MAX_BLOCKS: usize> PageSlab<MAX_BLOCKS> {
    pub const fn new(block_size: usize) -> Self {
        Self {
            block_size,
            .. unsafe { core::mem::zeroed() }
        }
    }

    pub fn set_offset(&mut self, offset: u64) { self.offset = offset }

    pub fn allocate(&mut self, count: usize) -> Option<u64> {
        for i in 0..self.block_count {
            if !self.blocks[i].used { continue }
            if let Some(addr) = self.alloc_in_block(i, count) { return Some(addr) }
        }
        None
    }

    pub fn free(&mut self, addr: u64, count: usize) {
        let Some(idx) = self.find_block(addr) else { return };
        let free_list = self.blocks[idx].free_list;
        let offset = self.offset;
        unsafe {
            let fr = &mut *(VirtAddr::new(addr + offset).as_mut_ptr::<FreeRange>());
            fr.next = free_list;
            fr.count = count as u32;
        }
        self.blocks[idx].free_list = addr;
        self.blocks[idx].allocated_blocks = self.blocks[idx].allocated_blocks.saturating_sub(count as u32);
    }

    pub fn add_block(&mut self, phys_base: u64, total_blocks: u32) {
        if self.block_count >= MAX_BLOCKS { return }
        let idx = self.block_count;
        self.block_count += 1;
        self.blocks[idx] = BlockInfo {
            phys_base,
            total_blocks,
            allocated_blocks: 0,
            free_list: 0,
            used: true,
        };
        let offset = self.offset;
        unsafe {
            let fr = &mut *(VirtAddr::new(phys_base + offset).as_mut_ptr::<FreeRange>());
            fr.next = 0;
            fr.count = total_blocks;
        }
        self.blocks[idx].free_list = phys_base;
    }

    pub fn owns(&self, addr: u64) -> bool { self.find_block(addr).is_some() }

    fn find_block(&self, addr: u64) -> Option<usize> {
        for i in 0..self.block_count {
            let b = &self.blocks[i];
            if !b.used { continue }
            let end = b.phys_base + (b.total_blocks as u64) * self.block_size as u64;
            if addr >= b.phys_base && addr < end { return Some(i) }
        }
        None
    }

    fn alloc_in_block(&mut self, idx: usize, count: usize) -> Option<u64> {
        let offset = self.offset;
        let block_size = self.block_size;
        let block_ptr: *mut BlockInfo = &mut self.blocks[idx];

        let mut prev_field: *mut u64 = unsafe { &mut (*block_ptr).free_list };
        let mut current = unsafe { (*block_ptr).free_list };

        while current != 0 {
            let range = unsafe { &mut *(VirtAddr::new(current + offset).as_mut_ptr::<FreeRange>()) };
            let rcount = range.count;
            let rnext = range.next;

            if (rcount as usize) >= count {
                let result = current;
                if (rcount as usize) == count { unsafe { *prev_field = rnext } }
                else {
                    let split = current + (count as u64) * block_size as u64;
                    unsafe {
                        let new = &mut *(VirtAddr::new(split + offset).as_mut_ptr::<FreeRange>());
                        new.next = rnext;
                        new.count = rcount - count as u32;
                        *prev_field = split;
                    }
                }
                unsafe { (*block_ptr).allocated_blocks += count as u32; }
                return Some(result);
            }
            prev_field = &mut range.next;
            current = rnext;
        }
        None
    }
}
