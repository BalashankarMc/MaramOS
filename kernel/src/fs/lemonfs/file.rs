use alloc::vec::Vec;

use crate::fs::FSError;

use super::{
    Disk, FSResult,
    bitmap::BitmapAllocator,
    disk::{DirEntry, DirEntryFlags, Filename, LinkPointer},
    links::{resolve_links, trace}
};

#[derive(Debug, Clone)]
pub struct File {
    pub name: Filename,
    pub size: u64,
    pub head: LinkPointer,
    pub links: Vec<LinkPointer>
}

impl File {
    pub fn new(name: &str, start: u64) -> Self {
        let pointer = LinkPointer::new(start, 1);
        Self {
            name: Filename::from_str(name),
            size: 0,
            head: pointer,
            links: Vec::new()
        }
    }

    pub fn from_dir_entry(disk: Disk, dentry: DirEntry) -> FSResult<Self> {
        if dentry.flags.contains(DirEntryFlags::IS_DIR) 
            || dentry.flags.contains(DirEntryFlags::IS_SYMLINK) { return Err(FSError::BadFileType) }
        
        let links = resolve_links(disk, dentry.link)?;
        let size = links.iter().map(|l| l.size).sum::<u64>();

        Ok(Self { name: dentry.name, size, head: dentry.link, links })
    }

    pub fn delete(self, disk: Disk, bitmap: &mut BitmapAllocator) -> FSResult<()> {
        let (direct, indirect) = trace(disk, self.head)?;

        // Free direct pointers
        for link in direct {
            let blocks = link.size.div_ceil(512);
            bitmap.free(link.start, blocks);
            disk.zero_sectors(link.start, blocks).map_err(|_| FSError::IOError)?;
        }

        // Free LinkBlocks
        for link in indirect {
            bitmap.free(link.start, 1);
            disk.zero_sectors(link.start, 1).map_err(|_| FSError::IOError)?;
        }

        Ok(())
    }
}