//! PCI function representation, BAR/capability parsing, and bus enumeration.
//!
//! Provides [`PCIFunction`] — the concrete type behind the [`PCIDevice`] trait —
//! and the [`scan_bus`] entry point that performs a full hierarchical
//! enumeration of the PCIe tree.

use super::config;
use super::{DeviceType, PCIDevice};

use alloc::vec::Vec;
use x86_64::VirtAddr;

/// Description of a single Base Address Register decoded from config space.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // Keep is_mmio and is_64bit
pub struct BarInfo {
    /// Physical address this BAR decodes.
    pub address: u64,
    /// Size of the region (bytes); 0 implies the BAR is absent.
    pub size: u64,
    /// `true` for MMIO BARs, `false` for I/O-port BARs.
    pub is_mmio: bool,
    /// `true` for 64-bit MMIO BARs (consumes two consecutive slots).
    pub is_64bit: bool,
}

impl BarInfo {
    /// A sentinel value representing an absent or invalid BAR.
    pub(crate) const fn empty() -> Self {
        BarInfo {
            address: 0,
            size: 0,
            is_mmio: false,
            is_64bit: false,
        }
    }
}

/// A single capability header record from the linked list at offset 0x34.
#[derive(Debug)]
pub(crate) struct CapHeader {
    id: u8,
    offset: u8,
}

/// A discovered PCI function with its identity, BARs, and capabilities.
///
/// This is the concrete type returned by [`scan_bus`] and stored in the
/// global device list.  All config space reads go through [`config::ecam_virt`].
#[derive(Debug)]
#[allow(dead_code)] // Keep revision and prog_if
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

impl PCIDevice for PCIFunction {
    fn bus(&self) -> u8 { self.bus }
    fn device(&self) -> u8 { self.device }
    fn function(&self) -> u8 { self.function }
    fn vendor_id(&self) -> u16 { self.vendor_id }
    fn device_type(&self) -> DeviceType { self.get_type() }
    fn device_id(&self) -> u16 { self.device_id }
    fn class(&self) -> u8 { self.class }
    fn subclass(&self) -> u8 { self.subclass }

    fn bar(&self, index: usize) -> Option<&BarInfo> {
        let b = &self.bars[index];
        if b.size == 0 { None } else { Some(b) }
    }

    fn find_capability(&self, id: u8) -> Option<u8> {
        self.caps.iter().find(|c| c.id == id).map(|c| c.offset)
    }

    fn read_u32(&self, offset: u16) -> u32 {
        unsafe { config::read32(self.bus, self.device, self.function, offset) }
    }

    fn write_u32(&self, offset: u16, value: u32) {
        unsafe {
            config::write32(self.bus, self.device, self.function, offset, value);
        }
    }
}

impl PCIFunction {
    /// Construct a new `PCIFunction` by reading the config space of the
    /// given BDF.  Parses the six standard BARs and the capability chain.
    pub fn new(bus: u8, device: u8, function: u8) -> Self {
        let virt = config::ecam_virt(bus, device, function);

        // Vendor/device at offset 0x00.
        let vendor_data = unsafe { virt.as_ptr::<u32>().read_volatile() };
        let vendor_id = (vendor_data & 0xFFFF) as u16;
        let device_id = (vendor_data >> 16) as u16;

        // Revision / class at offset 0x08.
        let class_data = unsafe { virt.as_ptr::<u32>().add(2).read_volatile() };
        let revision = (class_data & 0xFF) as u8;
        let prog_if = ((class_data >> 8) & 0xFF) as u8;
        let subclass = ((class_data >> 16) & 0xFF) as u8;
        let class = ((class_data >> 24) & 0xFF) as u8;

        let bars = parse_bars(virt);
        let caps = parse_caps(virt);

        Self {
            bus,
            device,
            function,
            vendor_id,
            device_id,
            revision,
            class,
            subclass,
            prog_if,
            bars,
            caps,
        }
    }

