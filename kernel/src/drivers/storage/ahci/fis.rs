//! Frame Information Structure (FIS) Management Module

/// Defines FIS types specified in SATA 3.0
#[repr(u8)]
pub enum FISType {
    RegisterHostToDevice = 0x27,
    RegisterDeviceToHost = 0x34,
    DMAActivate = 0x39,
    DMASetup = 0x41,
    Data = 0x46,
    BISTActivate = 0x58,
    PIOSetup = 0x5F,
    SetDeviceBits = 0xA1
}

/// Host to Device FIS Registeration
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
    pub count_low: u8,
    pub count_high: u8,
    pub icc: u8,
    pub control: u8,

    // DWORD 4 (reserved)
    _reserved1: [u8; 4]
}

impl FISRegisterH2D {
    pub fn new(port_mult: u8, command: u8, feature: u16, lba: u64, count: u16, icc: u8, control: u8) -> Self {
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
            count_low: count as u8,
            count_high: (count >> 8) as u8,
            icc,
            control,
            _reserved1: [0; 4]
        }
    }
}

/// Device to Host FIS Registration
#[repr(C, packed)]
pub struct FISRegisterD2H {
    // DWORD 0
    fis_type: FISType,
    port_multiplier_interrupt: u8,
    status: u8,
    error: u8,

    // DWORD 1
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8,
    
    // DWORD 2
    lba3: u8,
    lba4: u8,
    lba5: u8,
    _reserved0: u8,

    // DWORD 3
    count_low: u8,
    count_high: u8,
    _reserved1: [u8; 2],

    // DWORD 4 (reserved)
    _reserved2: [u8; 4]
}

/// Data FIS Header (Bidirectional)
#[repr(C, packed)]
pub struct FISDataHeader {
    fis_type: FISType,
    port_multiplier: u8,
    _reserved: [u8; 2]
}

/// Device to Host PIO Setup
#[repr(C, packed)]
pub struct FISPIOSetup {
    // DWORD 1
    fis_type: FISType,
    port_multiplier: u8,
    status: u8,
    error: u8,

    // DWORD 1
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8,
    
    // DWORD 2
    lba3: u8,
    lba4: u8,
    lba5: u8,
    _reserved0: u8,

    // DWORD 3
    count_low: u8,
    count_high: u8,
    _reserved1: u8,
    e_status: u8,

    // DWORD 4
    transfer_count: u16,
    _reserved2: [u8; 2]
}

/// Device to Host DMA Setup
#[repr(C, packed)]
pub struct FISDMASetup {
    // DWORD 1
    fis_type: FISType,
    port_multiplier_data: u8,
    _reserved0: [u8; 2],

    //DWORD 1 & 2
    dma_buffer_id: u64,

    // DWORD 3
    _reserved1: [u8; 4],

    // DWORD 4
    dma_buffer_offset: u32, // First 2 bits must be 0

    // DWORD 5
    transfer_count: u32, //Bit 0 must be 0

    // DWORD 6
    _reserved2: [u8; 4]
}