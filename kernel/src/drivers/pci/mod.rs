//! PCI/PCIe bus driver.
//!
//! Enumerates the PCI Express hierarchy via the ECAM (Enhanced Configuration
//! Access Mechanism) described by ACPI MCFG tables.  Discovers all functions on
//! all buses reachable from the host root bridges, including buses behind
//! PCI-to-PCI bridges (Type 1 headers).  Provides a [`PCIDevice`] trait that
//! driver code uses to access config space, BARs, and capabilities.

mod config;
mod device;
pub mod msix;
pub mod msi;

use crate::library::LateInit;
use alloc::vec::Vec;

pub use device::{BarInfo, PCIFunction};
use x86_64::VirtAddr;

static DEVICES: LateInit<Vec<PCIFunction>> = LateInit::new();

/// Classification of PCI function by its base class / subclass code.
#[derive(PartialEq, Debug)]
pub enum DeviceType {
    Nvme,
    Ahci,
    Eth,
    Wifi,
    Vga,
    GPU3D,
    Hda,
    Unknown,
}

/// Abstract interface to a PCI function.
///
/// Provides access to config-space registers, BARs, capabilities, and
/// common configuration (bus master, MMIO enable, I/O enable).
#[allow(dead_code)]
pub trait PCIDevice {
    fn bus(&self) -> u8;
    fn device(&self) -> u8;
    fn function(&self) -> u8;
    fn vendor_id(&self) -> u16;
    fn device_type(&self) -> DeviceType;
    fn device_id(&self) -> u16;
    fn class(&self) -> u8;
    fn subclass(&self) -> u8;
    fn bar(&self, index: usize) -> Option<&BarInfo>;
    fn find_capability(&self, id: u8) -> Option<u8>;

    fn read_u32(&self, offset: u16) -> u32;
    fn write_u32(&self, offset: u16, value: u32);

    fn read_u16(&self, offset: u16) -> u16 {
        (self.read_u32(offset & !2) >> ((offset as u32 & 2) * 8)) as u16
    }

    fn read_u8(&self, offset: u16) -> u8 {
        (self.read_u32(offset & !3) >> ((offset as u32 & 3) * 8)) as u8
    }

    fn write_u16(&self, offset: u16, value: u16) {
        let shift = (offset as u32 & 2) * 8;
        let mask = 0xFFFFu32 << shift;
        let aligned = offset & !2;
        let old = self.read_u32(aligned);
        self.write_u32(aligned, (old & !mask) | ((value as u32) << shift));
    }

    fn write_u8(&self, offset: u16, value: u8) {
        let shift = (offset as u32 & 3) * 8;
        let mask = 0xFFu32 << shift;
        let aligned = offset & !3;
        let old = self.read_u32(aligned);
        self.write_u32(aligned, (old & !mask) | ((value as u32) << shift));
    }

    fn enable_bus_master(&self) {
        let mut cmd = self.read_u32(0x04);
        cmd |= 1 << 2;
        self.write_u32(0x04, cmd);
    }

    fn enable_mmio(&self) {
        let mut cmd = self.read_u32(0x04);
        cmd |= 1 << 1;
        self.write_u32(0x04, cmd);
    }

    fn enable_io(&self) {
        let mut cmd = self.read_u32(0x04);
        cmd |= 1 << 0;
        self.write_u32(0x04, cmd);
    }
}

/// Initialise the PCIe subsystem.
///
/// 1. Reads the ACPI MCFG table to obtain ECAM base addresses and bus ranges.
/// 2. Maps each segment's ECAM region into the kernel's MMIO address space.
/// 3. Scans all buses (including those discovered behind bridges).
pub fn init() {
    let entries = crate::acpi::mcfg_entries();
    let segments = entries
        .iter()
        .map(|e| config::Segment {
            base_address: e.base_address,
            start_bus: e.start_bus,
            end_bus: e.end_bus,
            virt_addr: VirtAddr::zero(),
        })
        .collect();
    config::init_segments(segments);

    let devices = device::scan_bus();
    DEVICES.init(devices);
}

/// Return all cached functions that satisfy the predicate `f`.
///
/// This is the primary mechanism drivers use to discover devices of interest
/// (e.g. all NVMe controllers).
pub fn find_devices(f: impl Fn(&dyn PCIDevice) -> bool) -> Vec<&'static PCIFunction> {
    DEVICES.try_get().map_or(Vec::new(), |devices| {
        devices.iter().filter(|d| f(*d as &dyn PCIDevice)).collect()
    })
}
