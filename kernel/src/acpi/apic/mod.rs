//! APIC (Advanced Programmable Interrupt Contoller) management module

use alloc::vec::Vec;
use x86_64::VirtAddr;

use crate::{LateInit, acpi::SDTHeader, errors::ACPIError};

pub mod ioapic;
pub mod lapic;

pub struct IOApic {
    pub apic_id: u8,
    pub address: u32,
    pub gsi_base: u32,
}

pub struct InterruptOverride {
    pub source: u8,
    pub gsi: u32,
    pub flags: u16,
}


pub static IOAPICS: LateInit<Vec<IOApic>> = LateInit::new();
pub static OVERRIDES: LateInit<Vec<InterruptOverride>> = LateInit::new();

/// # Safety
/// Caller must ensure `apic_header` points to a valid MADT
unsafe fn init_apic(apic_header: VirtAddr) -> Result<(), ACPIError> {
    // Note: All unsafe blocks in this function fall under the function safety note
    // SDT length field at byte 4
    let length = unsafe { (apic_header + 4).as_ptr::<u32>().read_unaligned() };

    // MADT entries start after SDTHeader (36) + MADT header (8)
    let mut offset = size_of::<SDTHeader>() + size_of::<u64>();

    let mut ioapic_vec = Vec::new();
    let mut override_vec = Vec::new();

    while offset < length as usize {
        let addr = apic_header + offset as u64;

        let entry_type = unsafe { (addr).as_ptr::<u8>().read_volatile() };
        let entry_length = unsafe { (addr + 1).as_ptr::<u8>().read_volatile() };

        if entry_length == 0 {
            return Err(ACPIError::MADTEntryLengthZero)
        }

        match entry_type {
            0x01 => {  // I/O APIC
                ioapic_vec.push(IOApic {
                    apic_id: unsafe { (addr + 2).as_ptr::<u8>().read_unaligned() },
                    address: unsafe { (addr + 4).as_ptr::<u32>().read_unaligned() },
                    gsi_base:  unsafe { (addr + 8).as_ptr::<u32>().read_unaligned() },
                });
            }

            0x02 => {  // Interrupt Source Override
                override_vec.push(InterruptOverride {
                    source: unsafe { (addr + 3).as_ptr::<u8>().read_unaligned() },
                    gsi: unsafe { (addr + 4).as_ptr::<u32>().read_unaligned() },
                    flags: unsafe { (addr + 8).as_ptr::<u16>().read_unaligned() },
                });
            }

            _ => {}
        }

        offset += usize::from(entry_length);
    }

    IOAPICS.init(ioapic_vec);
    OVERRIDES.init(override_vec);

    Ok(())
}

pub fn init(apic_header: VirtAddr) -> Result<(), ACPIError> {
    unsafe { init_apic(apic_header)?; }
    ioapic::init()?;
    lapic::init()
}