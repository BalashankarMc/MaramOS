//! LemonFS hash table operations.
//!
//! Implements FNV-1a hashing, open-addressing lookup with linear probing,
//! tombstone deletion, and rename via tombstone + re-insert. Also provides
//! convenience wrappers for name encoding/comparison.

use alloc::{string::String, vec::Vec};

use super::{
    FILENAME_LEN,
    FSError,
    headers::{*, HASHES_PER_BLOCK as HPB},
    helpers::{FileName, File, Directory},
    io
};

use crate::{drivers::storage::{StorageDrive, lbas_to_pages}, fs::FSReturn, memory::{KMemory, PAGE_SIZE}};

const HASHES_PER_BLOCK: u64 = HPB as u64;

// Name Handlers

pub fn name_to_hash(name: &[u8; 40]) -> u64 {
    let mut hash: u64 = 0x811C9DC5;
    for &b in name {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x1000193)
    }
    hash
}

pub fn name_to_bytes(name: &str) -> FSReturn<[u8; FILENAME_LEN]> {
    if name.is_empty() || name.len() > FILENAME_LEN { return Err(FSError::InvalidFilename) }

    let mut bytes = [0_u8; FILENAME_LEN];
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    Ok(bytes)
}

pub fn bytes_to_name(bytes: &[u8]) -> &str {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(FILENAME_LEN);
    core::str::from_utf8(&bytes[..len]).unwrap_or_default()
}

/// Convenience wrapper: hash a string filename (converts to byte array first).
pub fn hash_name(name: &str) -> u64 {
    let mut bytes = [0u8; FILENAME_LEN];
    let len = name.len().min(FILENAME_LEN);
    bytes[..len].copy_from_slice(&name.as_bytes()[..len]);
    name_to_hash(&bytes)
}

/// Convenience wrapper: convert bytes back to a String (owned).
pub fn name_to_str(bytes: &[u8]) -> String {
    String::from(bytes_to_name(bytes))
}

/// Compare a byte-array name against an expected string.
pub fn name_eq(bytes: &[u8], expected: &str) -> bool {
    bytes_to_name(bytes) == expected
}

fn find_free_slot(drive: &mut dyn StorageDrive, name: &FileName) -> FSReturn<HashPointer> {
    let sb: SuperBlock = io::read_metadata(drive, 0)?;
    let hashes = sb.hashmap_size;
    let hash = name_to_hash(&name.0);

    let start = HashPointer::new(hash % hashes);
    let hash_blocks = hashes / HASHES_PER_BLOCK;
    for i in 0..hashes {
        let ptr = match start.add(hash_blocks, i) {
            Some(p) => p,
            None => break
        };

        let status = ptr.read(drive)?.status;
        if status == HashStatus::Unused || status == HashStatus::Tombstone { return Ok(ptr) }
    }

    Err(FSError::DiskFull)
}

fn scan(drive: &mut dyn StorageDrive, name: &FileName) -> FSReturn<(HashPointer, u64)> {
    let sb: SuperBlock = io::read_metadata(drive, 0)?;
    let hashes = sb.hashmap_size;
    let hash_blocks = hashes / HASHES_PER_BLOCK;
    let hash = name_to_hash(&name.0);

    // let start = hash % hashes;
    // let mut hash_start = None;
    // let mut n = 0;

    // for i in 0..hashes {
    //     let ptr = HashPointer::new((start + i) % hashes);
    //     let entry = ptr.read(drive)?;
    //     let status = entry.status;
    //     if status == HashStatus::Unused { break }
    //     if status == HashStatus::Tombstone { continue }
    //     if entry.name != *name { continue }
    //     if hash_start.is_none() { hash_start = Some(ptr) }
    //     if hash_start.is_some() { n += 1 }

    // }
    // if let Some(hash) = hash_start {
    //     return Ok((hash, n))
    // }

    let start = HashPointer::new(hash % hashes);
    let mut hash_start = None;

    let mut n = 0;

    for i in 0..hashes {
        let ptr = match start.add(hash_blocks, i) {
            Some(p) => p,
            None => break
        };

        let entry = ptr.read(drive)?;
        if entry.status == HashStatus::Unused { break }
        if entry.status == HashStatus::Tombstone { continue }
        if name_to_hash(&entry.name) != hash { break }
        if entry.name != name.0 { continue }

        if hash_start.is_none() { hash_start = Some(ptr) }
        else { n += 1 }
    }

    if let Some(hstart) = hash_start { return Ok((hstart, n)) }

    Err(FSError::FileNotFound)
}

pub fn new_file_hash(drive: &mut dyn StorageDrive, file: &File) -> FSReturn<HashPointer> {
    let mut slot = find_free_slot(drive, &file.name)?;

    let hash = HashEntry::new(FileType::File, file.name.0, file.start, file.size);
    slot.write(drive, hash)?;

    Ok(slot)
}

pub fn new_directory_hash(drive: &mut dyn StorageDrive, dir: &Directory) -> FSReturn<HashPointer> {
    let mut slot = find_free_slot(drive, &dir.name)?;

    let hash = HashEntry::new(FileType::Directory, dir.name.0, dir.start, 0);
    slot.write(drive, hash)?;

    Ok(slot)
}

pub fn find(drive: &mut dyn StorageDrive, name: &FileName, type_: FileType) -> FSReturn<Vec<HashPointer>> {
    let (start, count) = scan(drive, name)?;
    let mut res = Vec::with_capacity(count as usize + 1);
    res.push(start);
    for i in 1..=count {
        res.push(start.add(u64::MAX, i).unwrap()); // The count bound is enough. We dont need adds check
    }

    let mut indexes = Vec::with_capacity(count as usize);
    for (i, ptr) in res.iter().enumerate() {
        let hash = ptr.read(drive)?;
        if hash.type_ != type_|| hash.status != HashStatus::Used { indexes.push(i); }
    }

    indexes.reverse();

    for index in indexes { res.remove(index); }
    
    if res.is_empty() { return Err(FSError::FileNotFound) }

    Ok(res)
}

pub fn tombstone(drive: &mut dyn StorageDrive, mut ptr: HashPointer) -> FSReturn<()> {
    let mut entry = ptr.read(drive)?;
    entry.status = HashStatus::Tombstone;
    ptr.write(drive, entry)
}

pub fn rename_hash(drive: &mut dyn StorageDrive, mut old_ptr: HashPointer, new_name: &FileName) -> FSReturn<HashPointer> {
    let mut old_entry = old_ptr.read(drive)?;

    old_entry.status = HashStatus::Tombstone;
    old_ptr.write(drive, old_entry)?;

    let mut new_slot = find_free_slot(drive, new_name)?;
    let new_entry = HashEntry {
        status: HashStatus::Used,
        type_: old_entry.type_,
        name: new_name.0,
        start_block: old_entry.start_block,
        file_size: old_entry.file_size,
        link: old_entry.link,
    };
    new_slot.write(drive, new_entry)?;
    Ok(new_slot)
}