//! MSI-X interrupt programming helper.

use super::PCIFunction;
use crate::{descriptors::HardwareInterrupts, memory::MMIORegion};

/// Program a single MSI-X entry on `dev`.
/// 
/// # Arguments
/// `dev`: A reference to the `PCIFunction` to be programmed
/// `region`: The mapped BIR BAR region
/// `entry_index`: The table entry to program
/// `vector`: The Interrupt vector to fire
pub fn program(dev: &PCIFunction, region: &MMIORegion, entry_index: u16, vector: HardwareInterrupts) -> Option<()> {
    // Capability ID 0x11 = MSI-X.
    let cap = u16::from(dev.find_capability(0x11)?);

    // Capability + 2: message control — bits 10:0 = table size − 1.
    let msg_ctrl = dev.read_u16(cap + 2)?;
    let table_size = (msg_ctrl & 0x7FF) + 1;
    if entry_index >= table_size { return None }

    // Capability + 4: table BAR indicator / offset.
    let table_reg = dev.read_u32(cap + 4)?;
    let table_offset = u64::from(table_reg & !0x7);

    let lapic_id = crate::acpi::lapic_id();
    let msg_addr = 0xFEE0_0000 | (u64::from(lapic_id) << 12);
    let msg_data = vector as u32;

    // Write the 16-byte MSI-X entry: msg_addr (QWord), msg_data (DWord),
    // vector control (DWord, 0 = unmasked).
    let offset = (table_offset + u64::from(entry_index) * 16) as usize;
    if !region.write(offset, msg_addr) { return None }
    if !region.write(offset + 8, msg_data) { return None }
    if !region.write(offset + 12, 0) { return None }

    // Enable MSI-X (bit 15), clear function mask (bit 14).
    let mut ctrl = dev.read_u16(cap + 2)?;
    ctrl = (ctrl | (1 << 15)) & !(1 << 14);
    dev.write_u16(cap + 2, ctrl);

    // Set PCI command register bit 10 (MSI enable).
    let mut cmd = dev.read_u32(0x04)?;
    cmd |= 1 << 10;
    dev.write_u32(0x04, cmd);

    Some(())
}
