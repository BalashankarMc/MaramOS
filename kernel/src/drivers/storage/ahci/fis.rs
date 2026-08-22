//! Frame Information Structure (FIS) Management Module

/// Defines FIS types specified in SATA 3.0
#[repr(u8)]
pub enum FISType {
    RegisterHostToDevice = 0x27
}
/// Host to Device FIS Registration
#[repr(C, packed)]
pub struct FISRegisterH2D {
    // DWORD 0
    pub fis_type: FISType,
    pub port_multiplier_command: u8,
    pub command: u8,
    pub feature_low: u8,

    // DWORD 1
    pub lba0: u8,
    pub lba1: u8,
    pub lba2: u8,
    pub device: u8,

    // DWORD 2
    pub lba3: u8,
    pub lba4: u8,
    pub lba5: u8,
    pub feature_high: u8,

    // DWORD 3
    pub count: u16,
    pub icc: u8,
    pub control: u8,

    // DWORD 4 (reserved)
    _reserved: [u8; 4]
}

impl FISRegisterH2D {
    pub const fn new(port_mult: u8, command: u8, feature: u16, lba: u64, count: u16, icc: u8, control: u8) -> Self {
        Self {
            fis_type: FISType::RegisterHostToDevice,
            port_multiplier_command: port_mult,
            command,
            feature_low: feature as u8,
            feature_high: (feature >> 8) as u8,
            lba0: lba as u8,
            lba1: (lba >> 8) as u8,
            lba2: (lba >> 16) as u8,
            lba3: (lba >> 24) as u8,
            lba4: (lba >> 32) as u8,
            lba5: (lba >> 40) as u8,
            device: 0x40,
            count,
            icc,
            control,
            _reserved: [0; 4]
        }
    }
    pub const fn to_bytes(&self) -> &[u8] {
        let ptr = core::ptr::from_ref(self).cast::<u8>();
        unsafe { core::slice::from_raw_parts(ptr, size_of::<Self>()) }
    }
}