    /// Classify the function by its (class, subclass) pair.
    pub fn get_type(&self) -> DeviceType {
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

/// Decode the six Type-0 BARs starting at offset 0x10.
///
/// For each BAR:
///   - bit 0 == 0 → MMIO BAR.  Bits 2:1 encode width (0 = 32-bit, 2 = 64-bit).
///   - bit 0 == 1 → I/O BAR.
///
/// Sizing is discovered by writing all-ones and reading back the
/// write-mask (standard PCI sizing protocol).
fn parse_bars(virt: VirtAddr) -> [BarInfo; 6] {
    let mut bars = [BarInfo::empty(); 6];
    let ptr = virt.as_mut_ptr::<u32>();

    let mut i = 0;
    while i < 6 {
        let offset = 0x10 + i * 4;
        let orig = unsafe { ptr.add(offset / 4).read_volatile() };

        if orig == 0 {
            i += 1;
            continue;
        }

        if orig & 1 == 0 {
            let bar_type = (orig >> 1) & 0x3;

            if bar_type == 0 {
                // 32-bit MMIO.
                unsafe {
                    ptr.add(offset / 4).write_volatile(0xFFFF_FFFF);
                }
                let mask = unsafe { ptr.add(offset / 4).read_volatile() };
                unsafe {
                    ptr.add(offset / 4).write_volatile(orig);
                }
                bars[i] = BarInfo {
                    address: (orig & 0xFFFF_FFF0) as u64,
                    size: (!(mask & 0xFFFF_FFF0) + 1) as u64,
                    is_mmio: true,
                    is_64bit: false,
                };
            } else if bar_type == 2 && i < 5 {
                // 64-bit MMIO — consumes this BAR and the next.
                let orig_hi = unsafe { ptr.add(offset / 4 + 1).read_volatile() };
                unsafe {
                    ptr.add(offset / 4).write_volatile(0xFFFF_FFFF);
                }
                unsafe {
                    ptr.add(offset / 4 + 1).write_volatile(0xFFFF_FFFF);
                }
                let mask_lo = unsafe { ptr.add(offset / 4).read_volatile() };
                let mask_hi = unsafe { ptr.add(offset / 4 + 1).read_volatile() };
                unsafe {
                    ptr.add(offset / 4).write_volatile(orig);
                }
                unsafe {
                    ptr.add(offset / 4 + 1).write_volatile(orig_hi);
                }

                let addr = (orig as u64) | ((orig_hi as u64) << 32);
                let mask = (mask_lo as u64) | ((mask_hi as u64) << 32);
                let size = !(mask & !0xF) + 1;
                bars[i] = BarInfo {
                    address: addr & !0xF,
                    size,
                    is_mmio: true,
                    is_64bit: true,
                };
                bars[i + 1] = BarInfo::empty();
                i += 1;
            }
        } else {
            // I/O BAR.
            unsafe {
                ptr.add(offset / 4).write_volatile(0xFFFF_FFFF);
            }
            let mask = unsafe { ptr.add(offset / 4).read_volatile() };
            unsafe {
                ptr.add(offset / 4).write_volatile(orig);
            }
            bars[i] = BarInfo {
                address: (orig & 0xFFFC) as u64,
                size: (!(mask & 0xFFFC) + 1) as u64,
                is_mmio: false,
                is_64bit: false,
            };
        }

        i += 1;
    }

    bars
}

/// Walk the capabilities linked list starting at offset 0x34.
///
/// Each list element: byte 0 = capability ID, byte 1 = next pointer.
///
/// Bounded to 48 iterations and validates that each next pointer is within
/// the standard capability range to prevent hangs on corrupted/cyclic chains.
fn parse_caps(virt: VirtAddr) -> Vec<CapHeader> {
    let ptr = virt.as_ptr::<u8>();
    let caps_ptr = unsafe { ptr.add(0x34).read_volatile() };
    if caps_ptr == 0 {
        return Vec::new();
    }

    let mut caps = Vec::new();
    let mut offset = caps_ptr;
    for _ in 0..48 {
        let id = unsafe { ptr.add(offset as usize).read_volatile() };
        let next = unsafe { ptr.add(offset as usize + 1).read_volatile() };
        caps.push(CapHeader { id, offset });
        if next == 0 {
            break;
        }
        if next < 0x40 || next == offset {
            break;
        }
        offset = next;
    }

    caps
}

/// Perform a full hierarchical scan of the PCIe bus tree.
///
/// 1. Seeds the work list with all buses from the MCFG segments.
/// 2. For each bus, iterates devices 0..31, function 0 (mandatory).
/// 3. If a function has a Type 1 header (PCI-to-PCI bridge), reads its
///    secondary / subordinate bus registers and enqueues any new buses
///    that haven't been scanned yet and fall inside a mapped segment.
/// 4. If the header type's multi-function bit is set, scans functions 1..7
///    and checks each for bridges too.
///
/// This replaces a simpler flat scan that only covered MCFG-listed buses.
pub fn scan_bus() -> Vec<PCIFunction> {
    let mut devices: Vec<PCIFunction> = Vec::new();
    let mut to_scan: Vec<u8> = Vec::new();
    let mut scanned = [false; 256];

    // Seed with every bus already known from ACPI MCFG segments.
    for seg in config::segments() {
        for bus in seg.start_bus..=seg.end_bus {
            if !scanned[bus as usize] {
                scanned[bus as usize] = true;
                to_scan.push(bus);
            }
        }
    }

    // Iterative DFS over the bus tree — fewer stack concerns than recursion.
    while let Some(bus) = to_scan.pop() {
        for device in 0..32 {
            let vendor_data = unsafe { config::read32(bus, device, 0, 0x00) };
            let vendor_id = (vendor_data & 0xFFFF) as u16;
            if vendor_id == 0xFFFF {
                continue;
            }

            devices.push(PCIFunction::new(bus, device, 0));

            let header = unsafe { config::read32(bus, device, 0, 0x0C) };
            let header_type = (header >> 16) & 0xFF;

            // Type 1 header → PCI-to-PCI bridge → discover secondary buses.
            if header_type & 0x7F == 0x01 {
                enqueue_bridge_buses(bus, device, 0, &mut to_scan, &mut scanned);
            }

            // Multi-function device — check functions 1..7.
            if header_type & 0x80 != 0 {
                for function in 1..8 {
                    let vendor_data = unsafe { config::read32(bus, device, function, 0x00) };
                    if (vendor_data & 0xFFFF) == 0xFFFF {
                        continue;
                    }
                    devices.push(PCIFunction::new(bus, device, function));

                    let func_header = unsafe { config::read32(bus, device, function, 0x0C) };
                    let func_hdr_type = (func_header >> 16) & 0xFF;
                    if func_hdr_type & 0x7F == 0x01 {
                        enqueue_bridge_buses(bus, device, function, &mut to_scan, &mut scanned);
                    }
                }
            }
        }
    }

    devices
}

/// Read the secondary/subordinate bus registers from a Type 1 header and
/// enqueue any new, in-range buses for scanning.
///
/// The registers reside at Type 1 header offset 0x18:
///   - byte 1: secondary bus number
///   - byte 2: subordinate bus number
///
/// A range where secondary > subordinate is invalid and silently skipped.
fn enqueue_bridge_buses(bus: u8, device: u8, function: u8, to_scan: &mut Vec<u8>, scanned: &mut [bool; 256]) {
    let reg = unsafe { config::read32(bus, device, function, 0x18) };
    let sec_bus = ((reg >> 8) & 0xFF) as u8;
    let sub_bus = ((reg >> 16) & 0xFF) as u8;

    if sec_bus > sub_bus {
        return;
    }

    for b in sec_bus..=sub_bus {
        if !scanned[b as usize] && config::bus_in_segment(b) {
            scanned[b as usize] = true;
            to_scan.push(b);
        }
    }
}
