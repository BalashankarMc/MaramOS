//! PCI function representation, BAR/capability parsing, and bus enumeration.

use alloc::vec::Vec;

use crate::drivers::pci::{DeviceType, PciID, config::{Segment, find_entry}};

/// Description of a single Base Address Register decoded from config space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BarInfo {
    pub address: u64,
    pub size: u64,
    pub is_mmio: bool,
    pub is_64_bit: bool
}

impl BarInfo {
    /// A sentinel value representing an absent or invalid BAR.
    const fn empty() -> Self {
        Self {
            address: 0,
            size: 0,
            is_mmio: false,
            is_64_bit: false
        }
    }
}

/// A single capability header record from the linked list at offset 0x34.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapHeader {
    id: u8,
    offset: u8,
}

/// A discovered PCI function with its identity, BARs, and capabilities.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PCIFunction {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision: u8,
    pub class: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub(crate) bars: [BarInfo; 6],
    pub(crate) caps: Vec<CapHeader>,
}

impl PCIFunction {
    pub fn new(bus: u8, device: u8, function: u8) -> Option<Self> {
        let segment = super::config::find_entry(bus)?;

        // Vendor and device IDs
        let vendor_data = super::config::read32(segment, bus, device, function, 0)?;
        let vendor_id = vendor_data as u16;
        if vendor_id == 0xFFFF { return None }

        let device_id = (vendor_data >> 16) as u16;

        // Revision and class data
        let class_data = super::config::read32(segment, bus, device, function, 8)?;
        let revision = (class_data & 0xFF) as u8;
        let prog_if = ((class_data >> 8) & 0xFF) as u8;
        let subclass = ((class_data >> 16) & 0xFF) as u8;
        let class = ((class_data >> 24) & 0xFF) as u8;

        // BARs and capabilities
        let bars = parse_bars(segment, bus, device, function)?;
        let caps = parse_caps(segment, bus, device, function).unwrap_or_default();

        Some(Self {
            bus, device, function, vendor_id,
            device_id, revision, class, subclass,
            prog_if, bars, caps
        })
    }

    pub const fn bus(&self) -> u8 { self.bus }
    pub const fn device(&self) -> u8 { self.device }
    pub const fn function(&self) -> u8 { self.function }
    pub const fn vendor_id(&self) -> u16 { self.vendor_id }
    pub const fn class(&self) -> u8 { self.class }
    pub const fn subclass(&self) -> u8 { self.subclass }
    
    pub const fn id(&self) -> PciID {
        PciID { device: self.device, bus: self.bus, function: self.function }
    }

    pub fn read_u32(&self, offset: u16) -> Option<u32> {
        let entry = find_entry(self.bus)?;
        super::config::read32(entry, self.bus, self.device, self.function, offset)
    }

    pub fn write_u32(&self, offset: u16, val: u32) -> Option<()> {
        let entry = find_entry(self.bus)?;
        super::config::write32(entry, self.bus, self.device, self.function, offset, val)
    }

    pub fn read_u16(&self, offset: u16) -> Option<u16> {
        let entry = find_entry(self.bus)?;
        super::config::read16(entry, self.bus, self.device, self.function, offset)
    }

    pub fn write_u16(&self, offset: u16, val: u16) -> Option<()> {
        let entry = find_entry(self.bus)?;
        super::config::write16(entry, self.bus, self.device, self.function, offset, val)
    }

    pub fn read_u8(&self, offset: u16) -> Option<u8> {
        let entry = find_entry(self.bus)?;
        super::config::read8(entry, self.bus, self.device, self.function, offset)
    }

    pub fn write_u8(&self, offset: u16, val: u8) -> Option<()> {
        let entry = find_entry(self.bus)?;
        super::config::write8(entry, self.bus, self.device, self.function, offset, val)
    }

    pub fn enable_mmio(&self) -> Option<()> {
        let cmd = self.read_u32(4)? | 2;
        self.write_u32(4, cmd)
    }

    pub fn enable_bus_master(&self) -> Option<()> {
        let cmd = self.read_u32(4)? | 4;
        self.write_u32(4, cmd)
    }

