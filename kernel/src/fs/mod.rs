//! Filesystem abstraction layer.
//!
//! Defines the [`FileSystem`] trait and error types shared across all
//! filesystem implementations. Currently ships with LemonFS, a custom
//! on-disk filesystem with hash-table directories and link-chain large
//! file support.

mod lemonfs;

use alloc::{boxed::Box, string::String, vec::Vec};
pub use lemonfs::LemonFS;

use crate::{drivers::storage::StorageDrive, memory::PhysPage};

/// The LemonFS GUID (For GPT)
pub const LEMON_GUID: [u8; 16] = *b"LEMON DISK\0\0\0\0\0\0";

/// The FSError Enum
/// Provides descriptive error messages for Filesystem faults
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum FSError {
    FileNotFound,
    FileExists,
    DiskFull,
    IsFile,
    IsDirectory,
    IsRoot,
    InvalidFilename,
    IO,
    InvalidAccess,
    InitFailed,
    NoRoot,
    BadVersion
}

pub type FSReturn<T> = Result<T, FSError>;

pub trait FileSystem {
    fn init(drive: Box<dyn StorageDrive>) -> FSReturn<Self> where Self: Sized;
    
    // File & Directory Creation
    fn new_file(&mut self, path: &str, size: u64) -> FSReturn<()>;
    fn new_directory(&mut self, path: &str) -> FSReturn<()>;

    // File I/O
    fn read_file(&mut self, path: &str, dest: &mut PhysPage) -> FSReturn<()>;
    fn write_file(&mut self, path: &str, src: &PhysPage) -> FSReturn<()>;
    fn file_size(&mut self, path: &str) -> FSReturn<u64>;

    // Directory Listing
    fn list_directory(&mut self, path: &str) -> FSReturn<Vec<String>>;

    // File & Directory Moving
    fn move_(&mut self, src: &str, dest: &str) -> FSReturn<()>;

    // File & Directory Deletion
    fn remove_file(&mut self, path: &str) -> FSReturn<()>;
    fn remove_directory(&mut self, path: &str) -> FSReturn<()>;
}