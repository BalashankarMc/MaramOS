//! GUID Partition Table (GPT) parsing.
//!
//! Validates the protective MBR, verifies primary/backup GPT header CRC32
//! checksums, and enumerates partition entries. Partition drives are wrapped
//! as address-translating [`PartitionDrive`] instances that implement
//! [`StorageDrive`](crate::drivers::storage::StorageDrive).

mod headers;
mod partition;
mod partition_drive;

use alloc::{sync::Arc, vec::Vec};
use partition::*;

pub use headers::GPTPartitionEntry;
pub use partition_drive::{PartitionDrive, PartitionType};
use spin::Mutex;

use crate::drivers::storage::Drive;

type GResult<T> = Result<T, GPTError>;

#[derive(Debug)]
pub enum GPTError {
    InvalidProtectiveMBR,
    BadPartition,
    BadGPTHeader,
    IO
}

pub fn test_gpt(drive: Arc<Mutex<Drive>>) -> GResult<Vec<GPTPartitionEntry>> {
    let header = attest_integrity(drive.clone())?;
    parse_partitions(drive.clone(), header)
}

pub fn parse_gpt(drive: Arc<Mutex<Drive>>) -> GResult<Vec<PartitionDrive>> {
    let header = attest_integrity(drive.clone())?;
    let partitions = parse_partitions(drive.clone(), header)?;

    let mut res = Vec::with_capacity(partitions.len());

    for partition in partitions {
        res.push(PartitionDrive::from_partition(drive.clone(), &partition));
    }

    Ok(res)
}