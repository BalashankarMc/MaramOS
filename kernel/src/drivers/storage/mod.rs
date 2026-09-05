//! Storage Driver for `MaramOS`.
//! It also provides abstractions for Storage I/O

use core::fmt::Debug;

use crate::{KernelError, KernelResult, helpers::Time, log_warn, memory::{PAGE_SIZE, PhysPage}};
use super::pci::{DeviceType, find_devices, claim_device};
use alloc::{sync::Arc, vec::Vec};
use spin::Mutex;
use thiserror::Error;

pub const BLOCK_SIZE: u64 = 512;
const TIMEOUT: Time = Time::Seconds(5);
const BLOCKS_PER_PAGE: usize = PAGE_SIZE.div_ceil(BLOCK_SIZE as usize);

mod ahci;
mod gpt;

pub use gpt::{Partition, PartitionType, Guid};

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

#[derive(Error, Debug)]
pub enum GPTError {
    #[error("I/O Error!")]
    IOError,
    #[error("Bad / Corrupt GPT header!")]
    BadHeader,
    #[error("Entry checksum failed!")]
    EntryCRCFailed,
    #[error("Bad MBR Data!")]
    BadMBR
}

#[derive(Debug)]
pub struct Drive {
    inner: Arc<dyn StorageDrive>,
    partitions: Vec<Partition>
}

impl Drive {
    pub const fn partitions(&self) -> &Vec<Partition> { &self.partitions }
    
    pub fn find_partition<F: FnMut(&&Partition) -> bool>(&self, f: F) -> Option<&Partition> {
        self.partitions.iter().find(f)
    }

    pub fn zero(&self, partition: &Partition, offset: u64, count: u64) -> KernelResult<()> {
        if offset.checked_add(count).is_none() { Err(StorageError::BlockOutOfBounds)? }

        if offset + count > partition.size_blocks { Err(StorageError::BlockOutOfBounds)? }

        let start_block = partition.start.checked_add(offset).ok_or(StorageError::BlockOutOfBounds)?;
        self.inner.zero_blocks(start_block, count)
    }

    pub fn read(&self, partition: &Partition, offset: u64, count: u64) -> KernelResult<PhysPage> {
        if offset.checked_add(count).is_none() { Err(StorageError::BlockOutOfBounds)? }

        if offset + count > partition.size_blocks { Err(StorageError::BlockOutOfBounds)? }

        let start_block = partition.start.checked_add(offset).ok_or(StorageError::BlockOutOfBounds)?;
        self.inner.read_smart(start_block, count)
    }

    pub fn write(&self, partition: &Partition, offset: u64, count: u64, src: &PhysPage) -> KernelResult<()> {
        if offset.checked_add(count).is_none() { Err(StorageError::BlockOutOfBounds)? }

        if offset + count > partition.size_blocks { Err(StorageError::BlockOutOfBounds)? }
        if src.count * PAGE_SIZE < (count * self.inner.block_size()) as usize { Err(StorageError::CommandFailed)? }

        let start_block = partition.start.checked_add(offset).ok_or(StorageError::BlockOutOfBounds)?;
        self.inner.write_blocks(start_block, count, src)?;

        Ok(())        
    }

    pub fn sync(&self) -> KernelResult<()> {
        self.inner.sync()?;
        Ok(())
    }

    pub fn block_size(&self) -> u64 { self.inner.block_size() }
    pub fn block_count(&self) -> u64 { self.inner.block_count() }
}

static DRIVES: Mutex<Vec<Arc<Drive>>> = Mutex::new(Vec::new());

pub fn claim_drive<F: Fn(&Drive) -> bool>(f: F) -> Option<Arc<Drive>> {
    let lock = DRIVES.lock();
    for drive in lock.iter() {
        if f(drive) { return Some(drive.clone()) }
    }
    None
}

pub fn init() -> KernelResult<()> {
    let ahci_devices: Vec<Arc<dyn StorageDrive>> = find_devices(|d| d.device_type() == DeviceType::Ahci)
        .into_iter()
        .filter_map(claim_device)
        .filter_map(|drive| match ahci::init(drive) {
            Ok(d) => Some(d),
            Err(e) => {
                log_warn!("Failed to initialize AHCI drive: {e}");
                None
            }
        })
        .flatten()
        .map(|d| Arc::new(d) as Arc<dyn StorageDrive>)
        .collect();

    let mut lock = DRIVES.lock();

    for device in ahci_devices {
        let partitions = gpt::init(&device)?;
        let drive = Drive {
            inner: device,
            partitions
        };

        lock.push(Arc::new(drive));
    }

    Ok(())
}

trait StorageDrive: Debug + Send + Sync {
    /// Return the number of blocks
    fn block_count(&self) -> u64;

    /// Returns the block size of the drive
    fn block_size(&self) -> u64;

    /// Read `count` blocks starting at `start_block` into `dest`
    fn read_blocks(&self, start_block: u64, count: u64, dest: &PhysPage) -> Result<(), StorageError>;

    /// Write `count` blocks starting at `start_block` from `src`
    fn write_blocks(&self, start_block: u64, count: u64, src: &PhysPage) -> Result<(), StorageError>;

    fn zero_blocks(&self, start_block: u64, count: u64) -> KernelResult<()> {
        let pages = (count * self.block_size()).div_ceil(PAGE_SIZE as u64).min(512); // Max 512 pages (2MiB)
        let page = PhysPage::new(pages as usize)?;
        
        let mut rem = count;
        let mut offset = 0;

        while rem > 0 {
            let c = rem.min((1024 * 1024 * 2) / self.block_size()); // Max Blocks
            self.write_blocks(start_block + offset, c, &page)?;
            rem -= c;
            offset += c;
        }

        Ok(())
    }

    /// Write all cached writes to disk
    fn sync(&self) -> Result<(), StorageError>;
    

    /// Read `count` natively sized blocks starting at `start_block`
    fn read_smart(&self, start_block: u64, count: u64) -> Result<PhysPage, KernelError> {
        let blocks_per_page = PAGE_SIZE.div_ceil(self.block_size() as usize);
        let page_count = (count as usize).div_ceil(blocks_per_page);
        let page = PhysPage::new(page_count)?;

        self.read_blocks(start_block, count, &page)?;

        Ok(page)
    }
}
