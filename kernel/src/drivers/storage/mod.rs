//! Storage Driver for `MaramOS`.
//! It also provides abstractions for Storage I/O

use crate::{KernelError, errors::MemoryError, helpers::Time, memory::PhysPage, log_warn};
use super::pci::{DeviceType, find_devices, claim_device};
use alloc::{sync::Arc, vec::Vec};
use spin::Mutex;
use thiserror::Error;

pub const BLOCK_SIZE: u64 = 512;
const TIMEOUT: Time = Time::Seconds(5);

mod ahci;

use ahci::AHCIDrive;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("PCIe Error!")]
    PCIeError,
    #[error("Storage Device Init Failed!")]
    InitFailed,
    #[error("Storage: Invalid Block accessed!")]
    BlockOutOfBounds,
    #[error("Storage: Command Failed!")]
    CommandFailed,
    #[error("Storage: Timeout!")]
    Timeout
}

static DRIVES: Mutex<Vec<Arc<AHCIDrive>>> = Mutex::new(Vec::new());

pub fn claim_drive<F: Fn(&AHCIDrive) -> bool>(f: F) -> Option<Arc<AHCIDrive>> {
    let lock = DRIVES.lock();
    for (i, drive) in lock.iter().enumerate() {
        if f(drive) { return Some(lock[i].clone()) }
    }
    None
}

pub fn init() {
    let ahci_devices = find_devices(|d| d.device_type() == DeviceType::Ahci)
        .into_iter()
        .filter_map(claim_device);

    for device in ahci_devices {
        match ahci::init(device) {
            Ok(d) => DRIVES.lock().extend(d.into_iter().map(Arc::new)),
            Err(e) => log_warn!("Failed to initialize drive: {e}")
        }
    }
}

pub trait StorageDrive {
    /// Return the number of 512 byte blocks
    fn block_count(&self) -> u64;

    /// Read `count` 512 byte blocks starting at `start_block` into `dest`
    fn read_blocks(&self, start_block: u64, count: u64, dest: &mut PhysPage) -> Result<(), StorageError>;

    /// Write `count` 512 byte blocks starting at `start_block` from `src`
    fn write_blocks(&self, start_block: u64, count: u64, src: &PhysPage) -> Result<(), StorageError>;

    /// Zero `count` 512 byte blocks starting at `start_block`
    fn zero_blocks(&self, start_block: u64, count: u64) -> Result<(), StorageError>;

    /// Write all cached writes to disk
    fn sync(&self) -> Result<(), StorageError>;
    
    /// Read `count` 512 byte blocks starting at `start_block`
    fn read_smart(&self, start_block: u64, count: u64) -> Result<PhysPage, KernelError> {
        let page_count = count.div_ceil(8) as usize;
        let mut pages = PhysPage::new(page_count)
            .ok_or(MemoryError::OutOfMemory)?;

        self.read_blocks(start_block, count, &mut pages)?;

        Ok(pages)
    }
}
