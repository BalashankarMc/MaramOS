//! Command management module

/// Packed u8 for FIS info
#[repr(transparent)]
pub struct FISInfo(u8);

impl FISInfo {
    pub fn set(&mut self, fis_len: u8, to_device: bool) {
        self.0 = (self.0 & !0x1F) | fis_len;
        self.0 = (self.0 & !0x40) | u8::from(to_device) << 6;
    }

}

#[repr(C, packed)]
pub struct CommandHeader {
    // DWORD 0
    pub fis_info: FISInfo,
    pub cmd_info: u8,
    pub prd_table_len: u16,

    // DWORD 1
    pub prd_byte_count: u32,

    // DWORD 2 & 3
    pub cmd_table_base_addr_lower: u32,
    pub cmd_table_base_addr_upper: u32,

    // DWORD 4 -> 7
    _reserved: [u32; 4]
}

impl CommandHeader {
    pub const fn zeroed() -> Self {
        unsafe { core::mem::zeroed() }
    }
}

/// Packed u32 for Descriptor Byte Count and Interrupt on completion
#[repr(transparent)]
pub struct DescByteCountU32(u32);

impl DescByteCountU32 {
    pub fn set_descriptor_byte_count(&mut self, byte_count: u32) {
        assert!(byte_count > 0 && byte_count <= (1 << 22));
        self.0 = (self.0 & 0xFFC0_0000) | (byte_count - 1);
    }
}

#[repr(C, packed)]
pub struct PhysRegionDescTableEntry {
    pub data_base_addr: u64,
    _reserved: u32,
    pub dbc: DescByteCountU32
}

#[repr(C, packed)]
pub struct CommandTableHeader {
    pub command_fis: [u8; 64],
    pub atapi_cmd: [u8; 16],
    reserved: [u64; 6]
}

impl CommandTableHeader {
    pub const fn zeroed() -> Self {
        unsafe { core::mem::zeroed() }
    }
}