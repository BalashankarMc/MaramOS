use core::fmt::Debug;

use alloc::string::String;

#[repr(C, packed)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Guid {
    time_low: u32,
    time_mid: u16,
    time_high_ver: u16,
    clock_seq: [u8; 2], // Big Endian
    node: [u8; 6] // Big Endian
}

impl Guid {
    pub const fn new(time_low: u32, time_mid: u16, time_high_ver: u16, clock_seq: u16, node: u64) -> Self {
        let mut node_arr = [0; 6];
        node_arr.copy_from_slice(&node.to_be_bytes()[2..8]);

        Self {
            time_low,
            time_mid,
            time_high_ver,
            clock_seq: clock_seq.to_be_bytes(),
            node: node_arr
        }
    }

    pub const ZERO: Self = { Self::new(0, 0, 0, 0, 0) };
}

impl Debug for Guid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let t_low = self.time_low;
        let t_mid = self.time_mid;
        let t_high_ver = self.time_high_ver;
        let clock = u16::from_be_bytes(self.clock_seq);
        let mut arr = [0; 8];
        arr[2..].copy_from_slice(&self.node);
        let node = u64::from_be_bytes(arr);

        write!(f, "{t_low:08X}-{t_mid:04X}-{t_high_ver:04X}-{clock:04X}-{node:12X}")
    }
}

#[repr(C, packed)]
#[derive(Clone)]
pub struct PrimaryGPTHeader {
    pub signature: [u8; 8],
    pub revision: u32,
    pub size: u32,
    pub header_crc: u32,
    reserved: u32,
    pub curr_block: u64,
    pub backup_block: u64,
    pub first_usable: u64,
    pub last_usable: u64,
    pub disk_guid: Guid,
    pub entry_start: u64,
    pub entry_count: u32,
    pub entry_byte_count: u32,
    pub entry_array_crc: u32
}

#[repr(C, packed)]
pub struct PartitionEntry {
    pub type_guid: Guid,
    pub partition_guid: Guid,
    pub start_block: u64, // Inclusive
    pub end_block: u64, // Inclusive
    pub flags: u64,
    pub name: [u16; 36]
}

impl Debug for PartitionEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let t = super::PartitionType::from_guid(self.type_guid);
        let partition = self.partition_guid;
        let start = self.start_block;
        let end = self.end_block;
        let name_raw = self.name;
        let end_idx = name_raw.iter().position(|&d| d == 0).unwrap_or(36);
        let name = String::from_utf16_lossy(&name_raw[..end_idx]);

        write!(f, "Name: {name}\nType: {t}\nPart Guid: {partition:?}, start: {start}, end: {end}")
    }
}