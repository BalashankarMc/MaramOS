//! Implements `LemonFS` for `MaramOS`

use alloc::{boxed::Box, string::String, vec::Vec};
use crate::{drivers::storage::Guid, memory::{PAGE_SIZE, PhysPage}};
use super::{FSError, FileSystem, lba_abstractor::PartitionDrive};

use bitmap::BitmapAllocator;
use directory::{Directory, DirectoryEntry, FileType};
use disk::{BitmapBlock, DirBlock, DirEntryFlags, Filename, LinkBlock, LinkPointer, SuperBlock};
use path::Path;

type Disk<'a> = &'a PartitionDrive;
type FSResult<T> = Result<T, FSError>;

mod io;
mod path;
mod disk;
mod file;
mod links;
mod bitmap;
mod directory;
mod superblock;

pub const LEMONFS_GUID: Guid = Guid::new(0x8CBD_0D4E, 0xFE62, 0x49CC, 0xA982, 0xFBD6_F69E_D888);
pub const LEMONFS_MAGIC: [u8; 8] = *b"LEMONFS\0";

trait LemonData {}

#[derive(Debug)]
pub struct LemonFS {
    drive: PartitionDrive,

    // Caches
    superblock: SuperBlock,
    bitmap: BitmapAllocator,
    root: Vec<DirectoryEntry>
}

impl LemonFS {
    const fn root_ptr(&self) -> LinkPointer {
        LinkPointer { start: self.superblock.data_end, size: 512 }
    }

    fn root_dir(&self) -> Directory {
        Directory { name: Filename::from_str("/"), children: self.root.clone(), parent: 0, head: self.root_ptr() }
    }

    fn get_parent(&self, path: &Path) -> Result<Directory, FSError> {
        let parent_path = path.parent();

        let mut curr_parent = self.root_dir();
        for component in parent_path.components() {
            let dentry = curr_parent.get_child(&component.into()).ok_or(FSError::FileNotFound)?;

            curr_parent = match dentry {
                DirectoryEntry::File(_) => return Err(FSError::BadFileType),
                DirectoryEntry::SymLink { name: _, dest } => {
                    let block: DirBlock = dest.read(&self.drive)?;
                    Directory::from_dir_block(
                        &self.drive,
                        &block,
                        Filename::from_str(component),
                        *dest
                    )?
                },

                DirectoryEntry::Directory { name: _, link: _ } => Directory::from_directory_entry(&self.drive, dentry)?
            }
        }

        Ok(curr_parent)
    }

    fn build_file_chain(&mut self, blocks: &[u64]) -> FSResult<LinkPointer> {
        // Coalesce adjacent extents
        let mut extents: Vec<LinkPointer> = Vec::new();
        for &block in blocks {
            match extents.last_mut() {
                Some(p) if p.start + p.size / 512 == block => p.size += 512,
                _ => extents.push(LinkPointer::new(block, 512)),
            }
        }

        if extents.len() == 1 { return Ok(extents[0]) }
        let needed_blocks = extents.len().div_ceil(31);
        let mut lbas = Vec::new();
        for _ in 0..needed_blocks { lbas.push(self.bitmap.alloc(1).ok_or(FSError::NoSpace)?); }

        for i in 0..needed_blocks {
            let mut arr = [LinkPointer::new(0, 0); 32];
            let base = i * 31;
            let end = (base + 31).min(extents.len());

            arr[..end - base].copy_from_slice(&extents[base..end]);
            arr[31] = if i + 1 < needed_blocks { LinkPointer::new(lbas[i + 1], 0) } else { LinkPointer::new(0, 0) };

            io::write_data(&self.drive, lbas[i], LinkBlock(arr))?;
        }

        Ok(LinkPointer { start: lbas[0], size: 0 })
    }
}

impl FileSystem for LemonFS {
    fn create(&mut self, path: String, flags: u64) -> Result<(), FSError> {
        let path = Path::from_string(path);
        if path.is_root() { return Err(FSError::IsRoot) }
        let mut parent = self.get_parent(&path)?;

        let file_exists = parent.find_child_idx(&path.name());
        if file_exists.is_some() { return Err(FSError::FileExists) }

        let filetype = FileType::from_dirflags(DirEntryFlags::from_bits(flags as u8).ok_or(FSError::BadFlags)?);

        let start = match filetype {
            FileType::Directory | FileType::File => self.bitmap.alloc(1).ok_or(FSError::NoSpace)?,
            FileType::Symlink => 0
        };

        let dir_entry = DirectoryEntry::new(Filename::from_str(&path.name()), start, &filetype);
        parent.give_child(&self.drive, &mut self.bitmap, dir_entry.clone())?;

        // Update root cache (If necessary)
        if path.parent().is_root() { self.root.push(dir_entry) }

        Ok(())
    }

