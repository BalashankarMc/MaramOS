//! CRC32 Checksum implementation (IEEE 802.3) and
//! CRC64 Checksum implementation (CRC-64 / XZ)

const CRC32_POLYNOMIAL: u32 = 0xEDB8_8320;
const CRC64_POLYNOMIAL: u64 = 0xC96C_5795_D787_0F42;

const CRC32_TABLE: [u32; 256] = { // No for loops in consts :(
    let mut table = [0; 256];
    let mut i = 0;

    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 { crc = (crc >> 1) ^ CRC32_POLYNOMIAL }
            else { crc >>= 1 }
            j += 1;
        }

        table[i as usize] = crc;
        i += 1;
    }
    table
};

const CRC64_TABLE: [u64; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 { crc = (crc >> 1) ^ CRC64_POLYNOMIAL }
            else { crc >>= 1 }
            j += 1;
        }

        table[i as usize] = crc;
        i += 1;
    }

    table
};

/// Compute the CRC32 checksum of the `data`
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;

    for &byte in data {
        let idx = (crc ^ u32::from(byte)) & 0xFF;
        crc = (crc >> 8) ^ CRC32_TABLE[idx as usize];
    }

    crc ^ u32::MAX
}

/// Compute the CRC64 checksum of the `data`
pub fn crc64(data: &[u8]) -> u64 {
    let mut crc = u64::MAX;

    for &byte in data {
        let idx = (crc ^ u64::from(byte)) & 0xFF;
        crc = (crc >> 8) ^ CRC64_TABLE[idx as usize];
    }

    crc ^ u64::MAX
}