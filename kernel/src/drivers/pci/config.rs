use alloc::vec::Vec;
use x86_64::PhysAddr;

use crate::{LateInit, acpi::MCFGEntry, drivers::pci::PCIError, memory::{MMIORegion, PAGE_SIZE}};

/// A single PCI segment group, backed by a contiguous ECAM memory region.
pub struct Segment {
    pub base_address: u64,
    pub start_bus: u8,
    pub end_bus: u8,
    pub region: Option<MMIORegion>
}

impl Segment {
    pub const fn from_mcfg_entry(entry: &MCFGEntry) -> Self {
        Self {
            base_address: entry.base_address,
            start_bus: entry.start_bus,
            end_bus: entry.end_bus,
            region: None
        }
    }
}

static SEGMENTS: LateInit<Vec<Segment>> = LateInit::new();

pub fn init_segments(mut segments: Vec<Segment>) -> Result<(), PCIError> {
    for segment in &mut segments {
        if segment.start_bus > segment.end_bus { return Err(PCIError::InvalidMCFGEntry) }

        let bytes = (usize::from(segment.end_bus - segment.start_bus) + 1) << 20;
        let pages = bytes.div_ceil(PAGE_SIZE);
        segment.region = MMIORegion::new(PhysAddr::new(segment.base_address), pages);
    }

    SEGMENTS.init(segments);
    Ok(())
}

pub fn segments<'a>() -> &'a [Segment] {
    &SEGMENTS
}

pub fn ecam_offset(entry: &Segment, bus: u8, device: u8, function: u8, offset: u16) -> Option<usize> {

    if bus < entry.start_bus || bus > entry.end_bus { return None }

    let bus_offset = usize::from(bus - entry.start_bus) << 20;
    let dev_offset = usize::from(device) << 15;
    let func_offset = usize::from(function) << 12;
    let reg_offset = usize::from(offset) & 0xFC;

    Some(bus_offset | dev_offset | func_offset | reg_offset)
}

pub fn read32(entry: &Segment, bus: u8, device: u8, function: u8, offset: u16) -> Option<u32> {
    let offset = ecam_offset(entry, bus, device, function, offset)?;

    entry.region.as_ref()?.read(offset)
}

pub fn write32(entry: &Segment, bus: u8, device: u8, function: u8, offset: u16, val: u32) -> Option<()> {
    let offset = ecam_offset(entry, bus, device, function, offset)?;

    if entry.region.as_ref()?.write(offset, val) { return Some(()) }
    None
}

pub fn read16(entry: &Segment, bus: u8, device: u8, function: u8, offset: u16) -> Option<u16> {
    let aligned = offset & 0xFFFE;
    let shift = (offset & 2) * 8;
    let dword = read32(entry, bus, device, function, aligned)?;
    Some((dword >> shift) as u16)
}

pub fn write16(entry: &Segment, bus: u8, device: u8, function: u8, offset: u16, val: u16) -> Option<()> {
    let aligned = offset & 0xFFFE;
    let shift = (offset & 2) * 8;
    let old = read32(entry, bus, device, function, aligned)?;
    let mask = 0xFFFF << shift;
    write32(entry, bus, device, function, aligned, (old & !mask) | (u32::from(val) << shift))
}

pub fn read8(entry: &Segment, bus: u8, device: u8, function: u8, offset: u16) -> Option<u8> {
    let aligned = offset & 0xFFFC;
    let shift = (offset & 3) * 8;
    let dword = read32(entry, bus, device, function, aligned)?;
    
    Some((dword >> shift) as u8)
}

pub fn write8(entry: &Segment, bus: u8, device: u8, function: u8, offset: u16, val: u8) -> Option<()> {
    let aligned = offset & 0xFFFC;
    let shift = (offset & 3) * 8;
    let old = read32(entry, bus, device, function, aligned)?;
    let mask = 0xFF << shift;

    write32(entry, bus, device, function, aligned, (old & !mask) | (u32::from(val) << shift))
}

pub fn bus_in_entry(bus: u8) -> bool {
    segments()
        .iter()
        .any(|s| bus >= s.start_bus && bus <= s.end_bus)
}

pub fn find_entry<'a>(bus: u8) -> Option<&'a Segment> {
    segments()
        .iter()
        .find(|&s| bus >= s.start_bus && bus <= s.end_bus )
}