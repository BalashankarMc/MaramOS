//! HPET (High Precision Event Timer) driver.

use x86_64::{PhysAddr, VirtAddr};

use crate::{LateInit, errors::ACPIError, memory::phys_to_virt};

static HPET_REGS: LateInit<VirtAddr> = LateInit::new();
static HPET_PERIOD_FS: LateInit<u64> = LateInit::new();

/// Initialize the HPET
/// 
/// # Safety
/// `hpet_entry` must point to a valid HPET Entry
pub unsafe fn init(hpet_entry: VirtAddr) -> Result<(), ACPIError> {
    let address = unsafe { (hpet_entry + 44).as_ptr::<u64>().read_unaligned() };

    let regs = phys_to_virt(PhysAddr::new(address));
    let gcap = unsafe { regs.as_ptr::<u64>().read_volatile() };
    let period = gcap >> 32;

    if period == 0 { return Err(ACPIError::HPETPeriodZero) }

    HPET_REGS.init(regs);
    HPET_PERIOD_FS.init(period);

    let conf = unsafe { (regs + 0x10).as_ptr::<u64>().read_volatile() };
    unsafe { core::ptr::write_volatile(regs.as_mut_ptr::<u64>().add(2), conf | 1) }
    
    Ok(())
}

pub fn passed_nanos() -> u64 {
    let regs = *HPET_REGS;
    let period = *HPET_PERIOD_FS;

    let raw = unsafe { (regs + 0xF0).as_ptr::<u64>().read_volatile() };

    (raw * period) / 10_u64.pow(6)
}