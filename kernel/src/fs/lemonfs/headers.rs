//! Contains all the data headers for LemonFS

use super::{MAGIC, FILENAME_LEN};
use super::super::{FSError, FSReturn};
use crate::drivers::storage::{LBA_SIZE as LBA_U64, StorageDrive, lbas_to_pages};
use crate::fs::lemonfs::io;
use crate::memory::KMemory;

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::mem::size_of;
use core::ops::Deref;

const LBA_SIZE: usize = LBA_U64 as usize;
pub const CHILDREN_PER_BLOCK: usize = LBA_SIZE / size_of::<HashPointer>();

pub const HASHES_PER_BLOCK: usize = LBA_SIZE / size_of::<HashEntry>();
pub const LINKS_PER_BLOCK: usize = LBA_SIZE / size_of::<LinkEntry>();
const HASHPTRS_PER_BLOCK: usize = LBA_SIZE / size_of::<HashPointer>();

const BITMAP_SIZE: usize = LBA_SIZE;
const HASHBLOCK_PADDING: usize = LBA_SIZE % size_of::<HashEntry>();
const LINKBLOCK_PADDING: usize = LBA_SIZE % size_of::<LinkEntry>();
const DIRBLOCK_PADDING: usize = LBA_SIZE % size_of::<HashPointer>();

const _: () = assert!(size_of::<SuperBlock>() == LBA_SIZE);
const _: () = assert!(size_of::<HashTableBlock>() == LBA_SIZE);
const _: () = assert!(size_of::<LinkBlock>() == LBA_SIZE);

/// Superblock for LemonFS
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct SuperBlock {
    pub magic: u64,
    pub version: u16,
    pub total_blocks: u64,
    pub hashmap_size: u64,
    pub bitmap_size: u64,
    pub dirty: bool,
    _padding: [u8; 477],
} // LBA_SIZE Bytes

impl SuperBlock {
    /// Create a new superblock for the given storage drive
    pub fn new(drive: &mut dyn StorageDrive) -> Self {
        let lbas = drive.capacity() / LBA_U64;
        let hashmap_size = core::cmp::max(4096, lbas / 10_000);
        // Lbas - (SB + Hash Blocks) = Bitmap + Data
        let remaining = lbas - (1 + hashmap_size.div_ceil(HASHES_PER_BLOCK as u64));
        let bitmap_size = remaining.div_ceil(4096);

        Self {
            magic: MAGIC,
            version: 2,
            total_blocks: lbas,
            hashmap_size,
            bitmap_size,
            dirty: false,
            _padding: [u8::MAX; 477]
        }
    }

    pub fn bitmap_start(&self) -> u64 { 1 + self.hashmap_size.div_ceil(HASHES_PER_BLOCK as u64) }
    pub fn data_start(&self) -> u64 { self.bitmap_start() + self.bitmap_size }
}

// ---------- Hashes ---------- 

/// An entry in the hash table
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct HashEntry {
    pub status: HashStatus,
    pub type_: FileType,
    pub name: [u8; 40],
    pub start_block: u64,
    pub file_size: u64,
    pub link: LinkEntry
}

impl HashEntry {
    pub fn new(type_: FileType, name: [u8; 40], start: u64, size: u64) -> Self {
        HashEntry {
            status: HashStatus::Used,
            type_,
            name,
            start_block: start,
            file_size: size,
            link: LinkEntry { ptr: 0, size: 0 }
        }
    }
}

/// An LBA's worth of Hash Entries
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct HashTableBlock {
    pub entries: [HashEntry; HASHES_PER_BLOCK],
    _padding: [u8; HASHBLOCK_PADDING]
}

/// An pointer pointing to a hash entry by pointing to it's hash block and its index
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct HashPointer {
    pub lba: u64,
    pub offset: u8
}

impl HashPointer {
    /// Pass in index % sb.hash_size
    pub fn new(index: u64) -> Self {
        let lba = 1 + index / HASHES_PER_BLOCK as u64;
        let offset = (index % HASHES_PER_BLOCK as u64) as u8;
        Self { lba, offset }
    }

    pub fn lba(&self) -> u64 {
        self.lba
    }
}

/// The hash's status (Used, Unused, Tombstone)
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum HashStatus {
    Unused = 0,
    Used,
    Tombstone = u8::MAX
}

/// All representable filetypes
#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum FileType {
    File = 0,
    Directory,
    Unknown,
}

impl From<u8> for FileType {
    fn from(v: u8) -> Self {
        match v {
            0 => FileType::File,
            1 => FileType::Directory,
            _ => FileType::Unknown,
        }
    }
}

// ---------- Links ---------- 

