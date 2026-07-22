//! GPT integrity verification and partition enumeration.
//!
//! Validates the protective MBR boot signature, verifies the primary GPT
//! header CRC32 (falling back to the backup header on failure), and
//! enumerates partition entries with CRC32 validation.

use alloc::{sync::Arc, vec::Vec};
use spin::Mutex;

use crate::{drivers::storage::{Drive, LBA_SIZE, StorageDrive}, library::crc32};

use super::{headers::*, GPTError, GResult};

pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2A, 0xC1,
    0x1F, 0xF8, 0xD2, 0x11,
    0xBA, 0x4B, 0x00, 0xA0,
    0xC9, 0x3E, 0xC9, 0x3B,
];

/// Verifies the integrity of the given drive (GPT).
/// Returns None for an error, false for warnings and true for valid GPT.
pub fn attest_integrity(drive: Arc<Mutex<Drive>>) -> GResult<PrimaryGPTHeader> {

    let page = drive.lock().read_smart(0, 2).map_err(|_|GPTError::IO)?;

    let mbr: ProtectiveMBR = page.read_data(0);

    if mbr.boot_sig != 0xAA55 {
        log_error!("MBR Boot Signature is not 0xAA55!");
        return Err(GPTError::InvalidProtectiveMBR)
    }
    if mbr.partition_entry.os_type != 0xEE { warn!("MBR OS Type is not valid!") }

    let gpt_header: PrimaryGPTHeader = page.read_data(LBA_SIZE as usize);
    if gpt_header.signature != *b"EFI PART" {
        warn!("Primary GPT Header is corrupt. Attempting backup header");
        return attest_backup(drive)
    }
    if gpt_header.size < 92 || gpt_header.size as u64 > LBA_SIZE { return Err(GPTError::BadGPTHeader) }

    let mut temp = gpt_header;
    temp.checksum = 0;

    let addr = &temp as *const PrimaryGPTHeader as *const u8;
    let temp_slice = unsafe { core::slice::from_raw_parts(addr, gpt_header.size as usize) };

    if gpt_header.checksum != crc32(temp_slice) { return attest_backup(drive) }
    if gpt_header.end_lba > drive.lock().capacity() / LBA_SIZE || gpt_header.start_lba > gpt_header.end_lba {
        return attest_backup(drive)
    }

    Ok(gpt_header)
}

fn attest_backup(drive: Arc<Mutex<Drive>>) -> GResult<PrimaryGPTHeader> {
    let total_sectors = drive.lock().capacity() / LBA_SIZE;
    let backup_lba = total_sectors - 1;

    let page = drive.lock().read_smart(backup_lba, 1).map_err(|_| GPTError::IO)?;
    let header: PrimaryGPTHeader = page.read_data(0);

    if header.signature != *b"EFI PART" { return Err(GPTError::BadGPTHeader) }
    if header.size < 92 || header.size as u64 > LBA_SIZE { return Err(GPTError::BadGPTHeader) }

    // CRC check
    let mut temp = header;
    temp.checksum = 0;
    let addr = &temp as *const PrimaryGPTHeader as *const u8;
    let slice = unsafe { core::slice::from_raw_parts(addr, header.size as usize) };
    if header.checksum != crc32(slice) { return Err(GPTError::BadGPTHeader) }

    if header.end_lba > drive.lock().capacity() / LBA_SIZE || header.start_lba > header.end_lba { return Err(GPTError::BadGPTHeader) }

    Ok(header)
}

pub fn parse_partitions(drive: Arc<Mutex<Drive>>, header: PrimaryGPTHeader) -> GResult<Vec<GPTPartitionEntry>> {
    let start = header.part_entry_lba;
    let count = header.entry_count as u64;

    let mut res: Vec<GPTPartitionEntry> = Vec::new();
    let page = drive.lock().read_smart(
        start,
        (count * size_of::<GPTPartitionEntry>() as u64).div_ceil(LBA_SIZE)
    ).map_err(|_| GPTError::IO)?;

    let array_bytes = (count * size_of::<GPTPartitionEntry>() as u64) as usize;
    let array_slice = unsafe { core::slice::from_raw_parts(page.get_virt_addr().as_ptr::<u8>(), array_bytes) };

    if crc32(array_slice) != header.entry_array_crc { return Err(GPTError::BadPartition) }

    for i in 0..count as usize {
        let entry: GPTPartitionEntry = page.read_data(i * size_of::<GPTPartitionEntry>());
        
        if entry.partition_type == [0; 16] { continue }
        if entry.start_lba > entry.end_lba || entry.start_lba < header.start_lba || entry.end_lba > header.end_lba {
            warn!("Skipping partition {}: Invalid size", i + 1);
            continue
        }

        if entry.partition_type == crate::fs::LEMON_GUID { log_success!("Found LemonFS Partition") }

        res.push(entry);
    }

    Ok(res)
}