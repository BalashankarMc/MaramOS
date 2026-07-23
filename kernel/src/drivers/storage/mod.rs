//! A wrapper module for storage device drivers
//!
//! All storage devices MUST implement the StorageDrive trait to ensure proper usability
//!
//! # Safety: Contains potentially unsafe read+writes to storage drives and unsafe casting
//! of PCI Functions as storage devices (Use with caution)

mod nvme;
mod ahci;
mod storage_cache;

pub use nvme::COMPLETION_FLAG as nvme_io_completion;
use core::fmt::{self, Debug};

use alloc::{boxed::Box, vec::Vec};

use crate::{
    drivers::{pci::{DeviceType, PCIDevice, PCIFunction}, storage::storage_cache::{BlockCache, BlockData}}, library::Time, memory::{KMemory, PAGE_SIZE, PhysPage}
};

pub const TIMEOUT: u64 = Time::Seconds(10).to_nanos();
/// The supported LBA Size in bytes
pub const LBA_SIZE: u64 = 512;
/// Number of LBAs to prefetch after each demand read (one 4 KiB page).
const PREFETCH_WINDOW: u64 = 8;

/// The IOError struct. Used to describe any and all possible I/O Errors
#[derive(Debug)]
pub enum IOError {
    Timeout,
    CommandFailure,
    InitFailed,
    IncompatibleVersion,
    OutOfBoundsAccess
}

impl fmt::Display for IOError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IOError::Timeout => write!(f, "IO operation timed out"),
            IOError::CommandFailure => write!(f, "Command failed"),
            IOError::InitFailed => write!(f, "Device initialization failed"),
            IOError::IncompatibleVersion => write!(f, "Incompatible device version"),
            IOError::OutOfBoundsAccess => write!(f, "Out-of-bounds IO access attempted")
        }
    }
}

/// The Storage Drive trait to be implemented by storage drives
pub trait StorageDrive: Send + Debug {
    /// Returns the devices capacity in Bytes
    fn capacity(&self) -> u64;

    /// Read the given LBAs from the drive and onto a PhysPage. Returns an IOError on fault
    ///
    /// # Safety: The user must ensure the LBA and LBA + count stays inside the drive's capacity
    fn read_lbas(&mut self, lba: u64, count: u64, dest: &mut PhysPage) -> Result<(), IOError>;

    /// A smarter read function. Creates the PhysPage needed to read the requested lbas
    /// # Safety: The user must ensure the LBA and LBA + count stays inside the drive's capacity
    fn read_smart(&mut self, lba: u64, count: u64) -> Result<PhysPage, IOError> {
        let pages_needed = lbas_to_pages(count);
        let mut page = KMemory::alloc_pages(pages_needed);

        self.read_lbas(lba, count, &mut page)?;
        
        Ok(page)
    }

    /// Write to the given LBAs on the drive from PhysPage. Returns an IOError on fault
    ///
    /// # Safety: The user must ensure the LBA and LBA + count stays inside the drive's capacity
    fn write_lbas(&mut self, lba: u64, count: u64, src: &PhysPage) -> Result<(), IOError>;

    /// Writes zeroes the given LBAs on the drive. Returns an IOError on fault
    ///
    /// # Safety: The user must ensure the LBA and LBA + count stays inside the drive's capacity
    fn zero_lbas(&mut self, lba: u64, count: u64) -> Result<(), IOError>;
}

#[derive(Debug)]
pub struct Drive {
    inner: Box<dyn StorageDrive>,
    cache: storage_cache::BlockCache,
    block_data: storage_cache::BlockData,
}

impl StorageDrive for Drive {
    fn capacity(&self) -> u64 {
        self.inner.capacity()
    }

