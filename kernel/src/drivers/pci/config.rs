//! ECAM (Enhanced Configuration Access Mechanism) config-space access.
//!
//! Each PCI segment is described by an ACPI MCFG entry giving a physical base
//! address and a bus-number range.  The region is mapped as MMIO in the
//! kernel's virtual address space.  Config-space reads/writes are simple
//! volatile memory accesses at `offset = (bus << 20) | (dev << 15) | (func << 12) | reg`.

use crate::library::LateInit;
use crate::memory::{KMemory, PAGE_SIZE};

use alloc::vec::Vec;
use x86_64::{PhysAddr, VirtAddr};

/// A single PCI segment group, backed by a contiguous ECAM memory region.
#[derive(Debug)]
pub(crate) struct Segment {
    /// Physical base address of the ECAM region (from MCFG).
    pub(crate) base_address: u64,
    /// First bus number covered by this segment.
    pub(crate) start_bus: u8,
    /// Last bus number covered by this segment (inclusive).
    pub(crate) end_bus: u8,
    /// Virtual address where the ECAM region is mapped.
    pub(crate) virt_addr: VirtAddr,
}

static SEGMENTS: LateInit<Vec<Segment>> = LateInit::new();

/// Map all ECAM regions into the kernel MMIO aperture.
///
/// Each segment maps `(end_bus - start_bus + 1) << 20` bytes (1 MiB per bus).
pub(crate) fn init_segments(mut segments: Vec<Segment>) {
    for s in &mut segments {
        if s.end_bus < s.start_bus {
            panic!(
                "Invalid MCFG entry: end_bus ({}) < start_bus ({})",
                s.end_bus, s.start_bus
            );
        }
        let bytes = ((s.end_bus as u64 - s.start_bus as u64) + 1) << 20;
        let pages = (bytes as usize).div_ceil(PAGE_SIZE);
        s.virt_addr = KMemory::map_mmio(PhysAddr::new(s.base_address), pages);
    }
    SEGMENTS.init(segments);
}

/// Return all mapped segments.
pub(crate) fn segments() -> &'static [Segment] {
    SEGMENTS.get()
}

/// Return `true` when `bus` falls within one of the mapped segments.
///
/// Used during bridge scanning to avoid touching ECAM addresses for buses
/// the firmware did not describe in MCFG.
pub(crate) fn bus_in_segment(bus: u8) -> bool {
    segments()
        .iter()
        .any(|s| bus >= s.start_bus && bus <= s.end_bus)
}

/// Locate the segment that owns `bus`.  Panics if no segment covers it.
fn find_entry(bus: u8) -> &'static Segment {
    for s in segments() {
        if bus >= s.start_bus && bus <= s.end_bus {
            return s;
        }
    }
    panic!("No PCI segment for bus {}", bus);
}

/// Compute the virtual address of a function's config space.
///
/// ECAM layout: each bus occupies 1 MiB (32 devices × 8 functions × 4 KiB),
/// each device 32 KiB, each function 4 KiB.
pub(crate) fn ecam_virt(bus: u8, dev: u8, func: u8) -> VirtAddr {
    let entry = find_entry(bus);
    let offset = ((bus as u64 - entry.start_bus as u64) << 20)
        | ((dev as u64) << 15)
        | ((func as u64) << 12);

    entry.virt_addr + offset
}

/// Read 32 bits from a function's config space at the given byte `offset`.
///
/// # Safety
/// `bus`/`dev`/`func` must refer to a valid function; `offset` must be
/// 4-byte aligned (the implementation rounds down).
pub(crate) unsafe fn read32(bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
    let addr = ecam_virt(bus, dev, func);
    unsafe {
        addr.as_ptr::<u32>()
            .add((offset as usize & 0xFC) / 4)
            .read_volatile()
    }
}

/// Write 32 bits into a function's config space at the given byte `offset`.
///
/// # Safety
/// `bus`/`dev`/`func` must refer to a valid function; `offset` must be
/// 4-byte aligned (the implementation rounds down).
pub(crate) unsafe fn write32(bus: u8, dev: u8, func: u8, offset: u16, value: u32) {
    let addr = ecam_virt(bus, dev, func);
    unsafe {
        addr.as_mut_ptr::<u32>()
            .add((offset as usize & 0xFC) / 4)
            .write_volatile(value);
    }
}
