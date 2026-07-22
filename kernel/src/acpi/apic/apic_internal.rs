//! MADT I/O APIC entry parser.
//!
//! Walks the Multiple APIC Description Table to extract I/O APIC base
//! addresses and Global System Interrupt base numbers, storing them in
//! static [`LateInit`] cells for use by the I/O APIC driver.

use crate::library::LateInit;
use x86_64::VirtAddr;

pub unsafe fn init(apic_header: VirtAddr) {
    let length = unsafe { apic_header.as_ptr::<u32>().add(1).read_unaligned() };
    let mut offset: u64 = 44;

    while offset < length as u64 {
        let addr = apic_header + offset;

        let entry_type = unsafe { addr.as_ptr::<u8>().read_unaligned() };
        let entry_length = unsafe { (addr + 1).as_ptr::<u8>().read_unaligned() };

        if entry_type == 1 {
            let address = unsafe { addr.as_ptr::<u32>().add(1).read_unaligned() };
            let gsi_base = unsafe { addr.as_ptr::<u32>().add(2).read_unaligned() };

            IO_APIC_BASE.init(address);
            IO_APIC_GSI_BASE.init(gsi_base);
        }

        offset += entry_length as u64;
    }
}

pub static IO_APIC_BASE: LateInit<u32> = LateInit::new();
pub static IO_APIC_GSI_BASE: LateInit<u32> = LateInit::new();
