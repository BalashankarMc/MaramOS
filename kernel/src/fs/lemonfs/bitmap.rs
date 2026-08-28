use alloc::{boxed::Box, vec::Vec};
use super::{Disk, FSResult, disk::BitmapBlock, io::write_data};

const BITS_PER_BLOCK: usize = 4096;

#[derive(Debug)]
pub enum BlockState {
    Free,
    Full,
    Mixed(Box<BitmapBlock>)
}

#[derive(Debug)]
pub struct BitmapAllocator {
    pub blocks: Vec<BlockState>,
    data_start: u64
}

impl BitmapAllocator {
    pub const fn new(data_start: u64) -> Self { Self { blocks: Vec::new(), data_start } }
    pub fn add_block(&mut self, block: Box<BitmapBlock>) {
        self.blocks.push(BlockState::Mixed(block));
        self.collapse(self.blocks.len() - 1);
    }

    const fn block_of(bit: u64) -> usize { bit as usize / BITS_PER_BLOCK }
    const fn bit_in(bit: u64) -> usize { bit as usize % BITS_PER_BLOCK }

    fn collapse(&mut self, block_idx: usize) {
        let new = match &self.blocks[block_idx] {
            BlockState::Mixed(block) => {
                if block.0.iter().all(|&x| x == 0) { Some(BlockState::Free) }
                else if block.0.iter().all(|&x| x == 0xFF) { Some(BlockState::Full) }
                else { None }
            }
            _ => None,
        };

        if let Some(block_state) = new { self.blocks[block_idx] = block_state }
    }

    fn is_free(&self, bit: u64) -> bool {
        match &self.blocks[Self::block_of(bit)] {
            BlockState::Free => true,
            BlockState::Full => false,
            BlockState::Mixed(b) => !b.get(Self::bit_in(bit)),
        }
    }

    fn materialize(&mut self, block: usize) {
        let fill = match &self.blocks[block] {
            BlockState::Free => 0,
            BlockState::Full => 0xFF,
            BlockState::Mixed(_) => unreachable!()
        };

        self.blocks[block] = BlockState::Mixed(Box::new(BitmapBlock([fill; 512])));
    }

    fn set_bit(&mut self, bit: u64) {
        let block = Self::block_of(bit);
        let index = Self::bit_in(bit);

        let is_full = matches!(self.blocks[block], BlockState::Full);
        if is_full { return } // Nothing to do

        let is_mixed = matches!(self.blocks[block], BlockState::Mixed(_));
        if !is_mixed { self.materialize(block) }

        if let BlockState::Mixed(block) = &mut self.blocks[block] { block.set(index) }
        self.collapse(block);
    }

    fn clear_bit(&mut self, bit: u64) {
        let block = Self::block_of(bit);
        let index = Self::bit_in(bit);

        let is_empty = matches!(self.blocks[block], BlockState::Free);
        if is_empty { return } // Nothing to do

        let is_mixed = matches!(self.blocks[block], BlockState::Mixed(_));
        if !is_mixed { self.materialize(block) }

        if let BlockState::Mixed(block) = &mut self.blocks[block] { block.clear(index) }
        self.collapse(block);
    }

    fn find_first_fit(&self, count: u64) -> Option<usize> {
        if count == 0 { return None }

        let block_count = self.blocks.len();
        let mut start = None;
        let mut run = 0;

        for block in 0..block_count {
            match &self.blocks[block] {
                BlockState::Free => {
                    if start.is_none() { start = Some(block * BITS_PER_BLOCK) }
                    run += BITS_PER_BLOCK;
                }

                BlockState::Full => { start = None; run = 0 }          

                BlockState::Mixed(b) => {
                    let base = block * BITS_PER_BLOCK;
                    for bit in 0..BITS_PER_BLOCK {
                        if b.get(bit) { run = 0; start = None; continue }
                    
                        if start.is_none() { start = Some(base + bit) }
                        run += 1;
                    }
                }
            }
            if run >= count as usize { return start }
        }
        None
    }

    pub fn alloc(&mut self, count: u64) -> Option<u64> {
        let start = self.find_first_fit(count)? as u64;
        for x in 0..count { self.set_bit(x + start) }
        Some(start + self.data_start)
    }

    pub fn free(&mut self, start: u64, count: u64) {
        let base = start - self.data_start;
        for x in 0..count { self.clear_bit(x + base); }
    }

    /// Mark all LBAs past `lba` (exclusive) as used
    pub fn reserve_from(&mut self, lba: u64) {
        let start_bit = match lba.checked_sub(self.data_start) {
            Some(x) => x as usize,
            None => return // Nothing to do
        };

        for bit in (start_bit + 1)..self.blocks.len() * BITS_PER_BLOCK { self.set_bit(bit as u64) }
    }

    pub fn sync(&self, disk: Disk, bitmap_start: u64) -> FSResult<()> {
        for (i, block) in self.blocks.iter().enumerate() {
            let block_raw = match block {
                BlockState::Free => BitmapBlock([0; 512]),
                BlockState::Full => BitmapBlock([0xFF; 512]),
                BlockState::Mixed(b) => **b
            };

            write_data(disk, i as u64 + bitmap_start, block_raw)?;
        }

        Ok(())
    }
}

impl BitmapBlock {
    /// Returns `true` if bit `bit` is set.
    pub const fn get(&self, bit: usize) -> bool { (self.0[bit / 8] >> (bit % 8)) & 1 == 1 }

    /// Sets bit `bit`.
    pub const fn set(&mut self, bit: usize) { self.0[bit / 8] |= 1 << (bit % 8) }

    /// Clears bit `bit` (0-based, within this block).
    pub const fn clear(&mut self, bit: usize) { self.0[bit / 8] &= !(1 << (bit % 8)) }
}