    fn delete(&mut self, path: String) -> Result<(), FSError> {
        let path = Path::from_string(path);
        if path.is_root() { return Err(FSError::IsRoot) }
        let mut parent = self.get_parent(&path)?;
        let file_idx = parent.find_child_idx(&path.name()).ok_or(FSError::FileNotFound)?;

        let file = parent.children.swap_remove(file_idx);
        match file {
            DirectoryEntry::File(f) => f.delete(&self.drive, &mut self.bitmap)?,
            DirectoryEntry::Directory { name: _, link: _ } => {
                let dir = Directory::from_directory_entry(&self.drive, &file)?;
                dir.delete(&self.drive, &mut self.bitmap)?;
            },

            DirectoryEntry::SymLink { name: _, dest: _ } => () // Do nothing, removing the directory entry is enough
        }
        
        parent.sync(&self.drive, &mut self.bitmap)?;

        // Update root cache if necessary
        if path.parent().is_root() {
            let idx = self.root.iter().position(|e| e.name() == path.name()).ok_or(FSError::FileNotFound)?;
            self.root.swap_remove(idx);
        }

        Ok(())
    }

    fn size(&mut self, path: String) -> Result<u64, FSError> {
        let path = Path::from_string(path);
        if path.is_root() { return Err(FSError::IsRoot) }

        let parent = self.get_parent(&path)?;
        let dentry = parent.get_child(&path.name()).ok_or(FSError::FileNotFound)?;
        let DirectoryEntry::File(file) = dentry else { return Err(FSError::BadFileType) };

        Ok(file.size)
    }

    fn list(&mut self, path: String) -> Result<Vec<String>, FSError> {
        let path = Path::from_string(path);

        if path.is_root() { // Use cache
            return Ok(self.root.iter().map(DirectoryEntry::name).collect())
        }

        let parent = self.get_parent(&path)?;
        let dentry_pos = parent.find_child_idx(&path.name()).ok_or(FSError::FileNotFound)?;
        let dentry = &parent.children[dentry_pos];

        // let dir = Directory::from_directory_entry(&self.drive, dentry)?;
        let dir = match dentry {
            DirectoryEntry::File(_) => return Err(FSError::BadFileType),
            DirectoryEntry::SymLink { name, dest } => {
                let block: DirBlock = dest.read(&self.drive)?;
                Directory::from_dir_block(&self.drive, &block, *name, *dest)?
            },

            DirectoryEntry::Directory { name: _, link: _ } => {
                Directory::from_directory_entry(&self.drive, dentry)?
            }
        };

        Ok(dir.children.iter().map(DirectoryEntry::name).collect())
    }

    fn read(&mut self, path: String, dest_page: &crate::memory::PhysPage) -> Result<u64, FSError> {
        let path = Path::from_string(path);
        if path.is_root() { return Err(FSError::IsRoot) }

        let parent = self.get_parent(&path)?;
        let dentry = parent.get_child(&path.name()).ok_or(FSError::FileNotFound)?;

        let links = match dentry {
            DirectoryEntry::Directory { name: _, link: _ } => return Err(FSError::BadFileType),
            DirectoryEntry::File(f) => f.links.clone(),
            DirectoryEntry::SymLink { name: _, dest } => {
                links::resolve_links(&self.drive, *dest)?
            }
        };

        let size = links.iter().map(|l| l.size).sum::<u64>();
        let read_size = size.min((dest_page.count * PAGE_SIZE) as u64);
        let mut offset = 0;

        for link in links {
            if offset >= read_size { break }

            let page = self.drive.read_sectors(link.start, link.size.div_ceil(512)).map_err(|_| FSError::IOError)?;
            let bytes_to_read = (read_size - offset).min(link.size) as usize;

            let src = page.address().as_ptr::<u8>();
            let dst = (dest_page.address() + offset).as_mut_ptr::<u8>();

            // Safety: Just a memcopy into known good regions. Safe.
            unsafe { core::ptr::copy(src, dst, bytes_to_read) };
            offset += bytes_to_read as u64;
        }

        Ok(read_size)
    }

