use alloc::vec::Vec;
use x86_64::{PhysAddr, VirtAddr};

use crate::{LateInit, errors::ACPIError, memory::phys_to_virt};

const RTE_BASE: usize = 0x10;

static IOAPIC_REGIONS: LateInit<Vec<VirtAddr>> = LateInit::new();

pub fn init() -> Result<(), ACPIError> {
    let ioapics = super::IOAPICS.try_get().ok_or(ACPIError::IOAPICNotInitialized)?;

    if ioapics.is_empty() { return Err(ACPIError::MADTNoIoApicFound) }

    let mut regions = Vec::new();

    for ioapic in ioapics {
        let region = phys_to_virt(PhysAddr::new(u64::from(ioapic.address)));
        regions.push(region);
    }

    IOAPIC_REGIONS.init(regions);

    Ok(())
}

/// Finds the IOAPIC owning `gsi` and returns it's index
fn ioapic_index(gsi: u32) -> Result<usize, ACPIError> {
    let ioapics = super::IOAPICS.try_get().ok_or(ACPIError::IOAPICNotInitialized)?;

    for (i, ioapic) in ioapics.iter().enumerate() {
        let next_base = ioapics.get(i + 1).map_or(u32::MAX, |a| a.gsi_base);
        if gsi >= ioapic.gsi_base && gsi < next_base { return Ok(i) }
    }

    Err(ACPIError::GSIUnderflow)
}

/// Convert a GSI to an RTE index
fn gsi_to_entry(gsi: u32) -> Result<u8, ACPIError> {
    let index = ioapic_index(gsi)?;
    let base = super::IOAPICS.try_get().ok_or(ACPIError::IOAPICNotInitialized)?[index].gsi_base;

    Ok(gsi.checked_sub(base).ok_or(ACPIError::GSIUnderflow)? as u8)
}

fn ioapic_read(index: usize, reg: u8) -> Result<u32, ACPIError> {
    let region = IOAPIC_REGIONS.try_get().ok_or(ACPIError::IOAPICNotInitialized)?[index];
    let ptr = region.as_mut_ptr::<u32>();

    unsafe {
        ptr.write_volatile(u32::from(reg));
        Ok(ptr.add(4).read_volatile())
    }
}

fn ioapic_write(index: usize, reg: u8, val: u32) -> Result<(), ACPIError> {
    let region = IOAPIC_REGIONS.try_get().ok_or(ACPIError::IOAPICNotInitialized)?[index];
    let ptr = region.as_mut_ptr::<u32>();

    unsafe {
        ptr.write_volatile(u32::from(reg));
        ptr.add(4).write_volatile(val);
    }

    Ok(())
}

/// Route `gsi` to `vector` on `cpu`, with polarity/trigger from `flags`.
///
/// # Errors
/// Returns `GSIUnderflow` if `gsi` is owned by no I/O APIC, or
/// `IOAPICNotInitialized` if the IOAPIC(s) have not been initialized.
pub fn redirect(gsi: u32, vector: u8, cpu: u8, flags: u16) -> Result<(), ACPIError> {
    let index = ioapic_index(gsi)?;
    let entry = gsi_to_entry(gsi)?;

    let polarity = u32::from((flags >> 1) & 1);
    let trigger = u32::from((flags >> 3) & 1);

    let reg_low = RTE_BASE as u8 + 2 * entry;
    let reg_high = RTE_BASE as u8 + 2 * entry + 1;

    let old_low = ioapic_read(index, reg_low)?;
    // 0b10101 << 11 sets Polarity, Dest mode and Trigger flags
    let new_low = (old_low & !(0xFF | (0b10101 << 11))) | u32::from(vector) | (polarity << 13) | (trigger << 15);

    ioapic_write(index, reg_low, new_low)?;
    ioapic_write(index, reg_high, u32::from(cpu) << 24)
}

/// Enable delivery of `gsi`
/// 
/// # Errors
/// Returns `GSIUnderflow` if `gsi` is owned by no I/O APIC, or
/// `IOAPICNotInitialized` if the IOAPIC(s) have not been initialized.
pub fn unmask(gsi: u32) -> Result<(), ACPIError> {
    let index = ioapic_index(gsi)?;
    let entry = gsi_to_entry(gsi)?;
    let reg_low = RTE_BASE as u8 + 2 * entry;

    let val = ioapic_read(index, reg_low)?;
    ioapic_write(index, reg_low, val & !(1 << 16))
}

/// Disable delivery of `gsi`
///
/// # Errors
/// Returns `GSIUnderflow` if `gsi` is owned by no I/O APIC, or
/// `IOAPICNotInitialized` if the IOAPIC(s) have not been initialized.
pub fn mask(gsi: u32) -> Result<(), ACPIError> {
    let index = ioapic_index(gsi)?;
    let entry = gsi_to_entry(gsi)?;
    let reg_low = RTE_BASE as u8 + 2 * entry;

    let val = ioapic_read(index, reg_low)?;
    ioapic_write(index, reg_low, val | (1 << 16))
}