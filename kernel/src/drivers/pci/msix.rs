//! MSI-X interrupt programming helper.
//!
//! Provides [`program`] which locates the MSI-X capability on a PCI function,
//! maps the MSI-X table BAR, writes the message address/data for one entry,
//! and enables MSI-X.

use super::PCIDevice;
use crate::{descriptors::interrupts::HardwareInterrupts, memory::{KMemory, PAGE_SIZE}};

use x86_64::PhysAddr;

/// Program a single MSI-X entry on `dev`.
///
///   * `entry_index` — which table entry to program (0-based).
///   * `vector` — the APIC vector to fire on completion.
///
/// Returns `true` on success, `false` if the device lacks MSI-X, the entry
/// index is out of range, or the table BAR could not be mapped.
///
/// The message address targets the local APIC (`0xFEE00000 | lapic_id << 12`).
pub fn program(dev: &dyn PCIDevice, entry_index: u16, vector: HardwareInterrupts) -> bool {
    // Capability ID 0x11 = MSI-X.
    let cap = match dev.find_capability(0x11) {
        Some(c) => c as u16,
        None => return false,
    };

    // Capability + 2: message control — bits 10:0 = table size − 1.
    let msg_ctrl = dev.read_u16(cap + 2);
    let table_size = (msg_ctrl & 0x7FF) + 1;
    if entry_index >= table_size {
        return false;
    }

    // Capability + 4: table BAR indicator / offset.
    let table_reg = dev.read_u32(cap + 4);
    let bir = (table_reg & 0x7) as usize;
    let table_offset = (table_reg & !0x7) as u64;

    let bar = match dev.bar(bir) {
        Some(b) => b,
        None => return false,
    };

    let bar_virt = KMemory::map_mmio(
        PhysAddr::new(bar.address),
        (bar.size as usize).div_ceil(PAGE_SIZE),
    );

    let entry_virt = (bar_virt + table_offset) + (entry_index as u64) * 16;

    let lapic_id = crate::acpi::lapic_id();
    let msg_addr = 0xFEE00000u64 | ((lapic_id as u64) << 12);
    let msg_data = vector as u32;

    // Write the 16-byte MSI-X entry: msg_addr (QWord), msg_data (DWord),
    // vector control (DWord, 0 = unmasked).
    unsafe {
        entry_virt.as_mut_ptr::<u64>().write_volatile(msg_addr);
        entry_virt
            .as_mut_ptr::<u32>()
            .add(2)
            .write_volatile(msg_data);
        entry_virt.as_mut_ptr::<u32>().add(3).write_volatile(0);
    }

    // Enable MSI-X (bit 15), clear function mask (bit 14).
    let mut ctrl = dev.read_u16(cap + 2);
    ctrl = (ctrl | (1 << 15)) & !(1 << 14);
    dev.write_u16(cap + 2, ctrl);

    // Set PCI command register bit 10 (MSI enable).
    let mut cmd = dev.read_u32(0x04);
    cmd |= 1 << 10;
    dev.write_u32(0x04, cmd);

    true
}
