//! CRC32 Checksum implemetation (IEEE 802.3)

const POLYNOMIAL: u32 = 0xEDB8_8320;

const CRC_TABLE: [u32; 256] = { // No for loops in consts :(
    let mut table = [0; 256];
    let mut i = 0;

    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 { crc = (crc >> 1) ^ POLYNOMIAL }
            else { crc >>= 1 }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute the CRC checksum of the input
pub fn crc32(data: &[u8]) -> u32 {

    let mut crc = u32::MAX;

    for &byte in data {
        let idx = (crc ^ u32::from(byte)) & 0xFF;
        crc = (crc >> 8) ^ CRC_TABLE[idx as usize];
    }

    crc ^ u32::MAX
}