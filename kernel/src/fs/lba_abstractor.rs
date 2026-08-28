use alloc::sync::Arc;

use crate::{KernelResult, drivers::storage::{Drive, Partition, StorageError}, errors::MemoryError, memory::{PAGE_SIZE, PhysPage}};

#[derive(Debug)]
pub struct PartitionDrive {
    drive: Arc<Drive>,
    partition: Partition
}

impl PartitionDrive {
    pub const fn new(drive: Arc<Drive>, partition: Partition) -> Self { Self { drive, partition } }

    fn sectors_per_block(&self) -> u64 { self.drive.block_size() / 512 }
    pub fn sector_count(&self) -> u64 { self.partition.size_blocks * self.sectors_per_block() }

    pub fn read_sectors(&self, start: u64, count: u64) -> KernelResult<PhysPage> {
        if count == 0 { Err(StorageError::CommandFailed)? }

        let block_size = self.drive.block_size();

        let native_start = (start * 512) / block_size;
        let native_end = ((start + count) * 512).div_ceil(block_size);
        let native_count = native_end - native_start;

        let native_pages = self.drive.read(&self.partition, native_start, native_count)?;
        let dest = PhysPage::new((count as usize * 512).div_ceil(PAGE_SIZE)).ok_or(MemoryError::OutOfMemory)?;

        for i in 0..count {
            let off = ((start + i) * 512 - native_start * block_size) as usize;
            let buf = native_pages.read_data::<[u8; 512]>(off).ok_or(StorageError::CommandFailed)?;
            dest.write_data((i * 512) as usize, buf);
        }
        
        Ok(dest)
    }

    pub fn write_sectors(&self, start: u64, count: u64, src: &PhysPage) -> KernelResult<()> {
        if count == 0 { return Ok(()) }

        let block_size = self.drive.block_size();
        let sectors_per_block = block_size / 512;

        let native_start = (start * 512) / block_size;
        let native_end = ((start + count) * 512).div_ceil(block_size);
        let native_count = native_end - native_start;

        // Caller must have supplied at least count * 512 bytes
        if src.count * PAGE_SIZE < count as usize * 512 { Err(StorageError::CommandFailed)? }

        let staging = PhysPage::new(((native_count * block_size) as usize).div_ceil(PAGE_SIZE)).ok_or(MemoryError::OutOfMemory)?;

        for i in 0..native_count {
            let native_block = native_start + i;
            let first = native_block * sectors_per_block;
            let last = first + sectors_per_block - 1;
            let covered_start = first.max(start);
            let covered_end = last.min(start + count - 1);

            if covered_start != first || covered_end != last {
                let tmp = self.drive.read(&self.partition, native_block, 1)?;
                for j in first..=last {
                    let read_offset = (j * 512 - native_block * block_size) as usize;
                    let staging_offset = (j * 512 - native_start * block_size) as usize;
                    let data = tmp.read_data::<[u8; 512]>(read_offset).ok_or(StorageError::CommandFailed)?;
                    staging.write_data(staging_offset, data);
                }
            }

            for j in covered_start..=covered_end {
                let caller_offset = ((j - start) * 512) as usize;
                let staging_offset = (j * 512 - native_start * block_size) as usize;
                let data = src.read_data::<[u8; 512]>(caller_offset).ok_or(StorageError::CommandFailed)?;
                staging.write_data(staging_offset, data);
            }
        }

        self.drive.write(&self.partition, native_start, native_count, &staging)
    }

    pub fn zero_sectors(&self, start: u64, count: u64) -> KernelResult<()> {
        const MAX_SECTORS: u64 = 0x1000;

        if count == 0 { return Ok(()) }

        let page = PhysPage::new(count.min(MAX_SECTORS).div_ceil(8) as usize).ok_or(MemoryError::OutOfMemory)?;
        let mut sector = start;
        let mut rem = count;

        while rem > 0 {
            let chunk = rem.min(MAX_SECTORS);
            self.write_sectors(sector, chunk, &page)?;
            rem -= chunk;
            sector += chunk;
        }

        Ok(())
    }
}