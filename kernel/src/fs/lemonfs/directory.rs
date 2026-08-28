use alloc::{string::String, vec::Vec, vec};

use crate::fs::FSError;
use super::{Disk, FSResult,
    bitmap::BitmapAllocator,
    disk::{DirBlock, DirEntry, DirEntryFlags, ENTRIES_PER_DIRENTRY, Filename, LinkPointer},
    file::File,
    io::write_data
};

pub enum FileType {
    File,
    Directory,
    Symlink
}

impl FileType {
    pub const fn from_dirflags(flags: DirEntryFlags) -> Self {
        if flags.contains(DirEntryFlags::IS_DIR) { Self::Directory }
        else if flags.contains(DirEntryFlags::IS_SYMLINK) { Self::Symlink }
        else { Self::File }
    }
}

#[derive(Debug, Clone)]
pub enum DirectoryEntry {
    File(File),
    Directory { name: Filename, link: LinkPointer },
    SymLink { name: Filename, dest: LinkPointer }
}

impl DirectoryEntry {
    pub fn new(name: Filename, start: u64, filetype: &FileType) -> Self {
        match filetype {
            FileType::Directory => Self::Directory { name, link: LinkPointer::new(start, 512) },
            FileType::File => Self::File(File::new(&name.to_str(), start)),
            FileType::Symlink => Self::SymLink { name, dest: LinkPointer::new(0, 0) }
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Directory { name, link: _ } | Self::SymLink { name, dest: _ }=> name.to_str(),
            Self::File(f) => f.name.to_str()
        }
    }

    const fn new_dir(name: Filename, start: u64) -> Self { Self::Directory { name, link: LinkPointer::new(start, 512) } }
    const fn to_dentry(&self) -> DirEntry {
        match self {
            Self::File(f) => DirEntry::new(&f.name, DirEntryFlags::empty(), f.head),
            Self::Directory { name, link } => DirEntry::new(name, DirEntryFlags::IS_DIR, *link),
            Self::SymLink { name, dest } 
                => DirEntry::new(name, DirEntryFlags::IS_SYMLINK, *dest)
        }
    }

    fn from_dir_entry(disk: Disk, dentry: DirEntry) -> FSResult<Self> {
        if dentry.flags.contains(DirEntryFlags::IS_SYMLINK) { Ok(Self::SymLink { name: dentry.name, dest: dentry.link }) }
        else if dentry.flags.contains(DirEntryFlags::IS_DIR) { Ok(Self::Directory { name: dentry.name, link: dentry.link }) }
        else { Ok(Self::File(File::from_dir_entry(disk, dentry)?)) }
    }
}

#[derive(Debug, Clone)]
pub struct Directory {
    pub name: Filename,
    pub parent: u64,
    pub children: Vec<DirectoryEntry>,
    pub head: LinkPointer
}

impl Directory {
    pub fn find_child_idx(&self, name: &String) -> Option<usize> {
        self.children.iter().position(|e| e.name() == *name)
    }

    pub fn get_child(&self, name: &String) -> Option<&DirectoryEntry> {
        self.children.iter().find(|e| e.name() == *name)
    }

    pub fn from_dir_entry(disk: Disk, dentry: DirEntry, parent: u64) -> FSResult<Self> {
        let mut children = Vec::new();
        let flags = dentry.flags;

        if !flags.contains(DirEntryFlags::IS_DIR) || flags.contains(DirEntryFlags::IS_SYMLINK) { return Err(FSError::BadFileType) }
        add_children_from_link(disk, &mut children, dentry.link)?;
        
        Ok(Self { name: dentry.name, parent, children, head: dentry.link })
    }

    pub fn from_directory_entry(disk: Disk, dentry: &DirectoryEntry) -> FSResult<Self> {
        let dir_data = match dentry {
            DirectoryEntry::Directory { name, link } => (name, link),
            _ => return Err(FSError::BadFileType)
        };

        let name = *dir_data.0;
        let dir_block: DirBlock = dir_data.1.read(disk)?;

        Self::from_dir_block(disk, &dir_block, name, *dir_data.1)
    }

