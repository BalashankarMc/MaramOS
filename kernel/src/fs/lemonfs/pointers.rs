//! Helpers to traverse HashPointers and Links

use alloc::vec::Vec;

use crate::drivers::storage::{LBA_SIZE, StorageDrive};
use crate::fs::lemonfs::headers::{HASHES_PER_BLOCK, LINKS_PER_BLOCK, SuperBlock};
use crate::fs::lemonfs::io;
use crate::fs::{FSError, FSReturn};

use super::headers::{HashEntry, HashPointer, HashTableBlock, LinkEntry, LinkBlock};
use super::io::{read_metadata as ioread, write_metadata as iowrite};

impl HashPointer {
    pub fn read(&self, drive: &mut dyn StorageDrive) -> FSReturn<HashEntry> {
        let block: HashTableBlock = ioread(drive, self.lba)?;
        Ok(block.entries[self.offset as usize])
    }

    pub fn write(&mut self, drive: &mut dyn StorageDrive, entry: HashEntry) -> FSReturn<()> {
        let mut block: HashTableBlock = ioread(drive, self.lba)?;
        block.entries[self.offset as usize] = entry;
        iowrite(drive, self.lba, block)
    }

    pub fn add(&self, max_hash_lba: u64, n: u64) -> Option<Self> {
        let tmp = self.offset as u64 + n;
        let offset = (tmp % HASHES_PER_BLOCK as u64) as u8;
        let lba = self.lba + (tmp / HASHES_PER_BLOCK as u64);

        if lba > max_hash_lba { None }
        else { Some(Self { lba, offset }) }
    }

    pub fn is_valid(&self) -> bool { self.lba != 0 }
}

impl LinkEntry {
    pub fn is_valid(&self) -> bool { self.ptr != 0 }
    pub fn is_direct(&self) -> bool { self.size != 0 }
}

pub fn extend_link(drive: &mut dyn StorageDrive, link: &mut LinkEntry, append: LinkEntry) -> FSReturn<()> {
    if !link.is_valid() { *link = append; return Ok(()) } // Chain is empty

    let mut lba = link.ptr;
    loop {
        let mut block: LinkBlock = ioread(drive, lba)?;

        for i in 0..LINKS_PER_BLOCK {
            if !block.entries[i].is_valid() {
                block.entries[i] = append;
                return iowrite(drive, lba, block);
            }
        }

        // Block is full
        let last = block.entries[LINKS_PER_BLOCK - 1];
        if last.is_direct() {
            // Displace last entry to new block
            let new_block_lba = super::bitmap::bitmap_alloc_first_fit(drive, 1)?;
            let mut new_block = LinkBlock::new();
            new_block.entries[0] = last;
            new_block.entries[1] = append;
            iowrite(drive, new_block_lba, new_block)?;

            block.entries[LINKS_PER_BLOCK - 1] = LinkEntry { ptr: new_block_lba, size: 0 };
            return iowrite(drive, lba, block);
        }

        // Follow indirect chain
        lba = last.ptr;
    }
}

pub fn free_link_chain(drive: &mut dyn StorageDrive, link: LinkEntry) -> FSReturn<()> {
    let mut stack = Vec::new();
    stack.push(link);

    while let Some(entry) = stack.pop() {
        if !entry.is_valid() { continue }
        if entry.is_direct() {
            let lbas = entry.size.div_ceil(LBA_SIZE);
            super::bitmap::free_range(drive, entry.ptr, lbas)?;
        } else {
            let linkblock: LinkBlock = io::read_metadata(drive, entry.ptr)?;
            for e in linkblock.entries { stack.push(e) }
            super::bitmap::free_range(drive, entry.ptr, 1)?;
        }
    }
    Ok(())
}