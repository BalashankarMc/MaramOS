//! The ACPI module.
//!
//! Manages, initializes and provides handles for devices like the HPET
//! and XSDT entries like the MCFG.
//!  
//! # Dependencies: Memory Management
//!
//! # Safety: Performs pointer arithmetic and MMIO. Relatively unlikely to fail.

mod apic;
pub(crate) use apic::lapic::{ICR_HIGH_REG, ICR_LOW_REG, X2APIC_ICR_MSR, is_x2apic};
mod fadt;
mod hpet;
mod mcfg;

use crate::{
    boot::requests::RSDP_DATA, descriptors::interrupts::HardwareInterrupts, library::Time, memory::{KMemory, PAGE_SIZE}
};

use alloc::vec::Vec;
use x86_64::{PhysAddr, VirtAddr};

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct GenericAddress {
    address_space: u8,
    bit_width: u8,
    bit_offset: u8,
    _reserved: u8,
    address: u64,
}

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
    fn revision(&self) -> u8 {
        unsafe { core::ptr::addr_of!(self.revision).read_unaligned() }
    }

    fn verify_integrity(&self) -> bool {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                self as *const Self as *const u8,
                core::mem::size_of::<RSDPHeader>(),
            )
        };
        bytes.iter().fold(0_u8, |acc, &b| acc.wrapping_add(b)) == 0
    }

    fn xsdt_address(&self) -> VirtAddr {
        let addr = unsafe { core::ptr::addr_of!(self.xsdt_address).read_unaligned() };
        let offset = addr & 0xFFF;
        let base = KMemory::map_mmio(PhysAddr::new(addr), 1);
        let length = unsafe { (base + offset).as_ptr::<u32>().add(1).read_volatile() };
        let extra = (length.saturating_sub(1) as usize).div_ceil(PAGE_SIZE);
        if extra > 0 {
            let _ = KMemory::map_mmio(PhysAddr::new(addr + 4096), extra);
        }
        base + offset
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct SDTHeader {
    signature: u32,
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
    fn signature(&self) -> XSDTSignature {
        unsafe { XSDTSignature::from(core::ptr::addr_of!(self.signature).read_unaligned()) }
    }
}

fn verify_checksum(table: VirtAddr) -> bool {
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
    fn from(val: u32) -> Self {
        match val {
            HPET_SIG => Self::Hpet,
            APIC_SIG => Self::Apic,
            MCFG_SIG => Self::Mcfg,
            FADT_SIG => Self::Fadt,
            _ => Self::Unknown,
        }
    }
}

/// Parse the RSDP (Root System Description Pointer) and XSDT, then initialise
/// known ACPI tables: HPET, APIC (I/O APIC + Local APIC), FADT (reset/reboot),
/// and MCFG (PCIe ECAM base).
///
/// Note: `RSDP_DATA.address` is used directly as a virtual address because the
/// Limine bootloader maps the RSDP page into the higher-half address space.
pub fn init() {
    let rsdp_virt = RSDP_DATA.address as u64;

    let rsdp = unsafe { (rsdp_virt as *const RSDPHeader).read_unaligned() };
    assert!(rsdp.verify_integrity());
    assert_eq!(rsdp.revision(), 2, "Unsupported RSDP revision");

    let xsdt_virt = rsdp.xsdt_address();
    let xsdt_length = unsafe { xsdt_virt.as_ptr::<u32>().add(1).read_volatile() };
    assert!(verify_checksum(xsdt_virt));

    let xsdt_pages = (xsdt_length as usize).div_ceil(PAGE_SIZE);

    let entry_count = (xsdt_length as usize - 36) / 8;
    let entries = (xsdt_virt + 36).as_ptr::<u64>();

    for i in 0..entry_count {
        let table_phys = unsafe { entries.add(i).read_unaligned() };
        let table_page_base = KMemory::map_mmio(PhysAddr::new(table_phys), 1);
        let table_virt = table_page_base + (table_phys & 0xFFF);

        let header = unsafe { table_virt.as_ptr::<SDTHeader>().read_unaligned() };

        let table_pages =
            (header.length as usize + (table_phys & 0xFFF) as usize).div_ceil(PAGE_SIZE);
        if table_pages > 1 {
            let next_phys = (table_phys & !0xFFF) + 4096;
            let _ = KMemory::map_mmio(PhysAddr::new(next_phys), table_pages - 1);
        }

        if !verify_checksum(table_virt) {
            KMemory::unmap_mmio(table_page_base, table_pages);
            continue;
        }

        match header.signature() {
            XSDTSignature::Apic => apic::init(table_virt),
            XSDTSignature::Hpet => hpet::init(table_virt),
            XSDTSignature::Mcfg => mcfg::init(table_virt),
            XSDTSignature::Fadt => fadt::init(table_virt),
            XSDTSignature::Unknown => {}
        }

        KMemory::unmap_mmio(table_page_base, table_pages);
    }

    KMemory::unmap_mmio(xsdt_virt, xsdt_pages);
}

/// Initialize the Local APICs
pub fn lapic_init() {
    apic::lapic::init_ap();
}

/// Sends an EOI signal to the Local APICs
pub fn lapic_eoi() {
    apic::lapic::eoi();
}

/// Get the address of the LAPIC registers
pub fn lapic_regs() -> VirtAddr {
    *apic::lapic::LAPIC_REGS.get()
}

pub fn trigger_interrupt(int: HardwareInterrupts) {
    apic::lapic::send_self_ipi(int.as_u8());
}

/// Reads the MCFGEntries and returns a Vec<MCFGEntry>
pub fn mcfg_entries() -> Vec<mcfg::MCFGEntry> {
    mcfg::MCFG_ENTRIES
        .try_get()
        .map(|v| v.as_slice())
        .unwrap_or(&[])
        .to_vec()
}

/// Get the Local APIC's ID
pub fn lapic_id() -> u32 {
    apic::lapic::id()
}

/// Get the total number of nanoseconds from boot
pub fn passed_nanos() -> u64 {
    hpet::passed_nanos()
}

pub fn lapic_init_timer(vector: u8) {
    apic::lapic::init_timer(vector);
}

pub fn lapic_init_timer_ap() {
    apic::lapic::init_timer_ap();
}

/// Triggers a system reboot
#[allow(dead_code)]
pub fn reboot() -> ! {
    fadt::reboot()
}

pub fn busy_wait(period: Time) {
    let deadline = period.to_nanos() + passed_nanos();
    while deadline > passed_nanos() { core::hint::spin_loop() }
}

const HPET_SIG: u32 = u32::from_ne_bytes(*b"HPET");
const APIC_SIG: u32 = u32::from_ne_bytes(*b"APIC");
const MCFG_SIG: u32 = u32::from_ne_bytes(*b"MCFG");
const FADT_SIG: u32 = u32::from_ne_bytes(*b"FACP");
