//! MCFG (Memory-Mapped Configuration) table parsing.

use alloc::vec::Vec;
use x86_64::VirtAddr;

use crate::LateInit;

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct MCFGEntry {
    pub base_address: u64,
    pub segment_group: u16,
    pub start_bus: u8,
    pub end_bus: u8,
    reserved: [u8; 4],
}

pub static MCFG_ENTRIES: LateInit<Vec<MCFGEntry>> = LateInit::new();

/// Initializes MCFG data.
/// 
/// # Safety
/// `mcfg_header` must point to a valid MCFG Table
pub unsafe fn init(mcfg_header: VirtAddr) {
    let length = unsafe { mcfg_header.as_ptr::<u32>().add(1).read_unaligned() };
    if length < 44 {
        MCFG_ENTRIES.init(Vec::new());
        return;
    }
    let entry_count = (length as usize - 44) / 16;
    let entries = (mcfg_header + 44).as_ptr::<MCFGEntry>();
    let mut vec = Vec::with_capacity(entry_count);
    for i in 0..entry_count {
        let entry = unsafe { entries.add(i).read_unaligned() };
        vec.push(entry);
    }
    MCFG_ENTRIES.init(vec);
}
