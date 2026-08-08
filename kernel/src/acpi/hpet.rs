//! HPET (High Precision Event Timer) driver.

use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::{PhysAddr, VirtAddr};

use crate::{LateInit, errors::ACPIError, memory::phys_to_virt};

static HPET_REGS: LateInit<VirtAddr> = LateInit::new();
static HPET_PERIOD_FS: LateInit<u64> = LateInit::new();
static HPET_ACCUMULATOR: AtomicU64 = AtomicU64::new(0);
static HPET_LAST_RAW: AtomicU64 = AtomicU64::new(0);

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

    let raw = unsafe { u64::from((regs + 0xF0).as_ptr::<u32>().read_volatile()) };

    let last = HPET_LAST_RAW.load(Ordering::Relaxed);
    if raw < last {
        HPET_ACCUMULATOR.fetch_add(1u64 << 32, Ordering::Relaxed);
    }
    HPET_LAST_RAW.store(raw, Ordering::Relaxed);

    let count = HPET_ACCUMULATOR.load(Ordering::Relaxed) | raw;
    (count * period) / 10_u64.pow(6)
}