    pub fn from_dir_block(disk: Disk, dir_block: &DirBlock, name: Filename, head: LinkPointer) -> FSResult<Self> {
        let mut children = Vec::new();
        add_children_from_link(disk, &mut children, head)?;

        Ok(Self { name, parent: dir_block.parent, children, head })
    }

    pub fn give_child(&mut self, disk: Disk, bitmap: &mut BitmapAllocator, child: DirectoryEntry) -> Result<(), FSError> {
        self.children.push(child);
        self.sync(disk, bitmap)
    }

    pub fn delete(self, disk: Disk, bitmap: &mut BitmapAllocator) -> FSResult<()> {
        // Delete all children
        for child in self.children {
            match child {
                DirectoryEntry::File(f) => f.delete(disk, bitmap)?,
                DirectoryEntry::Directory { name: _, link } => {
                    let block: DirBlock = link.read(disk)?;
                    let dir = Self::from_dir_block(disk, &block, Filename::from_str(""), link)?;
                    dir.delete(disk, bitmap)?;
                },

                DirectoryEntry::SymLink { name: _, dest: _ } => () // Do nothing. Deleting the DirEntries will handle this
            }
        }

        // Delete DirBlocks
        let mut ptr = self.head;
        while ptr.is_valid() && ptr.is_direct() {
            let block: DirBlock = ptr.read(disk)?;
            bitmap.free(ptr.start, 1);
            disk.zero_sectors(ptr.start, 1).map_err(|_| FSError::IOError)?;
            ptr = block.next;
        }

        Ok(())
    }

    pub fn sync(&self, disk: Disk, bitmap: &mut BitmapAllocator) -> FSResult<()> {
        let blocks_needed = self.children.len().div_ceil(ENTRIES_PER_DIRENTRY).max(1);

        // Collect current chain
        let mut collected = Vec::new();
        let mut ptr = self.head;
        while ptr.is_direct() && ptr.is_valid() {
            collected.push(ptr.start);
            let block: DirBlock = ptr.read(disk)?;
            ptr = block.next;
        }

        // Free any surplus
        if collected.len() > blocks_needed {
            for &lba in &collected[blocks_needed..] { bitmap.free(lba, 1) }
        }

        let mut chain = vec![self.head.start];
        for i in 1..blocks_needed {
            if let Some(x) = collected.get(i) { chain.push(*x); }
            else { chain.push(bitmap.alloc(1).ok_or(FSError::NoSpace)?); }
        }

        // Write to disk
        let entry_chunks = self.children.chunks(ENTRIES_PER_DIRENTRY);
        for (i, chunk) in entry_chunks.enumerate() {
            let entries = chunk.iter().map(DirectoryEntry::to_dentry).collect::<Vec<DirEntry>>();
            let mut buf = [DirEntry::zero(); ENTRIES_PER_DIRENTRY];
            buf[..entries.len()].copy_from_slice(&entries);

            let next = chain.get(i + 1)
                .map_or(LinkPointer::new(0, 0), |lba| LinkPointer::new(*lba, 512));

            write_data(disk, chain[i], DirBlock::new(self.parent, &buf, next))?;
        }

        if self.children.is_empty() {
            write_data(disk, chain[0], DirBlock::new(
                self.parent, &[DirEntry::zero(); ENTRIES_PER_DIRENTRY], LinkPointer::new(0, 0))
            )?;
        }

        Ok(())
    }
}

fn add_children_from_link(disk: Disk, children: &mut Vec<DirectoryEntry>, start: LinkPointer) -> FSResult<()> {
    let mut ptr = start;
    while ptr.is_direct() {
        let block: DirBlock = ptr.read(disk)?;
        let block_children = block.entries.iter().filter_map(
            |e| {
                if e.name.is_empty() { None }
                else { Some(DirectoryEntry::from_dir_entry(disk, *e)) }
            }
        ).flatten();

        children.extend(block_children);
        ptr = block.next;
    }

    Ok(())
}