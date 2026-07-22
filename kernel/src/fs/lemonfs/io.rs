//! LemonFS metadata I/O helpers.
//!
//! Single-LBA reads and writes for on-disk structures (superblock, hash
//! table blocks, bitmap blocks, directory entries) with dirty-flag
//! management.

use super::headers::*;
use crate::{
    drivers::storage::{LBA_SIZE, StorageDrive},
    fs::{FSError, FSReturn}, memory::{KMemory, PhysPage}
};

/// Read a single LBA metadata block from the drive.
/// Will return FSError::InvalidAccess if sizeof::<T>() > LBA_SIZE or IO on IOError
pub fn read_metadata<T: Copy>(drive: &mut dyn StorageDrive, lba: u64) -> FSReturn<T> {
    if core::mem::size_of::<T>() > LBA_SIZE as usize { return Err(FSError::InvalidAccess) }

    let mut temp = KMemory::alloc_page();
    drive.read_lbas(lba, 1, &mut temp).map_err(|_| FSError::IO)?;

    Ok(temp.read_data(0))
}

pub fn write_metadata<T>(drive: &mut dyn StorageDrive, lba: u64, data: T) -> FSReturn<()> {
    if core::mem::size_of::<T>() > LBA_SIZE as usize { return Err(FSError::InvalidAccess) }

    let mut temp = KMemory::alloc_page();
    temp.write_data(0, data);
    drive.write_lbas(lba, 1, &temp).map_err(|_| FSError::IO)
}

/// Sets the Superblocks dirty byte. Return whether a write is necessary
pub fn set_dirty(superblock: &mut SuperBlock, dirty: bool) -> bool {
    if superblock.dirty == dirty { return false }
    superblock.dirty = dirty;
    true
}
