//! FADT (Fixed ACPI Description Table) handling.
//!
//! Extracts the system reset register (I/O port or MMIO) from the FADT
//! and provides a [`reboot`] function that triggers a CPU reset.

use crate::{library::LateInit, memory::KMemory};

use super::GenericAddress;

use x86_64::instructions::port::Port;
use x86_64::{PhysAddr, VirtAddr};

const REV_OFFSET: u64 = 8;
const RESET_REG_OFFSET: u64 = 116;
const RESET_VAL_OFFSET: u64 = 128;

static RESET_ADDR_SPACE: LateInit<u8> = LateInit::new();
static RESET_ADDRESS: LateInit<u64> = LateInit::new();
static RESET_VALUE: LateInit<u8> = LateInit::new();

pub fn init(entry: VirtAddr) {
    let revision = unsafe { (entry + REV_OFFSET).as_ptr::<u8>().read_volatile() };
    if revision < 2 {
        warn!("FADT: revision {} too old for reset register", revision);
        return
    }

    let reg = unsafe { (entry + RESET_REG_OFFSET).as_ptr::<GenericAddress>().read_volatile() };

    if reg.address_space == 0 || reg.address == 0 {
        warn!("FADT: no reset register");
        return
    }

    let val = unsafe { (entry + RESET_VAL_OFFSET).as_ptr::<u8>().read_volatile() };
    RESET_ADDR_SPACE.init(reg.address_space);
    RESET_ADDRESS.init(reg.address);
    RESET_VALUE.init(val);
}

pub fn reboot() -> ! {
    let space = *RESET_ADDR_SPACE.get();
    let addr = *RESET_ADDRESS.get();
    let val = *RESET_VALUE.get();
    match space {
        1 => unsafe { Port::<u8>::new(addr as u16).write(val) },
        0 => {
            let ptr = KMemory::map_mmio(PhysAddr::new(addr), 1);
            unsafe { ptr.as_mut_ptr::<u8>().write_volatile(val) }
        }
        _ => warn!("FADT: unsupported reset address space {}", space),
    }
    loop {
        x86_64::instructions::hlt();
    }
}
