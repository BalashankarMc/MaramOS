//! GPT partition-as-drive abstraction.
//!
//! Wraps a [`Drive`] with LBA offset translation so each partition
//! appears as an independent [`StorageDrive`] starting at LBA 0.
//! Classifies partitions by type GUID (LemonFS, Reserved, Unknown).

use alloc::{string::String, sync::Arc};
use spin::Mutex;

use crate::{drivers::storage::{Drive, LBA_SIZE, StorageDrive}, fs::LEMON_GUID, gpt::{GPTPartitionEntry, partition::EFI_GUID}};

#[derive(Debug, PartialEq)]
pub enum PartitionType {
    Reserved,
    LemonFS,
    Unknown
}

#[derive(Debug)]
pub struct PartitionDrive {
    pub parent: Arc<Mutex<Drive>>,
    pub start_lba: u64,
    pub blocks: u64,
    pub name: String,
    pub type_: PartitionType
}

impl PartitionDrive {
    pub fn from_partition(drive: Arc<Mutex<Drive>>, entry: &GPTPartitionEntry) -> Self {
        let mut name = String::with_capacity(36);
        for c in entry.name {
            if c == 0 { break }
            if let Some(res) = char::from_u32(c as u32) { name.push(res) }
        }

        Self {
            parent: drive,
            start_lba: entry.start_lba,
            blocks: entry.end_lba - entry.start_lba + 1,
            name,
            type_: match entry.partition_type {
                LEMON_GUID => PartitionType::LemonFS,
                EFI_GUID => PartitionType::Reserved,
                _ => PartitionType::Unknown
            }
        }
    }
}

use crate::drivers::storage::IOError;
use crate::memory::PhysPage;

impl StorageDrive for PartitionDrive {
    fn capacity(&self) -> u64 { self.blocks * LBA_SIZE }
    
    fn read_lbas(&mut self, lba: u64, count: u64, dest: &mut PhysPage) -> Result<(), IOError> {
        let new_lba = lba + self.start_lba;
        if lba + count > self.blocks { return Err(IOError::OutOfBoundsAccess) }

        self.parent.lock().read_lbas(new_lba, count, dest)
    }

    fn write_lbas(&mut self, lba: u64, count: u64, src: &PhysPage) -> Result<(), IOError> {
        let new_lba = lba + self.start_lba;
        if lba + count > self.blocks { return Err(IOError::OutOfBoundsAccess) }

        self.parent.lock().write_lbas(new_lba, count, src)
    }

    fn zero_lbas(&mut self, lba: u64, count: u64) -> Result<(), IOError> {
        let new_lba = lba + self.start_lba;
        if lba + count > self.blocks { return Err(IOError::OutOfBoundsAccess) }

        self.parent.lock().zero_lbas(new_lba, count)
    }
}