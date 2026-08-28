use core::fmt::Display;

use alloc::{format, string::String, sync::Arc, vec::Vec};
use crate::{helpers::crc32, log_warn};
use super::{StorageDrive, GPTError};

mod mbr;
mod headers;

pub use headers::Guid;
use mbr::ProtectiveMBR;
use headers::{PrimaryGPTHeader, PartitionEntry};

type Disk<'a> = &'a Arc<dyn StorageDrive>;

const GPT_SIG: [u8; 8] = *b"EFI PART";

// GUIDs
const EFI_GUID: Guid = Guid::new(0xC12A_7328, 0xF81F, 0x11D2, 0xBA4B, 0xA0_C93E_C93B);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PartitionType {
    Efi,
    LemonFS,
    Unknown(Guid)
}

impl Display for PartitionType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::Efi => "EFI",
            Self::LemonFS => "LemonFS",
            Self::Unknown(guid) => &format!("Unknown: {guid:?}")
        };

        f.write_str(s)
    }
}

impl PartitionType {
    const fn guid(self) -> Guid {
        match self {
            Self::Efi => EFI_GUID,
            Self::LemonFS => crate::fs::LEMONFS_GUID,
            Self::Unknown(guid) => guid
        }
    }

    const fn from_guid(guid: Guid) -> Self {
        match guid {
            crate::fs::LEMONFS_GUID => Self::LemonFS,
            EFI_GUID => Self::Efi,
            _ => Self::Unknown(guid)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Partition {
    pub start: u64,
    pub size_blocks: u64,
    pub name: String,
    pub type_: PartitionType,
    pub guid: Guid
}

pub fn init(drive: Disk) -> Result<Vec<Partition>, GPTError> {
    let mbr_block = drive.read_smart(0, 1).map_err(|_| GPTError::IOError)?;
    let mbr = mbr_block.read_data::<ProtectiveMBR>(0).ok_or(GPTError::IOError)?;

    if mbr.boot_signature != 0xAA55 { return Err(GPTError::BadMBR) }
    if mbr.protective_partition.type_ != 0xEE { log_warn!("MBR Protective Partition is not of type 0xEE!") }

    let mut is_backup = false;

    let pages = drive.read_smart(1, 1).map_err(|_| GPTError::IOError)?;
    let mut gpt_header = pages.read_data::<PrimaryGPTHeader>(0).ok_or(GPTError::IOError)?;

    // Safety: Turning a valid memory region into a slice. Safe
    let mut header_bytes = unsafe { core::slice::from_raw_parts_mut(pages.address().as_mut_ptr::<u8>(), gpt_header.size as usize) };

    if !check_gpt_header(drive, &gpt_header, header_bytes, is_backup) {
        let backup_lba = drive.block_count() - 1;
        drive.read_blocks(backup_lba, 1, &pages).map_err(|_| GPTError::IOError)?;
        is_backup = true;

        gpt_header = pages.read_data(0).ok_or(GPTError::IOError)?;
        // Safety: Turning a valid memory region into a slice. Safe
        header_bytes = unsafe { core::slice::from_raw_parts_mut(pages.address().as_mut_ptr::<u8>(), gpt_header.size as usize) };

        if !check_gpt_header(drive, &gpt_header, header_bytes, is_backup) { return Err(GPTError::BadHeader) }
    }

    let entry_count = u64::from(gpt_header.entry_count);
    let entry_byte_count = u64::from(gpt_header.entry_byte_count);

    let total = (entry_count * entry_byte_count).min(1024 * 1024 * 16);
    let array_block_count = total.div_ceil(drive.block_size());

    let mut entry_blocks = drive.read_smart(gpt_header.entry_start, array_block_count)
        .map_err(|_| GPTError::IOError)?;

    let mut offset = 0;
    let mut res: Vec<Partition> = Vec::new();
    let mut array_ptr = entry_blocks.address().as_ptr::<u8>();

    // Safety: Turning a known ok page address into a slice
    let mut slice = unsafe { core::slice::from_raw_parts(array_ptr, total as usize) };

    if crc32(slice) != gpt_header.entry_array_crc {
        let backup_header_page = drive.read_smart(gpt_header.backup_block, 1)
            .map_err(|_| GPTError::IOError)?;
        
        if !is_backup {
            gpt_header = backup_header_page.read_data::<PrimaryGPTHeader>(0).ok_or(GPTError::IOError)?;
            let addr = backup_header_page.address().as_mut_ptr::<u8>();
            is_backup = true;

            // Safety: Same as last
            let header_bytes = unsafe { core::slice::from_raw_parts_mut(addr, gpt_header.size as usize) };

            check_gpt_header(drive, &gpt_header, header_bytes, is_backup).ok_or(GPTError::BadHeader)?;    
        }

        let backup_array_start = gpt_header.entry_start;

        let entry_count = u64::from(gpt_header.entry_count);
        let entry_byte_count = u64::from(gpt_header.entry_byte_count);

        let total = (entry_count * entry_byte_count).min(1024 * 1024 * 16);
        let array_block_count = total.div_ceil(drive.block_size());

        entry_blocks = drive.read_smart(backup_array_start, array_block_count).map_err(|_| GPTError::IOError)?;
        array_ptr = entry_blocks.address().as_ptr::<u8>();

        // Safety: Same as last
        slice = unsafe { core::slice::from_raw_parts(array_ptr, total as usize) };

        if crc32(slice) != gpt_header.entry_array_crc { return Err(GPTError::EntryCRCFailed) }
    }

    while let Some(entry) = entry_blocks.read_data::<PartitionEntry>(offset) {
        if entry.type_guid == Guid::ZERO { break }

        if let Some(p) = res.last() && entry.start_block < p.start + p.size_blocks { continue } // Overlapping partitions

        // Skip bad entries
        if entry.start_block < gpt_header.first_usable || entry.end_block > gpt_header.last_usable { continue }
        if entry.start_block > entry.end_block { continue }

        let name_buf = entry.name;
        let end_idx = name_buf.iter().position(|&x| x == 0).unwrap_or(36);
        let name = String::from_utf16_lossy(&name_buf[..end_idx]);

        let partition = Partition {
            name,
            size_blocks: entry.end_block - entry.start_block + 1, // Inclusive
            start: entry.start_block,
            type_: PartitionType::from_guid(entry.type_guid),
            guid: entry.partition_guid
        };

        res.push(partition);
        offset += gpt_header.entry_byte_count as usize;
    }
    
    Ok(res)
}

fn check_gpt_header(drive: Disk, header: &PrimaryGPTHeader, header_bytes: &mut [u8], is_backup: bool) -> bool {
    // Check signature and size
    if header.signature != GPT_SIG || header.size < 92 || u64::from(header.size) > drive.block_size() { return false }

    // Validate MyLBA
    let expected_block = if is_backup { drive.block_count() - 1 } else { 1 };
    if header.curr_block != expected_block { return false }

    // CRC(Checksum) verification
    header_bytes[16..20].fill(0); // Zero the CRC Region
    let crc32 = crc32(header_bytes);
    if crc32 != header.header_crc { return false }

    // Entry spec-validation
    if header.entry_count < 128 { return false }
    if header.entry_byte_count < 128 { return false }
    let Some(total) = u64::from(header.entry_count).checked_mul(u64::from(header.entry_byte_count)) else { return false };

    // Bounds checking
    let alternate = if is_backup { 1 } else { drive.block_count() - 1 };
    if header.first_usable > header.last_usable || header.last_usable > drive.block_count() || header.entry_start <= 1 { return false }
    if header.backup_block != alternate { return false }

    let array_sectors = total.div_ceil(drive.block_size());
    if header.entry_start.checked_add(array_sectors) > Some(header.first_usable) { return false }

    true
}