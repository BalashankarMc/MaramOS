//! HPET (High Precision Event Timer) driver.
//!
//! Maps the HPET MMIO registers, reads the capabilityid register for the
//! counter period, and provides [`passed_nanos`] for elapsed-time queries.

use crate::library::LateInit;
use crate::memory::KMemory;

use x86_64::{PhysAddr, VirtAddr};

static HPET_REGS: LateInit<VirtAddr> = LateInit::new();
static HPET_PERIOD_FS: LateInit<u64> = LateInit::new();

pub fn init(hpet_entry: VirtAddr) {
    let address = unsafe { (hpet_entry + 44).as_ptr::<u64>().read_unaligned() };
    let regs = KMemory::map_mmio(PhysAddr::new(address), 1);
    let gcap = unsafe { regs.as_ptr::<u64>().read_volatile() };
    let period = gcap >> 32;

    HPET_REGS.init(regs);
    HPET_PERIOD_FS.init(period);

    let conf = unsafe { (regs + 0x10).as_ptr::<u64>().read_volatile() };
    unsafe {
        core::ptr::write_volatile(regs.as_mut_ptr::<u64>().add(2), conf | 1);
    }
}

pub fn passed_nanos() -> u64 {
    let regs = *HPET_REGS.get();
    let period = *HPET_PERIOD_FS.get();

    let count = unsafe { (regs + 0xF0).as_ptr::<u64>().read_volatile() };
    (count * period) / 10_u64.pow(6)
}
