//! Manages the ACPI (Advanced Configuration and Power Interface) tables
//! and provides handles for some hardware devices like the HPET and LAPIC Timers

use alloc::vec::Vec;
use x86_64::{PhysAddr, VirtAddr};

use crate::{
    KernelError,
    KernelResult,
    RSDP_RESPONSE,
    log_warn,
    descriptors::HardwareInterrupts,
    errors::ACPIError,
    memory::phys_to_virt};

mod apic;
mod hpet;
mod mcfg;

pub use mcfg::MCFGEntry;
pub use hpet::passed_nanos;

/// Initialize the Local APICs
#[allow(unused_imports)]
pub use apic::lapic::init_ap as lapic_init;

/// Sends an EOI signal to the Local APICs
pub use apic::lapic::eoi as lapic_eoi;

pub use apic::lapic::id as lapic_id;
pub use apic::lapic::init_timer as init_lapic_timer;
pub use apic::ioapic::redirect as redirect_ioapic;
pub use apic::ioapic::unmask as unmask_ioapic;

pub fn trigger_interrupt(int: HardwareInterrupts) {
    apic::lapic::send_self_ipi(int.as_u8());
}

/// Reads the `MCFGEntries` and returns a `Vec<MCFGEntry>`
pub fn mcfg_entries() -> Vec<mcfg::MCFGEntry> {
    mcfg::MCFG_ENTRIES
        .try_get()
        .unwrap_or(&Vec::<mcfg::MCFGEntry>::new())
        .clone()
}

const HPET_SIG: u32 = u32::from_ne_bytes(*b"HPET");
const APIC_SIG: u32 = u32::from_ne_bytes(*b"APIC");
const MCFG_SIG: u32 = u32::from_ne_bytes(*b"MCFG");
const FADT_SIG: u32 = u32::from_ne_bytes(*b"FACP");

#[repr(C, packed)]
struct RSDPHeader {
    signature: u64,
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32, // Unused (RSDT not supported)
    length: u32,
    xsdt_address: u64,
    ext_checksum: u8,
    reserved: [u8; 3],
}

impl RSDPHeader {
    fn verify_integrity(&self) -> bool {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                core::ptr::from_ref(self).cast::<u8>(),
                core::mem::size_of::<Self>(),
            )
        };
        bytes.iter().fold(0_u8, |acc, &b| acc.wrapping_add(b)) == 0
    }

    fn xsdt_address(&self) -> VirtAddr {
        phys_to_virt(PhysAddr::new(self.xsdt_address))
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct SDTHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

impl SDTHeader {
    const fn signature(&self) -> XSDTSignature {
        XSDTSignature::from(u32::from_le_bytes(self.signature))
    }
}

fn verify_checksum(table: VirtAddr) -> bool {
    // Safety: Data is valid, as it is part of the ACPI Tables
    let length = unsafe { table.as_ptr::<u32>().add(1).read_volatile() } as usize;
    let bytes = unsafe { core::slice::from_raw_parts(table.as_ptr::<u8>(), length) };
    bytes.iter().fold(0_u8, |acc, &b| acc.wrapping_add(b)) == 0
}

#[derive(Clone, Copy)]
enum XSDTSignature {
    Hpet,
    Apic,
    Mcfg,
    Fadt,
    Unknown,
}

impl XSDTSignature {
    const fn from(val: u32) -> Self {
        match val {
            HPET_SIG => Self::Hpet,
            APIC_SIG => Self::Apic,
            MCFG_SIG => Self::Mcfg,
            FADT_SIG => Self::Fadt,
            _ => Self::Unknown,
        }
    }
}

pub fn init() -> KernelResult<()> {
    // Safety: We know that this address was initialized by Limine
    let rsdp = unsafe { RSDP_RESPONSE.address.cast::<RSDPHeader>().read_unaligned() };

    if !rsdp.verify_integrity() { return Err(KernelError::ACPIError(ACPIError::RSDPIntegrityFailed)) }
    if rsdp.revision != 2 { return Err(KernelError::ACPIError(ACPIError::RSDPUnsupportedRevision)) }

    let xsdt_addr = rsdp.xsdt_address();
    if !verify_checksum(xsdt_addr) { return Err(KernelError::ACPIError(ACPIError::XSDTChecksumFailed)) }

    // Safety: We know that this was initialized as part of the ACPI Tables
    let xsdt = unsafe { xsdt_addr.as_ptr::<SDTHeader>().read() };

    let entry_count = (xsdt.length as usize - size_of::<SDTHeader>()) / size_of::<u64>();
    let entry_start = (xsdt_addr + size_of::<SDTHeader>() as u64).as_ptr::<u64>();

    for i in 0..entry_count {
        // Safety: Same as before
        let info_phys = unsafe { entry_start.add(i).read_unaligned() };
        let info_virt = phys_to_virt(PhysAddr::new(info_phys));

        if !verify_checksum(info_virt) {
            log_warn!("Skipping ACPI Table - Invalid Checksum!");
            continue
        }

        // Safety: Same as before
        let header = unsafe { info_virt.as_ptr::<SDTHeader>().read_unaligned() };

        // Safety: We know that `info_virt` is valid. Safe.
        match header.signature() {
            XSDTSignature::Apic => unsafe { apic::init(info_virt)? },
            XSDTSignature::Hpet => unsafe { hpet::init(info_virt)? },
            XSDTSignature::Mcfg => unsafe { mcfg::init(info_virt) },
            XSDTSignature::Unknown => log_warn!("Unknown ACPI Table found - Skipping"),
            XSDTSignature::Fadt => {}
        }
    }

    Ok(())
}