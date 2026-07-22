//! I/O APIC driver.
//!
//! MMIO register access for the I/O APIC: read/write redirect table entries,
//! mask/unmask IRQs, and map GSI numbers to CPU vectors.

#![allow(dead_code)] // Just not used yet

use crate::library::LateInit;
use crate::memory::KMemory;

use super::apic_internal;

use x86_64::{PhysAddr, VirtAddr};

const IOAPIC_DATA_REG: u64 = 0x10;

static IOAPIC_REGS: LateInit<VirtAddr> = LateInit::new();

pub fn init() {
    let phys = *apic_internal::IO_APIC_BASE.get() as u64;
    let regs = KMemory::map_mmio(PhysAddr::new(phys), 1);
    IOAPIC_REGS.init(regs);
}

pub fn ioapic_read(reg: u8) -> u32 {
    let base = *IOAPIC_REGS.get();
    unsafe {
        base.as_mut_ptr::<u32>().write_volatile(reg as u32);
        (base + IOAPIC_DATA_REG).as_ptr::<u32>().read_volatile()
    }
}

fn ioapic_write(reg: u8, val: u32) {
    let base = *IOAPIC_REGS.get();
    unsafe {
        base.as_mut_ptr::<u32>().write_volatile(reg as u32);
        (base + IOAPIC_DATA_REG)
            .as_mut_ptr::<u32>()
            .write_volatile(val);
    }
}

/// Map a GSI to an I/O APIC redirection entry index.
fn gsi_to_entry(gsi: u8) -> u8 {
    let gsi_base = *apic_internal::IO_APIC_GSI_BASE.get();
    (gsi as u32).checked_sub(gsi_base).unwrap_or(gsi_base) as u8
}

pub fn redirect(gsi: u8, vector: u8, cpu: u8, flags: u16) {
    let entry = gsi_to_entry(gsi);
    let reg_low = 0x10 + 2 * entry;
    let reg_high = 0x10 + 2 * entry + 1;
    let polarity = ((flags >> 1) & 0x1) as u32;
    let trigger = ((flags >> 3) & 0x1) as u32;
    let old_low = ioapic_read(reg_low);
    let new_low = (old_low & !(0xFF | (1 << 13) | (1 << 15) | (1 << 11)))
        | vector as u32
        | (polarity << 13)
        | (trigger << 15);

    ioapic_write(reg_low, new_low);
    ioapic_write(reg_high, (cpu as u32) << 24);
}

pub fn unmask(gsi: u8) {
    let entry = gsi_to_entry(gsi);
    let reg_low = 0x10 + 2 * entry;
    let val = ioapic_read(reg_low);
    ioapic_write(reg_low, val & !(1 << 16));
}
