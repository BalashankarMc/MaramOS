//! The on-disk data structures for `LemonFS`

use crate::fs::lemonfs::LemonData;
use alloc::string::String;
use bitflags::bitflags;

use super::LEMONFS_MAGIC;

const _: () = assert!(size_of::<SuperBlock>() == 512);
const _: () = assert!(size_of::<BitmapBlock>() == 512);
const _: () = assert!(size_of::<LinkBlock>() == 512);

pub const ENTRIES_PER_DIRENTRY: usize = 8;
const FILENAME_LEN: usize = 36;

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SuperBlockFlags: u64 {
        const READ_ONLY = 1;
        const DIRTY = 1 << 1;
        const IS_ROOT = 1 << 2;
        const NEEDS_CHECK = 1 << 3;
        const BACKUP_SYNC = 1 << 4;
    }

    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DirEntryFlags: u8 {
        /// If not a directory, it's a file
        const IS_DIR = 1;
        const IS_SYMLINK = 1 << 1;
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct SuperBlock {
    /// The `LemonFS` Magic
    pub magic: [u8; 8],
    pub version: u32,
    _reserved: [u8; 4], // Padding
    pub flags: SuperBlockFlags,
    /// CRC64 Checksum of the `SuperBlock`
    pub checksum: u64,
    /// Start of the bitmap
    pub bitmap_start: u64,
    /// Start of the data
    pub data_start: u64,
    /// End of usable data blocks
    pub data_end: u64,
    // Must be 0xBA
    _padding: [u8; 512 - 56]
}

impl SuperBlock {
    pub const fn new(part_block: u64) -> Option<Self> {
        if part_block < 0x1000 { return None }
        Some(Self {
            magic: LEMONFS_MAGIC,
            version: 1,
            _reserved: [0xBA; 4],
            flags: SuperBlockFlags::empty(),
            checksum: 0,
            bitmap_start: 1,
            data_start: 1 + part_block.div_ceil(0x1000),
            data_end: part_block - 2,
            _padding: [0xBA; 456]
        })
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BitmapBlock(pub [u8; 512]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Filename(pub [u8; FILENAME_LEN]);

impl Filename {
    pub fn from_str(s: &str) -> Self {
        let mut buf = [0; FILENAME_LEN];
        let n = s.len().min(FILENAME_LEN);
        buf[..n].copy_from_slice(&s.as_bytes()[..n]);

        Self(buf)
    }

    pub fn to_str(self) -> String {
        String::from_utf8_lossy_owned(self.0.to_vec().iter().filter_map(|&v| if v == 0 { None } else { Some(v) } ).collect())
    }

    pub fn is_empty(&self) -> bool { self.0.iter().all(|&v| v == 0) }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub name: Filename,
    pub flags: DirEntryFlags,
    pub link: LinkPointer
}

impl DirEntry {
    pub const fn new(name: &Filename, flags: DirEntryFlags, link: LinkPointer) -> Self {
        Self { name: *name, flags, link }
    }

    pub fn zero() -> Self {
        Self {
            name: Filename::from_str(""),
            flags: DirEntryFlags::empty(),
            link: LinkPointer::new(0, 0)
        }
    }
}

#[repr(C, packed)]
pub struct DirBlock {
    /// Parent's first dirblock
    pub parent: u64,
    pub entries: [DirEntry; ENTRIES_PER_DIRENTRY],
    pub next: LinkPointer,
    _padding: [u8; 64]
}

impl DirBlock {
    pub const fn new(parent: u64, entries: &[DirEntry; ENTRIES_PER_DIRENTRY], next: LinkPointer) -> Self {
        Self {
            parent,
            entries: *entries,
            next,
            _padding: [0xBA; 64]
        }
    }
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct LinkPointer {
    pub start: u64,
    pub size: u64
}

#[repr(C, packed)]
pub struct LinkBlock(pub [LinkPointer; 32]);

impl LemonData for SuperBlock {}
impl LemonData for BitmapBlock {}
impl LemonData for DirBlock {}
impl LemonData for LinkBlock {}