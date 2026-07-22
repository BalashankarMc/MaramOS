//! GPT on-disk data structures.
//!
//! Packed representations of the Protective MBR, GPT Header, and GPT
//! Partition Entry. All structs are `#[repr(C, packed)]` to match the
//! on-disk layout exactly.

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ProtectiveMBR {
    pub boot_code: [u8; 440], // Unused under GPT
    pub disk_signature: u32, // Legacy BIOS field
    _reserved: u16,
    pub partition_entry: ProtectivePartitionEntry,
    _reserved1: [u8; 48], // Unused Partition Entries
    pub boot_sig: u16 // 0x55AA
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct ProtectivePartitionEntry {
    pub boot_indicator: u8, // 0 (Non-Bootable)
    pub start_chs: [u8; 3], // Filler
    pub os_type: u8, // 0xEE (GPT Protective)
    pub end_chs: [u8; 3], // Filler
    
    pub start_lba: u32, // 1
    pub size_lbas: u32 // 0xFFFFFFFF if disk exceeds 32-bit LBA range
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct PrimaryGPTHeader {
    /// Must be *b"EFI PART"
    pub signature: [u8; 8],
    /// Usually 0x00010000
    pub revision: u32,
    /// Size of the header in bytes
    pub size: u32,
    // CRC32 of the header
    pub checksum: u32,

    /// Must be 0
    _reserved: u32,
    /// LBA of this header
    pub this_lba: u64,
    /// LBA of the backup header
    pub alternate_lba: u64,
    /// First usable LBA of this partition
    pub start_lba: u64,

    /// Last usable LBA
    pub end_lba: u64,
    /// Unique disk identifier
    pub disk_guid: [u8; 16],
    /// Starting LBA of the GUID Partition Entry array
    pub part_entry_lba: u64,
    /// Entry count in the array
    pub entry_count: u32,

    /// Bytes per entry
    pub entry_size: u32,
    /// CRC32 over the entire raw entry array bytes
    pub entry_array_crc: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct GPTPartitionEntry {
    /// FS / Role Dependant
    pub partition_type: [u8; 16],
    /// Unique Partition ID
    pub guid: [u8; 16],
    /// Start of the partition (Inclusive)
    pub start_lba: u64,
    /// End of the partition (Inclusive)
    pub end_lba: u64,

    /// Bitfield (0: Platform req, 1: EFI Ignore, 2: BIOS Bootable ...)
    pub flags: u64,
    /// Partition name in UTF16
    pub name: [u16; 36],
}