    fn read_lbas(&mut self, lba: u64, count: u64, dest: &mut PhysPage) -> Result<(), IOError> {
        let size = self.capacity() / LBA_SIZE;
        if lba + count > size { return Err(IOError::OutOfBoundsAccess) }
        
        if count == 1 {
            if let Some(cached) = self.cache.get_lba(lba) {
                self.block_data.record_access(lba);
                dest.write_data(0, cached);
                return Ok(());
            }
            self.inner.read_lbas(lba, 1, dest)?;
            self.block_data.record_access(lba);
            let data: [u8; LBA_SIZE as usize] = dest.read_data(0);
            self.cache.insert_lba(lba, &data, &self.block_data);
            Ok(())
        } else {
            let capacity_lbas = self.capacity() / LBA_SIZE;
            let prefetch_end = (lba + count + PREFETCH_WINDOW).min(capacity_lbas);
            let prefetch = prefetch_end - (lba + count);

            if prefetch == 0 { return self.inner.read_lbas(lba, count, dest) }

            let total = count + prefetch;
            let pages = lbas_to_pages(total);
            let mut combined = KMemory::alloc_pages(pages);

            self.inner.read_lbas(lba, total, &mut combined)?;

            let src_ptr = combined.get_virt_addr().as_ptr::<u8>();
            let dst_ptr = dest.get_virt_addr().as_mut_ptr::<u8>();
            unsafe {
                core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, (count as usize) * LBA_SIZE as usize);
            }

            for i in 0..prefetch as usize {
                let mut data = [0u8; LBA_SIZE as usize];
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        src_ptr.add((count as usize + i) * LBA_SIZE as usize),
                        data.as_mut_ptr(),
                        LBA_SIZE as usize,
                    );
                }
                self.cache.insert_lba(lba + count + i as u64, &data, &self.block_data);
            }

            Ok(())
        }
    }

    fn write_lbas(&mut self, lba: u64, count: u64, src: &PhysPage) -> Result<(), IOError> {
        let size = self.capacity() / LBA_SIZE;
        if lba + count > size { return Err(IOError::OutOfBoundsAccess) }

        self.inner.write_lbas(lba, count, src)?;
        if count == 1 {
            let data: [u8; LBA_SIZE as usize] = src.read_data(0);
            self.cache.update_lba(lba, &data);
        } else {
            for i in 0..count {
                self.cache.invalidate(lba + i);
            }
        }
        Ok(())
    }

    fn zero_lbas(&mut self, lba: u64, count: u64) -> Result<(), IOError> {
        let size = self.capacity() / LBA_SIZE;
        if lba + count > size { return Err(IOError::OutOfBoundsAccess) }

        self.inner.zero_lbas(lba, count)?;
        for i in 0..count {
            self.cache.invalidate(lba + i);
        }
        Ok(())
    }
}

/// Convert a given number of LBAs to number of 4KiB pages
pub fn lbas_to_pages(lba_count: u64) -> usize {
    ((lba_count * LBA_SIZE) as usize).div_ceil(PAGE_SIZE)
}

/// Initialize the given PCIFunction as a Storage Drive.
///
/// # Safety: User must ensure the given PCIFunction is a valid and supported storage device
pub fn init_drive(device: &'static PCIFunction) -> Result<Box<dyn StorageDrive>, IOError> {
    match device.device_type() {
        DeviceType::Nvme => {
            Ok(Box::new(nvme::NVMeDrive::new(device)?))
        },

        DeviceType::Ahci => {
            let mut dvec = ahci::init(device)?;
            if dvec.is_empty() { return Err(IOError::InitFailed) }

            let drive = dvec.pop().unwrap();
            Ok(Box::new(drive))
        },

        _ => Err(IOError::InitFailed)
    }
}

pub fn init_storage() -> Vec<Drive> {
    let mut drives = Vec::new();
    for device in super::pci::find_devices(|dev| dev.class() == 1) {
        if let Ok(drive) = init_drive(device) { drives.push(Drive {
            inner: drive,
            cache: BlockCache::new(),
            block_data: BlockData::new()
        }) }
    }
    drives
}