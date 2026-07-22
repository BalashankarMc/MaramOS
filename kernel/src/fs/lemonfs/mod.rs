//! Implements the LemonFS Filesystem

#![allow(dead_code, unused_imports)] // Remove later

use alloc::{boxed::Box, string::{String, ToString}, vec::Vec, vec};

use super::{FSReturn, FSError, FileSystem};
use crate::{
    drivers::storage::{LBA_SIZE, StorageDrive, lbas_to_pages},
    memory::{KMemory, PAGE_SIZE, PhysPage}
};

mod pointers;
pub mod headers;
mod helpers;
mod bitmap;
pub mod hash;
mod path;
mod io;

use headers::*;
use helpers::*;

const MAGIC: u64 = u64::from_le_bytes(*b"LEMONFS\0");
const FILENAME_LEN: usize = 40;

pub struct LemonFS {
    drive: Box<dyn StorageDrive>,
    superblock: SuperBlock
}

impl LemonFS {
    fn format(drive: &mut dyn StorageDrive) -> FSReturn<SuperBlock> {
        let sb = SuperBlock::new(drive);
        io::write_metadata(drive, 0, sb)?;

        let lbas = sb.data_start();
        drive.zero_lbas(1, lbas).map_err(|_| FSError::IO)?;

        let allocated_lba = bitmap::alloc_range(drive, sb.data_start(), 1)?;
        if allocated_lba != sb.data_start() { return Err(FSError::InitFailed) }

        let root_entry = Directory {
            name: FileName::ROOT,
            hash: HashPointer { lba: 0, offset: 0 },
            start: sb.data_start(),
            children: Vec::new()
        };

        let _root_ptr = hash::new_directory_hash(drive, &root_entry)?;
        Ok(sb)
    }
}

impl FileSystem for LemonFS {
    fn init(mut drive: Box<dyn StorageDrive>) -> FSReturn<Self> where Self: Sized {
        let mut sb: SuperBlock = io::read_metadata(drive.as_mut(), 0)?;
        let magic = sb.magic;

        if magic != u64::from_le_bytes(*b"LEMONFS\0") {
            sb = Self::format(&mut *drive)?
        }
        if sb.version != 2 { return Err(FSError::BadVersion) }

        Ok(Self {
            drive,
            superblock: sb
        })
    }

    fn new_file(&mut self, path: &str, size: u64) -> FSReturn<()> {
        if path.is_empty() || path == "/" { return Err(FSError::IsRoot) }
        let path = Path::from_str(path);
        let name = FileName::from_str(path.file_name()).map_err(|_| FSError::InvalidFilename)?;

        let drive = self.drive.as_mut();

        let parent_ptr = path::resolve_parent(drive, &path)?;
        let mut parent_dir = Directory::from_ptr(parent_ptr, drive)?;

        for child in &parent_dir.children {
            if child.read(drive)?.name == name.0 { return Err(FSError::FileExists) }
        }

        let data_start = bitmap::bitmap_alloc_first_fit(drive, size.div_ceil(LBA_SIZE))?;

        let file = File { name, start: data_start, size };
        let hash = hash::new_file_hash(drive, &file)?;

        parent_dir.add_child(drive, hash)
    }

    fn new_directory(&mut self, path: &str) -> FSReturn<()> {
        if path.is_empty() || path == "/" { return Err(FSError::IsRoot) }
        let path = Path::from_str(path);
        let name = FileName::from_str(path.file_name()).map_err(|_| FSError::InvalidFilename)?;

        let drive = self.drive.as_mut();

        let parent_ptr = path::resolve_parent(drive, &path)?;
        let mut parent_dir = Directory::from_ptr(parent_ptr, drive)?;

        for child in &parent_dir.children {
            if child.read(drive)?.name == name.0 { return Err(FSError::FileExists) }
        }

        let data_start = bitmap::bitmap_alloc_first_fit(drive, 1)?;

        let dir = Directory { name, start: data_start, children: Vec::new(), hash: HashPointer { lba: 0, offset: 0 } };
        let hash = hash::new_directory_hash(drive, &dir)?;

        parent_dir.add_child(drive, hash)
    }

