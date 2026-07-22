//! LemonFS helper types for files and directories.
//!
//! Provides fixed-size [`FileName`], path string wrappers, and [`File`]
//! / [`Directory`] abstractions that traverse both direct and linked
//! hash-entry blocks.

use alloc::{string::String, vec::Vec};
use crate::drivers::storage::{LBA_SIZE, StorageDrive};
use crate::fs::{FSError, FSReturn};

use super::{
    headers::*,
    hash::bytes_to_name,
    bitmap::bitmap_alloc_first_fit,
    io,
    pointers::extend_link,
    FILENAME_LEN
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileName(pub [u8; FILENAME_LEN]);

impl FileName {
    pub fn from_str(s: &str) -> FSReturn<Self> {
        if s.is_empty() || s.len() > FILENAME_LEN { return Err(FSError::InvalidFilename) }
        let mut bytes = [0u8; FILENAME_LEN];
        bytes[..s.len()].copy_from_slice(s.as_bytes());
        Ok(Self(bytes))
    }

    pub fn inner(&self) -> &str {
        let len = self.0.iter().position(|&b| b == 0).unwrap_or(FILENAME_LEN);
        core::str::from_utf8(&self.0[..len]).unwrap_or_default()
    }

    pub const ROOT: Self = {
        let mut res = [0; FILENAME_LEN];
        res[0] = b'/';
        Self(res)
    };
}

pub struct Path(String);

impl Path {
    pub fn from_str(s: &str) -> Self { Path(s.into()) }

    pub fn to_dir_vec(&self) -> Vec<&str> {
        self.0.split('/').collect()
    }

    pub fn parent(&self) -> Path {
        let mut parts = self.0.rsplitn(2, "/");
        let _file = parts.next();
        Path(parts.next().unwrap_or("/").into())
    }

    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct File {
    pub name: FileName,
    pub start: u64,
    pub size: u64,
}

pub struct Directory {
    pub name: FileName,
    pub hash: HashPointer,
    pub start: u64,
    pub children: Vec<HashPointer>,
}

pub fn find_free_dentry_slot(dentry: &DirectoryEntry) -> Option<usize> {
    dentry.entries.iter().position(|e| !e.is_valid())
}

impl Directory {
    pub fn from_ptr(ptr: HashPointer, drive: &mut dyn StorageDrive) -> FSReturn<Self> {
        // Get hash entry
        let hash_entry = ptr.read(drive)?;
        if hash_entry.type_ != FileType::Directory { return Err(FSError::IsFile) }

        let lba = hash_entry.start_block;
        let dentry: DirectoryEntry = io::read_metadata(drive, lba)?;

        // Resolve Direct children
        let mut children = Vec::new();
        for entry in dentry.entries { if entry.is_valid() { children.push(entry) } }

        // Resolve Linked children
        let mut stack = Vec::new();
        stack.push(hash_entry.link);

        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if !entry.is_direct() {
                let link_block: LinkBlock = io::read_metadata(drive, entry.ptr)?;
                for entry in link_block.entries { stack.push(entry) }
                continue
            }

            let dentry: DirectoryEntry = io::read_metadata(drive, entry.ptr)?;
            for entry in dentry.entries { if entry.is_valid() { children.push(entry) } }
        }

        Ok(Self {
            name: FileName(hash_entry.name),
            hash: ptr,
            start: hash_entry.start_block,
            children
        })

    }

    pub fn add_child(&mut self, drive: &mut dyn StorageDrive, child: HashPointer) -> FSReturn<()> {
        self.children.push(child); // Add to Vec

        // Now write to disk
        let hash = self.hash.read(drive)?;

        // Check for a direct slot
        let data_start = hash.start_block;
        let mut dentry: DirectoryEntry = io::read_metadata(drive, data_start)?;

        if let Some(idx) = find_free_dentry_slot(&dentry) {
            dentry.entries[idx] = child;
            io::write_metadata(drive, data_start, dentry)?;
            return Ok(())
        }

        // Check for indirect slot
        let mut stack = Vec::new();
        stack.push(hash.link);
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if !entry.is_direct() {
                let link_block: LinkBlock = io::read_metadata(drive, entry.ptr)?;
                for entry in link_block.entries { stack.push(entry) }
                continue
            }

            let mut dentry: DirectoryEntry = io::read_metadata(drive, entry.ptr)?;
            if let Some(idx) = find_free_dentry_slot(&dentry) {
                dentry.entries[idx] = child;
                io::write_metadata(drive, entry.ptr, dentry)?;
                return Ok(())
            }
        }

        // Create new link and allocate
        let new_lba = bitmap_alloc_first_fit(drive, 1)?;
        let mut dentry_new = DirectoryEntry::new();
        dentry_new.entries[0] = child;
        io::write_metadata(drive, new_lba, dentry_new)?;

        let mut hash = self.hash.read(drive)?;
        extend_link(drive, &mut hash.link, LinkEntry { ptr: new_lba, size: LBA_SIZE })?;
        self.hash.write(drive, hash)
    }

    pub fn remove_child(&mut self, drive: &mut dyn StorageDrive, child_ptr: HashPointer) -> FSReturn<()> {
        self.children.retain(|c| c.lba != child_ptr.lba || c.offset != child_ptr.offset);

        let hash = self.hash.read(drive)?;
        let data_start = hash.start_block;
        let mut dentry: DirectoryEntry = io::read_metadata(drive, data_start)?;

        for slot in dentry.entries.iter_mut() {
            if slot.lba == child_ptr.lba && slot.offset == child_ptr.offset {
                *slot = HashPointer { lba: 0, offset: 0 };
                return io::write_metadata(drive, data_start, dentry);
            }
        }

        let mut stack = Vec::new();
        stack.push(hash.link);
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                let mut dentry: DirectoryEntry = io::read_metadata(drive, entry.ptr)?;
                for slot in dentry.entries.iter_mut() {
                    if slot.lba == child_ptr.lba && slot.offset == child_ptr.offset {
                        *slot = HashPointer { lba: 0, offset: 0 };
                        return io::write_metadata(drive, entry.ptr, dentry);
                    }
                }
            } else {
                let linkblock: LinkBlock = io::read_metadata(drive, entry.ptr)?;
                for e in linkblock.entries { stack.push(e) }
            }
        }

        Err(FSError::FileNotFound)
    }
}