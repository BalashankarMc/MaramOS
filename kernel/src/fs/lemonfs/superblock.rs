use crate::fs::FSError;

use super::{Disk, FSResult, disk::{SuperBlock, SuperBlockFlags}, io::{read_data, write_data}};

/// Gets the superblock from the drive and falls back to the backup if necessary.
/// 
/// # Return
/// Returns (`Superblock`, `bool`), where the second entry is `true` if we had to use the backup
pub fn get_superblock(disk: Disk) -> FSResult<(SuperBlock, bool)> {
    let sb: SuperBlock = read_data(disk, 0)?;
    if !check_sb(&sb) { // Fallback to backup
        let backup: SuperBlock = read_data(disk, disk.sector_count() - 1)?;
        if !check_sb(&backup) { return Err(FSError::BadSuperblock) }
        return Ok((backup, true))
    }
    Ok((sb, false))
}

pub fn write_sb(disk: Disk, sb: &SuperBlock) -> FSResult<()> {
    let mut flags = sb.flags;
    flags &= !SuperBlockFlags::DIRTY;
    flags |= SuperBlockFlags::BACKUP_SYNC;

    let mut superblock = *sb;
    superblock.flags = flags;
    superblock.checksum = 0;
    superblock.checksum = get_sb_checksum(&superblock);

    write_data(disk, 0, superblock)?;
    write_data(disk, disk.sector_count() - 1, superblock)?;

    Ok(())
}

fn check_sb(sb: &SuperBlock) -> bool {
    let mut clone = *sb;
    clone.checksum = 0;
    sb.checksum == get_sb_checksum(&clone)
}

fn get_sb_checksum(sb: &SuperBlock) -> u64 {
    let ptr = core::ptr::from_ref(sb).cast::<u8>();
    
    // Safety: Casting *const u8 into &[u8]. Safe
    let slice = unsafe { core::slice::from_raw_parts(ptr, size_of::<SuperBlock>()) };

    crate::helpers::crc64(slice)
}
