//! Storage Driver for `MaramOS`.
//! It also provides abstractions for Storage I/O

use crate::{KernelError, errors::MemoryError, memory::PhysPage};
use thiserror::Error;

pub const BLOCK_SIZE: usize = 512;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Storage Device Init Failed!")]
    InitFailed,
    #[error("Storage: Invalid Block accessed!")]
    BlockOutOfBounds
}

pub trait StorageDrive {
    /// Return the number of 512 byte blocks
    fn block_count(&self) -> u64;

    /// Read `count` 512 byte blocks starting at `start` into `dest`
    fn read_blocks(&self, start_block: u64, count: u64, dest: &mut PhysPage) -> Result<(), StorageError>;

    /// Write `count` 512 byte blocks starting at `start` from `src`
    fn write_blocks(&self, start_block: u64, count: u64, src: &PhysPage) -> Result<(), StorageError>;

    /// Read `count` 512 byte blocks starting at `start`
    fn read_smart(&self, start_block: u64, count: u64) -> Result<PhysPage, KernelError> {
        let page_count = count.div_ceil(4) as usize;
        let mut pages = PhysPage::new(page_count)
            .ok_or(MemoryError::OutOfMemory)?;

        self.read_blocks(start_block, count, &mut pages)?;

        Ok(pages)
    }
}