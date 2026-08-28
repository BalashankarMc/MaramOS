//! Filesystem module for `MaramOS`.
//! Currently only supports `LemonFS`.
//! Will support more filesystems later

mod lba_abstractor;
mod lemonfs;

use lba_abstractor::PartitionDrive;
pub use lemonfs::LEMONFS_GUID;
use core::fmt::Debug;
use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
use spin::Mutex;
use thiserror::Error;

use crate::{drivers::storage::{Drive, PartitionType}, memory::PhysPage};

pub static FILESYSTEMS: Mutex<Vec<Arc<dyn FileSystem>>> = Mutex::new(Vec::new());

pub trait FileSystem: Debug + Send + Sync {
    fn read(&mut self, path: String, dest: &PhysPage) -> Result<u64, FSError>;
    fn write(&mut self, path: String, src: &PhysPage) -> Result<(), FSError>;
    fn list(&mut self, path: String) -> Result<Vec<String>, FSError>;
    fn create(&mut self, path: String, flags: u64) -> Result<(), FSError>;
    fn delete(&mut self, path: String) -> Result<(), FSError>;
    fn size(&mut self, path: String) -> Result<u64, FSError>;
    fn route_symlink(&mut self, symlink_path: String, dest_path: String) -> Result<(), FSError>;
    fn sync(&mut self) -> Result<(), FSError>;
}

#[derive(Debug, Error)]
pub enum FSError {
    #[error("Wrong Filesystem type!")]
    WrongFS,
    #[error("Invalid FS Superblock!")]
    BadSuperblock,
    #[error("I/O Error!")]
    IOError,
    #[error("Out of space on partition")]
    NoSpace,
    #[error("Wrong File Type!")]
    BadFileType,
    #[error("File not found!")]
    FileNotFound,
    #[error("File already exists!")]
    FileExists,
    #[error("Invalid Filename")]
    BadFilename,
    #[error("You cannot do this with FS root!")]
    IsRoot,
    #[error("Bad flags!")]
    BadFlags
}

pub fn init(drive: &Arc<Drive>) -> Result<Vec<Box<dyn FileSystem>>, FSError> {
    let partitions = drive.partitions();
    let mut filesystems: Vec<Box<dyn FileSystem>> = Vec::new();

    for partition in partitions {
        let partition_drive = PartitionDrive::new(drive.clone(), partition.clone());
        match partition.type_ {
            PartitionType::LemonFS => {
                let fs = lemonfs::init(partition_drive)?;
                let b = Box::new(fs);
                filesystems.push(b);    
            },

            PartitionType::Efi => (),
            PartitionType::Unknown(_) => {
                let superblock = partition_drive.read_sectors(0, 1).map_err(|_| FSError::IOError)?;
                let magic: [u8; 8] = superblock.read_data(0).ok_or(FSError::IOError)?;
                if magic == lemonfs::LEMONFS_MAGIC {
                    let fs = lemonfs::init(partition_drive)?;
                    let b = Box::new(fs);
                    filesystems.push(b);
                }
            }
        }

    }

    Ok(filesystems)
}