    fn read_file(&mut self, path: &str, dest: &mut PhysPage) -> FSReturn<()> {
        if path.is_empty() || path == "/" { return Err(FSError::IsRoot) }
        let path = Path::from_str(path);
        let name = FileName::from_str(path.file_name()).map_err(|_| FSError::InvalidFilename)?;
        
        let drive = self.drive.as_mut();

        let parent_ptr = path::resolve_parent(drive, &path)?;
        let parent_dir = Directory::from_ptr(parent_ptr, drive)?;

        let mut file_ptr = None;
        for entry in &parent_dir.children {
            if entry.read(drive)?.name == name.0 { file_ptr = Some(*entry) }
        }

        if file_ptr.is_none() { return Err(FSError::FileNotFound) }
        let file_hash = file_ptr.unwrap().read(drive)?;
        if file_hash.type_ != FileType::File { return Err(FSError::IsDirectory) }

        let safe_len = (dest.size() * PAGE_SIZE).min(file_hash.file_size as usize);

        let mut page = KMemory::alloc_pages((file_hash.file_size as usize).div_ceil(PAGE_SIZE));
        let lbas_direct = file_hash.file_size.div_ceil(LBA_SIZE);
        drive.read_lbas(file_hash.start_block, lbas_direct, &mut page).map_err(|_| FSError::IO)?;
        let src = page.get_virt_addr().as_ptr::<u8>();
        let dst = dest.get_virt_addr().as_mut_ptr::<u8>();
        unsafe { core::ptr::copy_nonoverlapping(src, dst, safe_len) }
        if file_hash.file_size >= (dest.size() * PAGE_SIZE) as u64 { return Ok(()) }

        let mut stack = Vec::new();
        stack.push(file_hash.link);

        let mut offset = file_hash.file_size;

        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                let pages = entry.size.div_ceil(PAGE_SIZE as u64) as usize;
                let mut page = KMemory::alloc_pages(pages);
                let lbas = entry.size.div_ceil(LBA_SIZE);

                drive.read_lbas(entry.ptr, lbas, &mut page).map_err(|_| FSError::IO)?;

                let src = page.get_virt_addr().as_ptr::<u8>();
                let dst = (dest.get_virt_addr() + offset).as_mut_ptr::<u8>();

                let safe_len = ((dest.size() * PAGE_SIZE) - offset as usize).min(file_hash.file_size as usize);

                unsafe { core::ptr::copy_nonoverlapping(src, dst, safe_len) }
                offset += entry.size;
                if offset >= (dest.size() * PAGE_SIZE) as u64 { return Ok(()) }
            } else {
                let linkblock: LinkBlock = io::read_metadata(drive, entry.ptr)?;
                for link in linkblock.entries { stack.push(link) }
            }
        }
        Ok(())
    }

    fn write_file(&mut self, path: &str, src: &PhysPage) -> FSReturn<()> {
        if path.is_empty() || path == "/" { return Err(FSError::IsRoot) }
        let path = Path::from_str(path);
        let name = FileName::from_str(path.file_name()).map_err(|_| FSError::InvalidFilename)?;

        let drive = self.drive.as_mut();

        let parent_ptr = path::resolve_parent(drive, &path)?;
        let parent_dir = Directory::from_ptr(parent_ptr, drive)?;

        let mut file_ptr = None;
        for entry in &parent_dir.children {
            if entry.read(drive)?.name == name.0 { file_ptr = Some(*entry) }
        }

        let mut file_ptr = file_ptr.ok_or(FSError::FileNotFound)?;
        let mut file_hash = file_ptr.read(drive)?;
        if file_hash.type_ != FileType::File { return Err(FSError::IsDirectory) }

        let src_base = src.get_virt_addr().as_ptr::<u8>();
        let max_bytes = (src.size() * PAGE_SIZE) as u64;
        let mut new_size = max_bytes;
        unsafe {
            for i in 0..max_bytes {
                if *src_base.add(i as usize) == 0x04 {
                    new_size = i;
                    break;
                }
            }
        }

        let old_size = file_hash.file_size;
        let old_lbas = old_size.div_ceil(LBA_SIZE);
        let data_start = file_hash.start_block;

        if new_size <= old_size {
            pointers::free_link_chain(drive, file_hash.link)?;
            file_hash.link = LinkEntry { ptr: 0, size: 0 };

            let new_lbas = new_size.div_ceil(LBA_SIZE);
            drive.write_lbas(data_start, new_lbas, src).map_err(|_| FSError::IO)?;

            if new_lbas < old_lbas {
                bitmap::free_range(drive, data_start + new_lbas, old_lbas - new_lbas)?;
            }

            file_hash.file_size = new_size;
        } else {
            let new_lbas = new_size.div_ceil(LBA_SIZE);

            if let Ok(new_start) = bitmap::bitmap_alloc_first_fit(drive, new_lbas) {
                pointers::free_link_chain(drive, file_hash.link)?;
                bitmap::free_range(drive, data_start, old_lbas)?;

                drive.write_lbas(new_start, new_lbas, src).map_err(|_| FSError::IO)?;

                file_hash.start_block = new_start;
                file_hash.file_size = new_size;
                file_hash.link = LinkEntry { ptr: 0, size: 0 };
            } else {
                drive.write_lbas(data_start, old_lbas, src).map_err(|_| FSError::IO)?;

                let overflow_size = new_size - old_size;
                let overflow_lbas = overflow_size.div_ceil(LBA_SIZE);
                let overflow_start = bitmap::bitmap_alloc_first_fit(drive, overflow_lbas)?;

                let pages = lbas_to_pages(overflow_lbas);
                let temp = KMemory::alloc_pages(pages);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        (src.get_virt_addr() + old_size).as_ptr::<u8>(),
                        temp.get_virt_addr().as_mut_ptr::<u8>(),
                        overflow_size as usize,
                    );
                }
                drive.write_lbas(overflow_start, overflow_lbas, &temp).map_err(|_| FSError::IO)?;

                pointers::extend_link(drive, &mut file_hash.link, LinkEntry { ptr: overflow_start, size: overflow_size })?;

                file_hash.file_size = new_size;
            }
        }

        file_ptr.write(drive, file_hash)
    }

    fn file_size(&mut self, path: &str) -> FSReturn<u64> {
        if path.is_empty() || path == "/" { return Err(FSError::IsRoot) }
        let path = Path::from_str(path);
        let name = FileName::from_str(path.file_name()).map_err(|_| FSError::InvalidFilename)?;

        let drive = self.drive.as_mut();

        let parent_ptr = path::resolve_parent(drive, &path)?;
        let parent_dir = Directory::from_ptr(parent_ptr, drive)?;

        let mut file_ptr = None;
        for entry in &parent_dir.children {
            if entry.read(drive)?.name == name.0 { file_ptr = Some(*entry) }
        }

        let file_ptr = file_ptr.ok_or(FSError::FileNotFound)?;
        let file_hash = file_ptr.read(drive)?;
        if file_hash.type_ != FileType::File { return Err(FSError::IsDirectory) }

        let mut size = file_hash.file_size;
        let mut stack = vec![file_hash.link];
        while let Some(link) = stack.pop() {
            if !link.is_valid() { continue }
            if link.is_direct() { size += link.size }
            else {
                let link_block: LinkBlock = io::read_metadata(drive, link.ptr)?;
                for l in link_block.entries { stack.push(l) }
            }
        }

        Ok(size)
    }

    fn list_directory(&mut self, path: &str) -> FSReturn<Vec<String>> {
        let path = Path::from_str(path);
        let drive = self.drive.as_mut();
        let dir_ptr = path::resolve(drive, &path)?;
        let dir = Directory::from_ptr(dir_ptr, drive)?;
        let mut res = Vec::new();
        for child in dir.children {
            res.push(hash::bytes_to_name(&child.read(drive)?.name).to_string());
        }
        Ok(res)
    }

    fn move_(&mut self, src: &str, dest: &str) -> FSReturn<()> {
        if src.is_empty() || src == "/" { return Err(FSError::IsRoot) }
        let src_path = Path::from_str(src);
        let dest_path = Path::from_str(dest);
        let drive = self.drive.as_mut();

        let src_ptr = path::resolve(drive, &src_path)?;
        let src_entry = src_ptr.read(drive)?;
        let src_name = src_path.file_name();
        let src_parent_ptr = path::resolve_parent(drive, &src_path)?;

        let dest_dir_ptr;
        let new_name;

        if let Ok(dest_ptr) = path::resolve(drive, &dest_path) {
            let dest_entry = dest_ptr.read(drive)?;
            if dest_entry.type_ != FileType::Directory {
                return Err(FSError::FileExists);
            }
            dest_dir_ptr = dest_ptr;
            new_name = src_name;
        } else {
            let parent_ptr = path::resolve_parent(drive, &dest_path)?;
            dest_dir_ptr = parent_ptr;
            new_name = dest_path.file_name();
        }

        if src_parent_ptr.lba == dest_dir_ptr.lba
            && src_parent_ptr.offset == dest_dir_ptr.offset
            && new_name == src_name
        {
            return Ok(());
        }

        if src_entry.type_ == FileType::Directory {
            if dest_dir_ptr.lba == src_ptr.lba && dest_dir_ptr.offset == src_ptr.offset {
                return Err(FSError::InvalidAccess);
            }
            let src_prefix = alloc::format!("{}/", src);
            if dest_path.as_str().starts_with(&src_prefix) {
                return Err(FSError::InvalidAccess);
            }
        }

        let mut src_parent = Directory::from_ptr(src_parent_ptr, drive)?;
        src_parent.remove_child(drive, src_ptr)?;

        let new_name_obj = FileName::from_str(new_name).map_err(|_| FSError::InvalidFilename)?;
        if new_name_obj.0 != src_entry.name {
            let new_ptr = hash::rename_hash(drive, src_ptr, &new_name_obj)?;
            let mut dest_dir = Directory::from_ptr(dest_dir_ptr, drive)?;
            dest_dir.add_child(drive, new_ptr)?;
        } else {
            let mut dest_dir = Directory::from_ptr(dest_dir_ptr, drive)?;
            dest_dir.add_child(drive, src_ptr)?;
        }

        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> FSReturn<()> {
        if path.is_empty() || path == "/" { return Err(FSError::IsRoot) }
        let path = Path::from_str(path);
        let name = FileName::from_str(path.file_name()).map_err(|_| FSError::InvalidFilename)?;
        let drive = self.drive.as_mut();

        let parent_ptr = path::resolve_parent(drive, &path)?;
        let mut parent_dir = Directory::from_ptr(parent_ptr, drive)?;

        let mut file_ptr = None;
        for entry in &parent_dir.children {
            if entry.read(drive)?.name == name.0 { file_ptr = Some(*entry) }
        }

        let file_ptr = file_ptr.ok_or(FSError::FileNotFound)?;
        let file_hash = file_ptr.read(drive)?;
        if file_hash.type_ != FileType::File { return Err(FSError::IsDirectory) }

        parent_dir.remove_child(drive, file_ptr)?;

        let data_lbas = file_hash.file_size.div_ceil(LBA_SIZE);
        bitmap::free_range(drive, file_hash.start_block, data_lbas)?;

        pointers::free_link_chain(drive, file_hash.link)?;

        hash::tombstone(drive, file_ptr)
    }

    fn remove_directory(&mut self, path: &str) -> FSReturn<()> {
        if path.is_empty() || path == "/" { return Err(FSError::IsRoot) }
        let path = Path::from_str(path);
        let name = FileName::from_str(path.file_name()).map_err(|_| FSError::InvalidFilename)?;
        let drive = self.drive.as_mut();

        let parent_ptr = path::resolve_parent(drive, &path)?;
        let mut parent_dir = Directory::from_ptr(parent_ptr, drive)?;

        let mut dir_ptr = None;
        for entry in &parent_dir.children {
            if entry.read(drive)?.name == name.0 { dir_ptr = Some(*entry) }
        }

        let dir_ptr = dir_ptr.ok_or(FSError::FileNotFound)?;
        let dir_hash = dir_ptr.read(drive)?;
        if dir_hash.type_ != FileType::Directory { return Err(FSError::IsFile) }

        let dir = Directory::from_ptr(dir_ptr, drive)?;
        if !dir.children.is_empty() { return Err(FSError::FileExists) }

        parent_dir.remove_child(drive, dir_ptr)?;

        bitmap::free_range(drive, dir_hash.start_block, 1)?;

        pointers::free_link_chain(drive, dir_hash.link)?;

        hash::tombstone(drive, dir_ptr)
    }
}