    pub fn bar(&self, index: usize) -> Option<BarInfo> {
        self.bars
            .get(index)
            .copied()
            .filter(|b| b != &BarInfo::empty())
    }

    pub fn find_capability(&self, id: u8) -> Option<u8> {
        self.caps
            .iter()
            .find(|c| c.id == id )
            .map(|c| c.offset)
    }

    pub const fn device_type(&self) -> DeviceType {
        match self.class {
            0x01 => match self.subclass {
                0x06 => DeviceType::Ahci,
                0x08 => DeviceType::Nvme,
                _ => DeviceType::Unknown,
            },
            0x02 => match self.subclass {
                0x00 => DeviceType::Eth,
                0x80 => DeviceType::Wifi,
                _ => DeviceType::Unknown,
            },
            0x03 => match self.subclass {
                0x00 => DeviceType::Vga,
                0x02 => DeviceType::GPU3D,
                _ => DeviceType::Unknown,
            },
            0x04 => match self.subclass {
                0x03 => DeviceType::Hda,
                _ => DeviceType::Unknown,
            },
            _ => DeviceType::Unknown,
        }
    }
}

impl Drop for PCIFunction {
    fn drop(&mut self) {
        super::DEVICES.lock().push(self.clone());
    }
}

fn parse_bars(segment: &Segment, bus: u8, device: u8, function: u8) -> Option<[BarInfo; 6]> {
    let mut bars = [BarInfo::empty(); 6];

    let read = |offset| { super::config::read32(segment, bus, device, function, offset) };
    let write = |offset, val| { super::config::write32(segment, bus, device, function, offset, val) };

    let mut iter = bars.iter_mut();
    
    while let Some(bar) = iter.next() {
        let i = 5 - iter.len();
        let offset = 0x10 + i as u16 * 4;
        let original = read(offset)?;

        if original == 0 { continue } // Absent BAR

        let is_mmio = original & 1 == 0;
        let bar_type = (original >> 1) & 3;

        match (bar_type, is_mmio) {
            (0, true) => { // 32 bit MMIO
                write(offset, u32::MAX)?;
                
                let mask = read(offset)?;
                write(offset, original)?;

                let x = 0xFFFF_FFF0;

                *bar = BarInfo {
                    address: u64::from(original & x),
                    size: u64::from(!(mask & x) + 1),
                    is_mmio,
                    is_64_bit: false
                }
            }

            (2, true) => { // 64 bit MMIO

                if i >= 5 { continue } // Last BAR cannot be 64 bit

                let original_high = read(offset + 4)?;
                write(offset, u32::MAX)?;
                write(offset + 4, u32::MAX)?;

                let mask_high = read(offset + 4)?;
                let mask_low = read(offset)?;

                write(offset + 4, original_high)?;
                write(offset, original)?;

                let mask = (u64::from(mask_high) << 32) | u64::from(mask_low);
                let addr = (u64::from(original_high) << 32) | u64::from(original);
                let size = !(mask & !0xF) + 1;

                *bar = BarInfo {
                    address: addr & !0xF,
                    size,
                    is_mmio,
                    is_64_bit: true
                };

                // Skip the next BAR
                iter.next();
            }

            (_, false) => { // IO BAR
                write(offset, u32::MAX)?;
                let mask = read(offset)?;
                write(offset, original)?;

                *bar = BarInfo {
                    address: u64::from(original & 0xFFFC),
                    size: u64::from(!(mask & 0xFFFC) + 1),
                    is_mmio,
                    is_64_bit: false
                }
            }

            (_, _) => { } // Invalid, continue
        }
    }

    Some(bars)
}

fn parse_caps(segment: &Segment, bus: u8, device: u8, function: u8) -> Option<Vec<CapHeader>> {
    let read = |offset| { super::config::read8(segment, bus, device, function, offset) };

    let caps_ptr = read(0x34)?;
    if caps_ptr == 0 { return None }

    let mut caps = Vec::new();
    let mut offset = caps_ptr;

    for _ in 0..48 {
        let id = read(u16::from(offset))?;
        let next = read(u16::from(offset) + 1)?;
        caps.push(CapHeader { id, offset });
        if next == 0 || (next < 0x40 || next == offset) { break }

        offset = next;
    }

    Some(caps)
}
