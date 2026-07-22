//! LemonFS path resolution.
//!
//! Splits slash-separated paths, walks the directory tree from root, and
//! resolves both full paths and parent directories to [`HashPointer`]
//! values.

use super::{headers::*, helpers::*, io};
use super::super::{FSError, FSReturn};
use crate::drivers::storage::StorageDrive;
use crate::fs::lemonfs::hash;

pub fn resolve(drive: &mut dyn StorageDrive, path: &Path) -> FSReturn<HashPointer> {
    let path_full = path.to_dir_vec();
    let root_ptr = *hash::find(drive, &FileName::ROOT, FileType::Directory)
        .map_err(|_| FSError::NoRoot)?.first().ok_or(FSError::NoRoot)?;

    if path_full.len() <= 1 { return Ok(root_ptr) }

    let mut current = Directory::from_ptr(root_ptr, drive)?;

    for &dir in &path_full[1..] {
        let &child_ptr = current.children.iter().find(|&ptr| {
            if ptr.lba == 0 { return false }
            let e = match ptr.read(drive) {
                Ok(e) => e,
                Err(_) => return false
            };
            
            e.status == HashStatus::Used && super::hash::bytes_to_name(&e.name) == dir
        } ).ok_or(FSError::FileNotFound)?;

        if &dir != path_full.last().unwrap() {
            let entry = child_ptr.read(drive)?;
            if entry.type_ != FileType::Directory { return Err(FSError::IsFile) }
            current = Directory::from_ptr(child_ptr, drive)?;
        } else {
            return Ok(child_ptr);
        }
    }
    unreachable!()
}

pub fn resolve_parent(drive: &mut dyn StorageDrive, path: &Path) -> FSReturn<HashPointer> {
    resolve(drive, &path.parent())
}