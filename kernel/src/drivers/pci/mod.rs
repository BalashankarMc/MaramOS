use core::fmt::Display;

use alloc::vec::Vec;

use config::Segment;
pub use device::PCIFunction;

use crate::{LateInit, log_success};

mod config;
mod device;
mod msix;
mod msi;

#[derive(Debug)]
pub enum PCIError {
    InvalidMCFGEntry
}

/// Classification of PCI function by its base class / subclass code.
#[derive(PartialEq, Eq, Debug)]
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

impl Display for DeviceType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::Ahci => "AHCI Storage",
            Self::Nvme => "NVMe Storage",
            Self::Eth => "Ethernet",
            Self::Wifi => "Wifi Card",
            Self::GPU3D => "GPU",
            Self::Hda => "Audio Device",
            Self::Vga => "VGA Output",
            Self::Unknown => "Unknown"
        };

        f.write_str(s)
    }
}

pub static DEVICES: LateInit<Vec<PCIFunction>> = LateInit::new();

pub fn init() -> Result<(), PCIError> {
    let entries = crate::acpi::mcfg_entries();

    let segments = entries
        .iter()
        .map(Segment::from_mcfg_entry)
        .collect::<Vec<Segment>>();

    config::init_segments(segments)?;

    let devices = scan();
    DEVICES.init(devices);

    log_success!("PCIe subsystem initialized!");
    Ok(())
}

fn scan() -> Vec<PCIFunction> {
    let mut devices = Vec::new();
    let mut to_scan = Vec::new();
    let mut scanned_buses = [false; 256];

    // Seed every bus from the MCFG Segments list
    for segment in config::segments() {
        for bus in segment.start_bus ..= segment.end_bus {
            if !scanned_buses[usize::from(bus)] {
                scanned_buses[usize::from(bus)] = true;
                to_scan.push((segment, bus));
            }
        }
    }

    while let Some((segment, bus)) = to_scan.pop() {
        for device in 0..32 {
            if config::read16(segment, bus, device, 0, 0).unwrap_or(0xFFFF) == 0xFFFF { continue }

            let header = config::read32(segment, bus, device, 0, 0xC).unwrap_or(0);
            let header_type = (header >> 16) & 0xFF;

            visit_function(segment, bus, device, 0, &mut devices, &mut to_scan, &mut scanned_buses);

            if header_type & 0x80 != 0 {
                for function in 1..8 { // Zero already visited
                    if config::read16(segment, bus, device, function, 0).unwrap_or(0xFFFF) == 0xFFFF { continue }
                    visit_function(segment, bus, device, function, &mut devices, &mut to_scan, &mut scanned_buses);
                }
            }
        }
    }

    devices
}

fn visit_function<'a>(segment: &'a Segment, bus: u8, device: u8, function: u8, devices: &mut Vec<PCIFunction>, to_scan: &mut Vec<(&'a Segment, u8)>, scanned: &mut [bool; 256]) {
    let Some(func) = PCIFunction::new(bus, device, function) else { return };
    devices.push(func);

    let header = config::read32(segment, bus, device, function, 0xC).unwrap_or_default();
    if (header >> 16) & 0x7F == 1 {
        let reg = config::read32(segment, bus, device, function, 0x18).unwrap_or_default();
        let sec_bus = ((reg >> 8) & 0xFF) as u8;
        let sub_bus = ((reg >> 16) & 0xFF) as u8;

        if sec_bus <= sub_bus {
            for b in sec_bus..=sub_bus {
                if !scanned[usize::from(b)] {
                    scanned[usize::from(b)] = true;
                    to_scan.push((segment, b));
                }
            }
        }
    }
}

pub fn find_devices(f: impl Fn(&PCIFunction) -> bool) -> Vec<&'static PCIFunction> {
    DEVICES.iter().filter(|d| f(d)).collect()
}