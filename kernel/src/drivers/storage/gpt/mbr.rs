#[repr(C, packed)]
pub struct ProtectiveMBR {
    boot_code_area: [u8; 440],
    disk_sig: u32,
    reserved: u16,
    pub protective_partition: MBRPartition,
    partitions: [MBRPartition; 3],
    pub boot_signature: u16
}

#[repr(C, packed)]
pub struct MBRPartition {
    pub boot_indicator: u8, // 0x0
    start_chs: [u8; 3],
    pub type_: u8, // 0xEE
    end_chs: [u8; 3],
    pub start_block: u32, // 0x1
    pub size: u32 // Disk Size -1 or u32::MAX
}