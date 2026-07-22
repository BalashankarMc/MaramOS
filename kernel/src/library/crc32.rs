//! CRC32 IEEE 802.3 implementation.
//!
//! Uses a compile-time generated 256-entry lookup table and the reflected
//! polynomial form. The [`crc32`] function processes a byte slice and
//! returns the standard CRC32 checksum.

/// CRC32 IEEE 802.3 polynomial (reflected form).
const POLYNOMIAL: u32 = 0xEDB88320;

/// Pre-computed 256-entry lookup table.
/// Each entry i is the CRC of the single byte 0x00..=0xFF.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ POLYNOMIAL;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Compute CRC32 (IEEE 802.3) over `data`, starting from `0xFFFFFFFF`
/// and XORing the final result with `0xFFFFFFFF`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        let idx = (crc ^ byte as u32) & 0xFF;
        crc = (crc >> 8) ^ CRC_TABLE[idx as usize];
    }
    crc ^ 0xFFFFFFFF
}