/// A pointer to the next fragment of a file.
/// Stores the pointer to the data (if direct) and the size of the link.
/// If indirect, ptr points to the link-lba and size is 0
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct LinkEntry {
    pub ptr: u64,
    pub size: u64
}

/// An LBA containing LinkEntries
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct LinkBlock {
    pub entries: [LinkEntry; LINKS_PER_BLOCK],
    _padding: [u8; LINKBLOCK_PADDING]
}

impl LinkBlock {
    pub fn new() -> Self {
        Self { ..unsafe { core::mem::zeroed() } }
    }
}

// ---------- Bitmap ----------

/// A Bitmap for file allocation
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct Bitmap {
    pub entries: [u8; BITMAP_SIZE],
}

impl Bitmap {
    pub fn new() -> Self { Bitmap { entries: [0; BITMAP_SIZE] } }
    pub fn set(&mut self, index: usize, used: bool) -> FSReturn<()> {
        if index >= BITMAP_SIZE * 8 { return Err(FSError::IO) }
        let (byte_idx, mask) = self.byte_index(index);
        if used { self.entries[byte_idx] |= mask } else { self.entries[byte_idx] &= !mask }
        Ok(())
    }

    pub fn check(&self, index: usize) -> FSReturn<bool> {
        if index >= BITMAP_SIZE * 8 { return Err(FSError::IO) }
        let (byte_idx, mask) = self.byte_index(index);
        Ok(self.entries[byte_idx] & mask > 0)
    }

    fn byte_index(&self, index: usize) -> (usize, u8) { (index / 8, 1 << (index % 8)) }
}

// ---------- Directory ----------

/// An Entry for a directory (1 LBA)
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct DirectoryEntry {
    pub entries: [HashPointer; HASHPTRS_PER_BLOCK], // Pointers to HashBlocks + offset into block
    _padding: [u8; DIRBLOCK_PADDING]
}

impl DirectoryEntry {
    pub fn new() -> Self { DirectoryEntry { .. unsafe { core::mem::zeroed() } } }
}

// ---------- Directory Cache ----------

/// In-memory cache for directory hash entries.
/// Fixed capacity of 128 entries with LRU eviction.
#[derive(Debug)]
pub struct DirectoryCache {
    entries: [(u64, HashEntry); 128],
    ages: [u64; 128],
    count: usize,
    tick: u64,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self {
            entries: [(0, unsafe { core::mem::zeroed() }); 128],
            ages: [0; 128],
            count: 0,
            tick: 0,
        }
    }

    pub fn insert(&mut self, parent: u64, entry: &HashEntry) {
        // Update existing
        for i in 0..self.count {
            if self.entries[i].0 == parent && name_eq(&self.entries[i].1.name, entry_name_str(&entry.name)) {
                self.entries[i].1 = *entry;
                self.ages[i] = self.tick;
                self.tick += 1;
                return;
            }
        }
        // Evict LRU if full
        if self.count == 128 {
            let mut oldest_idx = 0;
            let mut oldest_age = self.ages[0];
            for i in 1..128 {
                if self.ages[i] < oldest_age {
                    oldest_age = self.ages[i];
                    oldest_idx = i;
                }
            }
            self.entries[oldest_idx] = (parent, *entry);
            self.ages[oldest_idx] = self.tick;
        } else {
            self.entries[self.count] = (parent, *entry);
            self.ages[self.count] = self.tick;
            self.count += 1;
        }
        self.tick += 1;
    }

    pub fn lookup(&mut self, parent: u64, name: &str) -> Option<&HashEntry> {
        for i in 0..self.count {
            if self.entries[i].0 == parent && name_eq(&self.entries[i].1.name, name) {
                self.ages[i] = self.tick;
                self.tick += 1;
                return Some(&self.entries[i].1);
            }
        }
        None
    }

    pub fn remove(&mut self, parent: u64, name: &str) {
        for i in 0..self.count {
            if self.entries[i].0 == parent && name_eq(&self.entries[i].1.name, name) {
                // Shift remaining entries
                for j in i..self.count - 1 {
                    self.entries[j] = self.entries[j + 1];
                    self.ages[j] = self.ages[j + 1];
                }
                self.count -= 1;
                return;
            }
        }
    }
}

fn name_eq(bytes: &[u8], expected: &str) -> bool {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(40);
    core::str::from_utf8(&bytes[..len]).unwrap_or_default() == expected
}

fn entry_name_str(name: &[u8]) -> &str {
    let len = name.iter().position(|&b| b == 0).unwrap_or(40);
    core::str::from_utf8(&name[..len]).unwrap_or_default()
}