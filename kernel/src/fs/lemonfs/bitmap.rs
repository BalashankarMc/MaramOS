//! LemonFS bitmap allocator.
//!
//! Manages on-disk block allocation via a fixed-size bitmap. Provides
//! first-fit allocation, range free, and contiguous-free counting for
//! the filesystem's data region.

use super::super::{FSError, FSReturn};
use super::{
    headers::{SuperBlock, Bitmap},
    io
};

use crate::drivers::storage::StorageDrive;

const BITMAP_BITS: usize = 4096;

pub fn free_range(drive: &mut dyn StorageDrive, start_lba: u64, count: u64) -> FSReturn<()> {
    if count == 0 { return Ok(()) }

    let sb: SuperBlock = io::read_metadata(drive, 0)?;
    let bitmap_start = sb.bitmap_start();
    let start_bit = (start_lba - sb.data_start()) as usize;
    let end_bit = start_bit + count as usize;

    for block_idx in (start_bit / BITMAP_BITS) ..= (end_bit - 1) / BITMAP_BITS {
        let mut bitmap: Bitmap = io::read_metadata(drive, bitmap_start + block_idx as u64)?;
        let local_start = start_bit.saturating_sub(block_idx * BITMAP_BITS);
        let local_end = BITMAP_BITS.min(end_bit - block_idx * BITMAP_BITS);

        for bit in local_start..local_end { bitmap.set(bit, false)? }

        io::write_metadata(drive, bitmap_start + block_idx as u64, bitmap)?
    }
    Ok(())
}

pub fn alloc_range(drive: &mut dyn StorageDrive, start_lba: u64, count: u64) -> FSReturn<u64> {
    let sb: SuperBlock = io::read_metadata(drive, 0)?;
    let bitmap_start = sb.bitmap_start();
    let data_start = sb.data_start();

    let start_bit = (start_lba - data_start) as usize;
    let end_bit = start_bit + count as usize;

    for block_idx in (start_bit / BITMAP_BITS) ..= (end_bit - 1) / BITMAP_BITS {
        let mut bitmap: Bitmap = io::read_metadata(drive, bitmap_start + block_idx as u64)?;

        let local_start = start_bit.saturating_sub(block_idx * BITMAP_BITS);
        let local_end = BITMAP_BITS.min(end_bit - block_idx * BITMAP_BITS);

        for b in local_start..local_end { bitmap.set(b, true)? }
        io::write_metadata(drive, bitmap_start + block_idx as u64, bitmap)?;
    }
    Ok(sb.data_start() + start_bit as u64)
}

pub fn bitmap_alloc_first_fit(drive: &mut dyn StorageDrive, count: u64) -> FSReturn<u64> {
    let sb: SuperBlock = io::read_metadata(drive, 0)?;

    if count == 0 { return Ok(sb.data_start()) }

    let count = count as usize;
    let total_data = (sb.total_blocks - sb.data_start()) as usize;
    let mut consecutive = 0;
    let mut run_start = 0;
    for block_idx in 0..sb.bitmap_size {
        let bitmap: Bitmap = io::read_metadata(drive, sb.bitmap_start() + block_idx)?;
        let bits = core::cmp::min(4096, total_data.saturating_sub(block_idx as usize * 4096));
        for bit in 0..bits {
            if !bitmap.check(bit)? {
                if consecutive == 0 { run_start = block_idx as usize * 4096 + bit }
                consecutive += 1;
                if consecutive == count { return alloc_range(drive, sb.data_start() + run_start as u64, count as u64) }
            } else { consecutive = 0 }
        }
    }
    Err(FSError::DiskFull)
}

pub fn free_contiguous(drive: &mut dyn StorageDrive, start_lba: u64, max_count: u64) -> FSReturn<u64> {
    let sb: SuperBlock = io::read_metadata(drive, 0)?;

    let start_bit = (start_lba - sb.data_start()) as usize;
    let total_data = (sb.total_blocks - sb.data_start()) as usize;
    let limit = core::cmp::min(max_count as usize, total_data.saturating_sub(start_bit));
    let end_bit = start_bit + limit;
    let mut bit = start_bit;
    while bit < end_bit {
        let block_idx = bit / 4096;
        let local_start = bit % 4096;
        let bitmap: Bitmap = io::read_metadata(drive, sb.bitmap_start() + block_idx as u64)?;
        let local_end = core::cmp::min(4096, end_bit - block_idx * 4096);
        for local in local_start..local_end {
            if bitmap.check(local)? { return Ok((bit - start_bit) as u64) }
            bit += 1;
        }
    }
    Ok(limit as u64)
}
