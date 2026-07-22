//! MSI interrupt programming helper.
//!
//! Provides [`program`] which locates the MSI capability on a PCI function,
//! writes the message address/data registers, and enables MSI.

use crate::descriptors::interrupts::HardwareInterrupts;

use super::PCIDevice;

/// Program MSI on `dev` with a single vector.
///
/// Locates the MSI capability (ID 0x05), writes the message address
/// targeting the local APIC of the BSP, sets `vector` in message data,
/// and enables MSI (disabling INTx via command register bit 10).
///
/// Both 32-bit and 64-bit MSI are handled transparently.  If the device
/// supports per-vector masking, all vectors are unmasked.
///
/// Returns `true` on success, `false` if the device lacks an MSI capability.
pub fn program(dev: &dyn PCIDevice, vector: HardwareInterrupts) -> bool {
    let cap = match dev.find_capability(0x05) {
        Some(c) => c as u16,
        None => return false,
    };

    let msg_ctrl = dev.read_u16(cap + 2);
    let is_64bit = (msg_ctrl >> 7) & 1 == 1;
    let per_vector_masking = (msg_ctrl >> 8) & 1 == 1;

    let lapic_id = crate::acpi::lapic_id();
    let msg_addr = 0xFEE00000u32 | (lapic_id << 12);

    // Delivery Mode = 000 (Fixed), Destination Mode = 0 (Physical),
    // Redirection Hint = 0 (specific CPU), Vector = `vector`.
    let msg_data: u16 = vector as u16;

    // Write Message Address (low 32 bits).
    dev.write_u32(cap + 4, msg_addr);

    if is_64bit {
        // Upper address — 0 for x86 (physical APIC address < 4 GiB).
        dev.write_u32(cap + 8, 0);
        // Message Data at cap + 12.
        dev.write_u16(cap + 12, msg_data);
    } else {
        // Message Data at cap + 8.
        dev.write_u16(cap + 8, msg_data);
    }

    // Unmask all vectors when per-vector masking is supported.
    if per_vector_masking {
        let mask_off = if is_64bit { cap + 16 } else { cap + 12 };
        dev.write_u32(mask_off, 0);
    }

    // Enable MSI (bit 0), set MME to 000 (1 message).
    let mut ctrl = dev.read_u16(cap + 2);
    ctrl |= 1;
    ctrl &= !(0x7 << 4);
    dev.write_u16(cap + 2, ctrl);

    // Disable INTx (command register bit 10).
    let mut cmd = dev.read_u32(0x04);
    cmd |= 1 << 10;
    dev.write_u32(0x04, cmd);

    true
}