    fn write(&mut self, path: String, src: &crate::memory::PhysPage) -> Result<(), FSError> {
        let path = Path::from_string(path);
        if path.is_root() { return Err(FSError::IsRoot) }

        let mut parent = self.get_parent(&path)?;
        let dentry_index = parent.find_child_idx(&path.name()).ok_or(FSError::FileNotFound)?;
        let dentry = &mut parent.children[dentry_index];

        let DirectoryEntry::File(file) = dentry else { return Err(FSError::BadFileType) };
        let head = file.head;

        let mut blocks = Vec::new();
        let (direct, indirect) = links::trace(&self.drive, head)?;

        // Remove all linkblocks
        for link in &indirect {
            self.bitmap.free(link.start, 1);
            self.drive.zero_sectors(link.start, 1).map_err(|_| FSError::IOError)?;
        }

        for link in &direct {
            for block in 0..link.size.div_ceil(512) { blocks.push(link.start + block) }
        }

        let sectors = (src.count * PAGE_SIZE).div_ceil(512);
        let mut chain = Vec::with_capacity(sectors);

        // Allocate more blocks (if needed)
        for i in 0..sectors {
            chain.push(match blocks.get(i) {
                Some(&b) => b,
                None => self.bitmap.alloc(1).ok_or(FSError::NoSpace)?
            });
        }

        // Free any surplus
        for &block in blocks.iter().skip(sectors) {
            self.bitmap.free(block, 1);
            self.drive.zero_sectors(block, 1).map_err(|_| FSError::IOError)?;
        }

        // Build new link chain
        let head = self.build_file_chain(&chain)?;
        file.head = head;

        let links = links::resolve_links(&self.drive, head)?;

        // Unwrap: resolve_links() always returns atleast one link pointer. Safe
        let sizes = links.iter().map(|l| l.size);
        let max_size = sizes.clone().max().unwrap() as usize;
        file.size = sizes.sum::<u64>();
        let page = PhysPage::new(max_size.div_ceil(PAGE_SIZE)).map_err(|_| FSError::IOError)?;

        let mut offset = 0;
        for link in &links {
            // Copy data into page
            let src = (src.address() + offset).as_ptr::<u8>();
            let dst = page.address().as_mut_ptr::<u8>();

            // Safety: A memcpy to known good regions. Safe
            unsafe { core::ptr::copy_nonoverlapping(src, dst, link.size as usize) }

            // Write to disk
            self.drive.write_sectors(link.start, link.size.div_ceil(512), &page).map_err(|_| FSError::IOError)?;

            // No need to zero the full page
            // Safety: Just zeroing the page. Safe
            unsafe { core::ptr::write_bytes(dst, 0, link.size as usize) };
            offset += link.size;
        }

        parent.sync(&self.drive, &mut self.bitmap)?;

        // Update root cache, if necessary
        if path.parent().is_root() && let Some(i) = self.root.iter().position(|e| e.name() == path.name()) {
            self.root[i] = parent.children[dentry_index].clone();
        }

        Ok(())
    }

    fn route_symlink(&mut self, symlink_path: String, dest_path: String) -> Result<(), FSError> {
        let sym_path = Path::from_string(symlink_path);
        if sym_path.is_root() { return Err(FSError::IsRoot) }

        let mut parent = self.get_parent(&sym_path.parent())?;
        let dentry_idx = parent.find_child_idx(&sym_path.name()).ok_or(FSError::FileNotFound)?;
        let entry = &mut parent.children[dentry_idx];
        let DirectoryEntry::SymLink { name: _, dest } = entry else { return Err(FSError::BadFileType) };
        
        let target = {
            let dest_path = Path::from_string(dest_path);
            let dest_parent = self.get_parent(&dest_path)?;
            let dest_index = dest_parent.find_child_idx(&dest_path.name()).ok_or(FSError::FileNotFound)?;
            match &dest_parent.children[dest_index] {
                DirectoryEntry::File(f) => f.head,
                DirectoryEntry::Directory { link, .. } => *link,
                DirectoryEntry::SymLink { dest, .. } => *dest,
            }
        };

        *dest = target;
        parent.sync(&self.drive, &mut self.bitmap)?;

        if sym_path.parent().is_root() && let Some(i) = self.root.iter().position(|e| e.name() == sym_path.name()) {
            self.root[i] = parent.children[dentry_idx].clone();
        }

        Ok(())
    }

    fn sync(&mut self) -> Result<(), FSError> {
        superblock::write_sb(&self.drive, &self.superblock)?;
        self.bitmap.sync(&self.drive, self.superblock.bitmap_start)
    }
}

pub fn init(drive: PartitionDrive) -> FSResult<LemonFS> {
    let superblock = match superblock::get_superblock(&drive) {
        Ok(sb) => sb.0,
        Err(FSError::BadSuperblock) => format(&drive)?,
        Err(e) => return Err(e)
    };

    let mut bitmap = BitmapAllocator::new(superblock.data_start);

    for block in superblock.bitmap_start..superblock.data_start {
        let bitmap_block: BitmapBlock = io::read_data(&drive, block)?;
        bitmap.add_block(Box::new(bitmap_block));
    }

    bitmap.reserve_from(superblock.data_end - 1);

    let root_ptr = LinkPointer::new(superblock.data_end, 512);
    let root_block: DirBlock = root_ptr.read(&drive)?;
    let root_dir = Directory::from_dir_block(&drive, &root_block, Filename::from_str("/"), root_ptr)?;

    Ok(LemonFS { drive, superblock, bitmap, root: root_dir.children })
}

fn format(disk: Disk) -> FSResult<SuperBlock> {
    let sb = SuperBlock::new(disk.sector_count()).ok_or(FSError::NoSpace)?;
    superblock::write_sb(disk, &sb)?;

    disk.zero_sectors(sb.bitmap_start, sb.data_start - sb.bitmap_start).map_err(|_| FSError::IOError)?;
    disk.zero_sectors(sb.data_end, 1).map_err(|_| FSError::IOError)?;

    Ok(sb)
}