use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::mem::size_of;
use std::path::Path;

const LBA_SIZE: u64 = 512;
const LEMONFS_MAGIC: u64 = u64::from_le_bytes(*b"LEMONFS\0");
const FILENAME_LEN: usize = 40;
const HASHES_PER_BLOCK: u64 = 6; // 512 / size_of::<HashEntry>() = 512/74 = 6
const LINKS_PER_BLOCK: usize = 32; // 512 / size_of::<LinkEntry>() = 512/16 = 32
const HASHPTRS_PER_BLOCK: usize = 56; // 512 / size_of::<HashPointer>() = 512/9 = 56
const DIRBLOCK_PADDING: usize = 512 - HASHPTRS_PER_BLOCK * size_of::<HashPointer>(); // 8

// ---------------------------------------------------------------------------
// On-disk structures — must match the kernel exactly
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct SuperBlock {
    magic: u64,
    version: u16,
    total_blocks: u64,
    hashmap_size: u64,
    bitmap_size: u64,
    dirty: u8,
    _padding: [u8; 477],
}

impl SuperBlock {
    fn bitmap_start(&self) -> u64 { 1 + self.hashmap_size.div_ceil(HASHES_PER_BLOCK) }
    fn data_start(&self) -> u64 { self.bitmap_start() + self.bitmap_size }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HashEntry {
    status: u8,     // 0=Unused, 1=Used, 0xFF=Tombstone
    type_: u8,      // 0=File, 1=Directory
    name: [u8; FILENAME_LEN],
    start_block: u64,
    file_size: u64,
    link_ptr: u64,  // LinkEntry.ptr (indirect LBA, 0 if unused)
    link_size: u64, // LinkEntry.size (0 if indirect or unused)
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HashTableBlock {
    entries: [HashEntry; HASHES_PER_BLOCK as usize],
    _pad: [u8; 68], // 512 - 6*74 = 68
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HashPointer {
    lba: u64,
    offset: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct LinkEntry {
    ptr: u64,
    size: u64,
}

impl LinkEntry {
    fn is_valid(&self) -> bool { self.ptr != 0 }
    fn is_direct(&self) -> bool { self.size != 0 }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct LinkBlock {
    entries: [LinkEntry; LINKS_PER_BLOCK],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct DirectoryEntry {
    entries: [HashPointer; HASHPTRS_PER_BLOCK],
    _pad: [u8; DIRBLOCK_PADDING],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Bitmap {
    entries: [u8; 512],
}

#[repr(u8)]
enum EntryType {
    File = 0,
    Directory = 1,
}

// ---------------------------------------------------------------------------
// LemonFS driver
// ---------------------------------------------------------------------------

pub struct LemonFS {
    file: File,
    superblock: SuperBlock,
}

impl LemonFS {
    /// Create a fresh LemonFS filesystem in the file at `path`.
    /// The file must already exist and be sized to the desired partition size.
    pub fn format(path: &str) -> Result<Self, String> {
        let img_path = Path::new(path);
        if !img_path.exists() {
            return Err(format!("'{}' does not exist", path));
        }

        let meta = std::fs::metadata(path)
            .map_err(|e| format!("Failed to stat '{}': {}", path, e))?;
        let total_bytes = meta.len();
        let total_lbas = total_bytes / LBA_SIZE;
        if total_lbas < 3 {
            return Err(format!("'{}' is too small for a LemonFS partition", path));
        }

        let hashmap_size = std::cmp::max(4096u64, total_lbas / 10_000);
        let hash_blocks = hashmap_size.div_ceil(HASHES_PER_BLOCK);
        let remaining = total_lbas - (1 + hash_blocks);
        let bitmap_size = remaining.div_ceil(4096);

        let superblock = SuperBlock {
            magic: LEMONFS_MAGIC,
            version: 2,
            total_blocks: total_lbas,
            hashmap_size,
            bitmap_size,
            dirty: 0,
            _padding: [0u8; 477],
        };

        let mut file = OpenOptions::new().read(true).write(true).open(path)
            .map_err(|e| format!("Failed to open '{}': {}", path, e))?;

        // Write superblock at LBA 0
        let mut sb_buf = [0u8; size_of::<SuperBlock>()];
        unsafe {
            std::ptr::write_unaligned(sb_buf.as_mut_ptr() as *mut SuperBlock, superblock);
        }
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        file.write_all(&sb_buf).map_err(|e| e.to_string())?;

        // Zero all hash table blocks (LBA 1 .. bitmap_start)
        let bitmap_start = superblock.bitmap_start();
        let zero_block = [0u8; LBA_SIZE as usize];
        for lba in 1..bitmap_start {
            file.seek(SeekFrom::Start(lba * LBA_SIZE)).map_err(|e| e.to_string())?;
            file.write_all(&zero_block).map_err(|e| e.to_string())?;
        }

        // Zero all bitmap blocks
        for lba in bitmap_start..bitmap_start + superblock.bitmap_size {
            file.seek(SeekFrom::Start(lba * LBA_SIZE)).map_err(|e| e.to_string())?;
            file.write_all(&zero_block).map_err(|e| e.to_string())?;
        }

        // Allocate first data block for root directory (bit 0 in bitmap)
        let data_start = superblock.data_start();
        let root_lba = data_start;
        {
            let bit = 0usize;
            let byte_idx = bit / 8;
            let mask = 1u8 << (bit % 8);
            let bm_lba = bitmap_start;
            let mut bm_block = [0u8; LBA_SIZE as usize];
            file.seek(SeekFrom::Start(bm_lba * LBA_SIZE)).map_err(|e| e.to_string())?;
            file.read_exact(&mut bm_block).map_err(|e| e.to_string())?;
            bm_block[byte_idx] |= mask;
            file.seek(SeekFrom::Start(bm_lba * LBA_SIZE)).map_err(|e| e.to_string())?;
            file.write_all(&bm_block).map_err(|e| e.to_string())?;
        }

        // Write empty directory entry block at root_lba
        let empty_dir = [0u8; LBA_SIZE as usize];
        file.seek(SeekFrom::Start(root_lba * LBA_SIZE)).map_err(|e| e.to_string())?;
        file.write_all(&empty_dir).map_err(|e| e.to_string())?;

        // Create root directory hash entry: status=1, type_=1(Dir), name="/",
        //   start_block=root_lba, file_size=0, link_ptr=0, link_size=0
        let root_name = Self::name_to_bytes("/").map_err(|e| e.to_string())?;
        let root_hash = Self::hash_name(&root_name);
        let slot = root_hash % superblock.hashmap_size;
        let hash_lba = 1 + slot / HASHES_PER_BLOCK;
        let hash_offset = (slot % HASHES_PER_BLOCK) as usize;

        let root_entry = HashEntry {
            status: 1,
            type_: 1, // Directory
            name: root_name,
            start_block: root_lba,
            file_size: 0,
            link_ptr: 0,
            link_size: 0,
        };

        // Read the hash table block, insert root entry, write it back
        let mut hash_block_buf = [0u8; LBA_SIZE as usize];
        file.seek(SeekFrom::Start(hash_lba * LBA_SIZE)).map_err(|e| e.to_string())?;
        file.read_exact(&mut hash_block_buf).map_err(|e| e.to_string())?;

        // Each HashEntry is 74 bytes, pack them into the block
        let entry_size = size_of::<HashEntry>();
        let entry_start = hash_offset * entry_size;
        let entry_bytes = unsafe {
            std::slice::from_raw_parts(&root_entry as *const HashEntry as *const u8, entry_size)
        };
        hash_block_buf[entry_start..entry_start + entry_size].copy_from_slice(entry_bytes);

        file.seek(SeekFrom::Start(hash_lba * LBA_SIZE)).map_err(|e| e.to_string())?;
        file.write_all(&hash_block_buf).map_err(|e| e.to_string())?;

        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        Ok(LemonFS { file, superblock })
    }

    pub fn open(path: &str) -> Result<Self, String> {
        let img_path = Path::new(path);
        if !img_path.exists() {
            return Err(format!("'{}' does not exist", path));
        }

        let mut file = OpenOptions::new().read(true).write(true).open(path)
            .map_err(|e| format!("Failed to open '{}': {}", path, e))?;

        let mut sb_buf = [0u8; size_of::<SuperBlock>()];
        file.seek(SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        file.read_exact(&mut sb_buf)
            .map_err(|e| format!("Failed to read superblock: {}", e))?;
        let superblock: SuperBlock =
            unsafe { std::ptr::read_unaligned(sb_buf.as_ptr() as *const SuperBlock) };

        if superblock.magic != LEMONFS_MAGIC {
            drop(file);
            eprintln!("  (unformatted partition detected — formatting)");
            return Self::format(path);
        }

        Ok(LemonFS { file, superblock })
    }

    // -- Low-level I/O --

    fn read_block<T: Copy>(&mut self, lba: u64) -> Result<T, String> {
        let size = size_of::<T>();
        let mut buf = vec![0u8; size];
        self.file
            .seek(SeekFrom::Start(lba * LBA_SIZE))
            .map_err(|e| e.to_string())?;
        self.file
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error at LBA {}: {}", lba, e))?;
        Ok(unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const T) })
    }

    fn write_block<T: Copy>(&mut self, lba: u64, value: &T) -> Result<(), String> {
        let size = size_of::<T>();
        let mut buf = vec![0u8; size];
        unsafe {
            std::ptr::write_unaligned(buf.as_mut_ptr() as *mut T, *value);
        }
        self.file
            .seek(SeekFrom::Start(lba * LBA_SIZE))
            .map_err(|e| e.to_string())?;
        self.file
            .write_all(&buf)
            .map_err(|e| format!("Write error at LBA {}: {}", lba, e))?;
        Ok(())
    }

    fn write_raw(&mut self, lba: u64, buf: &[u8]) -> Result<(), String> {
        self.file
            .seek(SeekFrom::Start(lba * LBA_SIZE))
            .map_err(|e| e.to_string())?;
        self.file
            .write_all(buf)
            .map_err(|e| format!("Write error at LBA {}: {}", lba, e))
    }

    fn read_lba_bytes(&mut self, lba: u64) -> Result<[u8; 512], String> {
        let mut buf = [0u8; 512];
        self.file
            .seek(SeekFrom::Start(lba * LBA_SIZE))
            .map_err(|e| e.to_string())?;
        self.file
            .read_exact(&mut buf)
            .map_err(|e| format!("Read error at LBA {}: {}", lba, e))?;
        Ok(buf)
    }

    // -- Hash helpers --

    fn hash_name(name: &[u8; FILENAME_LEN]) -> u64 {
        let mut hash: u64 = 0x811C9DC5;
        for &b in name {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x1000193);
        }
        hash
    }

    fn name_to_bytes(name: &str) -> Result<[u8; FILENAME_LEN], String> {
        if name.is_empty() || name.len() > FILENAME_LEN {
            return Err("Invalid filename (must be 1-40 characters)".into());
        }
        let mut bytes = [0u8; FILENAME_LEN];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(bytes)
    }

    fn name_eq(bytes: &[u8; FILENAME_LEN], name: &str) -> bool {
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(FILENAME_LEN);
        len == name.len() && &bytes[..len] == name.as_bytes()
    }

    fn bytes_to_name(bytes: &[u8; FILENAME_LEN]) -> String {
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(FILENAME_LEN);
        String::from_utf8_lossy(&bytes[..len]).to_string()
    }

    // -- HashPointer helpers --

    fn hash_ptr_read(&mut self, ptr: &HashPointer) -> Result<HashEntry, String> {
        let block: HashTableBlock = self.read_block(ptr.lba)?;
        Ok(block.entries[ptr.offset as usize])
    }

    fn hash_ptr_write(&mut self, ptr: &HashPointer, entry: &HashEntry) -> Result<(), String> {
        let mut block: HashTableBlock = self.read_block(ptr.lba)?;
        block.entries[ptr.offset as usize] = *entry;
        self.write_block(ptr.lba, &block)
    }

    fn hash_ptr_add(&self, ptr: &HashPointer, n: u64) -> Option<HashPointer> {
        let tmp = ptr.offset as u64 + n;
        let offset = (tmp % HASHES_PER_BLOCK) as u8;
        let lba = ptr.lba + (tmp / HASHES_PER_BLOCK);
        if lba > self.superblock.hashmap_size.div_ceil(HASHES_PER_BLOCK) {
            None
        } else {
            Some(HashPointer { lba, offset })
        }
    }

    fn hash_ptr_valid(ptr: &HashPointer) -> bool { ptr.lba != 0 }

    // -- Hash table operations --

    fn find_free_slot(&mut self, name: &[u8; FILENAME_LEN]) -> Result<HashPointer, String> {
        let hash = Self::hash_name(name);
        let total_slots = self.superblock.hashmap_size;
        let start = hash % total_slots;

        for probe in 0..total_slots {
            let slot = (start + probe) % total_slots;
            let block_lba = 1 + slot / HASHES_PER_BLOCK;
            let entry_idx = (slot % HASHES_PER_BLOCK) as usize;

            let block: HashTableBlock = self.read_block(block_lba)?;
            let status = block.entries[entry_idx].status;

            if status == 0 || status == 0xFF {
                return Ok(HashPointer { lba: block_lba, offset: entry_idx as u8 });
            }
        }

        Err("Hash table is full".into())
    }

    fn find_hash_slot(
        &mut self,
        name: &[u8; FILENAME_LEN],
    ) -> Result<(HashPointer, bool), String> {
        let hash = Self::hash_name(name);
        let total_slots = self.superblock.hashmap_size;
        let start = hash % total_slots;
        let mut first_free: Option<HashPointer> = None;

        for probe in 0..total_slots {
            let slot = (start + probe) % total_slots;
            let block_lba = 1 + slot / HASHES_PER_BLOCK;
            let entry_idx = (slot % HASHES_PER_BLOCK) as usize;

            let block: HashTableBlock = self.read_block(block_lba)?;
            let entry = &block.entries[entry_idx];
            let ptr = HashPointer { lba: block_lba, offset: entry_idx as u8 };

            if entry.status == 0 {
                // Unused — end of probe chain. Return first free slot if any.
                return Ok((first_free.unwrap_or(ptr), false));
            }

            if entry.status == 0xFF {
                // Tombstone — skip, but remember as free slot
                if first_free.is_none() { first_free = Some(ptr); }
                continue;
            }

            if entry.status == 1 && entry.name == *name {
                return Ok((ptr, true));
            }
        }

        Err("Hash table is full".into())
    }

    fn write_hash_entry(
        &mut self,
        name: &[u8; FILENAME_LEN],
        entry_type: EntryType,
        start_block: u64,
        file_size: u64,
        link_ptr: u64,
        link_size: u64,
    ) -> Result<HashPointer, String> {
        let (ptr, _exists) = self.find_hash_slot(name)?;
        let entry = HashEntry {
            status: 1,
            type_: entry_type as u8,
            name: *name,
            start_block,
            file_size,
            link_ptr,
            link_size,
        };
        self.hash_ptr_write(&ptr, &entry)?;
        Ok(ptr)
    }

    fn tombstone(&mut self, ptr: &HashPointer) -> Result<(), String> {
        let mut entry = self.hash_ptr_read(ptr)?;
        entry.status = 0xFF; // Tombstone
        self.hash_ptr_write(ptr, &entry)
    }

    fn find_entry(&mut self, name: &[u8; FILENAME_LEN]) -> Result<(HashPointer, HashEntry), String> {
        let (ptr, exists) = self.find_hash_slot(name)?;
        if !exists {
            return Err("Entry not found".into());
        }
        let entry = self.hash_ptr_read(&ptr)?;
        Ok((ptr, entry))
    }

    // -- Bitmap operations --

    fn free_blocks(&mut self, start_lba: u64, count: u64) -> Result<(), String> {
        if count == 0 { return Ok(()) }
        let data_start = self.superblock.data_start();
        let bitmap_start = self.superblock.bitmap_start();
        let start_bit = (start_lba - data_start) as usize;
        let end_bit = start_bit + count as usize;

        for block_idx in start_bit / 4096..=(end_bit - 1) / 4096 {
            let mut bitmap: Bitmap = self.read_block(bitmap_start + block_idx as u64)?;
            let local_start = start_bit.saturating_sub(block_idx * 4096);
            let local_end = std::cmp::min(4096, end_bit - block_idx * 4096);
            for b in local_start..local_end {
                let (byte_idx, mask) = (b / 8, 1 << (b % 8));
                bitmap.entries[byte_idx] &= !mask;
            }
            self.write_block(bitmap_start + block_idx as u64, &bitmap)?;
        }
        Ok(())
    }

    fn alloc_blocks(&mut self, count: u64) -> Result<u64, String> {
        if count == 0 {
            return Ok(self.superblock.data_start());
        }

        let data_start = self.superblock.data_start();
        let total_blocks = self.superblock.total_blocks;
        let bitmap_size = self.superblock.bitmap_size;
        let bitmap_start = self.superblock.bitmap_start();
        let total_data = (total_blocks - data_start) as usize;

        for block_idx in 0..bitmap_size {
            let bitmap: Bitmap = self.read_block(bitmap_start + block_idx)?;
            let bits = std::cmp::min(4096, total_data.saturating_sub(block_idx as usize * 4096));

            let mut consecutive = 0u64;
            let mut run_start = 0usize;

            for bit in 0..bits {
                let (byte_idx, mask) = (bit / 8, 1 << (bit % 8));
                let is_set = bitmap.entries[byte_idx] & mask != 0;

                if !is_set {
                    if consecutive == 0 {
                        run_start = block_idx as usize * 4096 + bit;
                    }
                    consecutive += 1;
                    if consecutive == count {
                        return self.alloc_commit(run_start, count as usize);
                    }
                } else {
                    consecutive = 0;
                }
            }
        }

        Err("Disk full".into())
    }

    fn alloc_commit(&mut self, start_bit: usize, count: usize) -> Result<u64, String> {
        let data_start = self.superblock.data_start();
        let bitmap_start = self.superblock.bitmap_start();
        let end_bit = start_bit + count;

        for block_idx in start_bit / 4096..=(end_bit - 1) / 4096 {
            let mut bitmap: Bitmap = self.read_block(bitmap_start + block_idx as u64)?;
            let local_start = start_bit.saturating_sub(block_idx * 4096);
            let local_end = std::cmp::min(4096, end_bit - block_idx * 4096);
            for b in local_start..local_end {
                let (byte_idx, mask) = (b / 8, 1 << (b % 8));
                bitmap.entries[byte_idx] |= mask;
            }
            self.write_block(bitmap_start + block_idx as u64, &bitmap)?;
        }

        Ok(data_start + start_bit as u64)
    }

    // -- Link chain operations --

    fn extend_link(
        &mut self,
        link: &mut LinkEntry,
        append: LinkEntry,
    ) -> Result<(), String> {
        if !link.is_valid() {
            *link = append;
            return Ok(());
        }

        let mut lba = link.ptr;
        loop {
            let mut block: LinkBlock = self.read_block(lba)?;

            for i in 0..LINKS_PER_BLOCK {
                if !block.entries[i].is_valid() {
                    block.entries[i] = append;
                    return self.write_block(lba, &block);
                }
            }

            // Block is full — follow indirect chain
            let last = block.entries[LINKS_PER_BLOCK - 1];
            if last.is_direct() {
                // Displace last entry to new block
                let new_block_lba = self.alloc_blocks(1)?;
                let mut new_block = LinkBlock { entries: [LinkEntry { ptr: 0, size: 0 }; LINKS_PER_BLOCK] };
                new_block.entries[0] = last;
                new_block.entries[1] = append;
                self.write_block(new_block_lba, &new_block)?;

                block.entries[LINKS_PER_BLOCK - 1] = LinkEntry { ptr: new_block_lba, size: 0 };
                return self.write_block(lba, &block);
            }

            lba = last.ptr;
        }
    }

    fn free_link_chain(&mut self, link: LinkEntry) -> Result<(), String> {
        let mut stack = vec![link];
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                let lbas = entry.size.div_ceil(LBA_SIZE);
                self.free_blocks(entry.ptr, lbas)?;
            } else {
                let linkblock: LinkBlock = self.read_block(entry.ptr)?;
                for e in linkblock.entries { stack.push(e) }
                self.free_blocks(entry.ptr, 1)?;
            }
        }
        Ok(())
    }

    /// Read file data following the link chain starting from `link`.
    fn read_link_data(
        &mut self,
        link: &LinkEntry,
        base_lba: u64,
        base_size: u64,
        out: &mut Vec<u8>,
    ) -> Result<(), String> {
        // Read direct blocks from base
        if base_size > 0 {
            let lbas = base_size.div_ceil(LBA_SIZE);
            for i in 0..lbas {
                let block = self.read_lba_bytes(base_lba + i)?;
                let start = (i * LBA_SIZE) as usize;
                let end = std::cmp::min(start + LBA_SIZE as usize, base_size as usize);
                out.extend_from_slice(&block[..end - start]);
            }
        }

        // Follow link chain
        let mut stack = vec![*link];
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                let lbas = entry.size.div_ceil(LBA_SIZE);
                for i in 0..lbas {
                    let block = self.read_lba_bytes(entry.ptr + i)?;
                    let start = (i * LBA_SIZE) as usize;
                    let end = std::cmp::min(start + LBA_SIZE as usize, entry.size as usize);
                    out.extend_from_slice(&block[..end - start]);
                }
            } else {
                let linkblock: LinkBlock = self.read_block(entry.ptr)?;
                for e in linkblock.entries { stack.push(e) }
            }
        }
        Ok(())
    }

    /// Compute total file size including link chain.
    fn total_file_size(&mut self, _start_block: u64, file_size: u64, link: &LinkEntry) -> Result<u64, String> {
        let mut total = file_size;
        let mut stack = vec![*link];
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                total += entry.size;
            } else {
                let linkblock: LinkBlock = self.read_block(entry.ptr)?;
                for e in linkblock.entries { stack.push(e) }
            }
        }
        Ok(total)
    }

    // -- Directory operations --

    fn read_dentry(&mut self, lba: u64) -> Result<DirectoryEntry, String> {
        self.read_block(lba)
    }

    fn find_free_dentry_slot(dentry: &DirectoryEntry) -> Option<usize> {
        dentry.entries.iter().position(|e| !Self::hash_ptr_valid(e))
    }

    fn dir_children(&mut self, dir_entry: &HashEntry) -> Result<Vec<HashPointer>, String> {
        let mut children = Vec::new();

        // Direct slots
        let dentry: DirectoryEntry = self.read_block(dir_entry.start_block)?;
        for entry in dentry.entries {
            if Self::hash_ptr_valid(&entry) { children.push(entry); }
        }

        // Linked directory entry blocks
        let link = LinkEntry { ptr: dir_entry.link_ptr, size: dir_entry.link_size };
        let mut stack = vec![link];
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                let dentry: DirectoryEntry = self.read_block(entry.ptr)?;
                for e in dentry.entries {
                    if Self::hash_ptr_valid(&e) { children.push(e); }
                }
            } else {
                let linkblock: LinkBlock = self.read_block(entry.ptr)?;
                for e in linkblock.entries { stack.push(e) }
            }
        }

        Ok(children)
    }

    fn dir_find_child(
        &mut self,
        dir_entry: &HashEntry,
        child_name: &str,
    ) -> Result<(HashPointer, HashEntry), String> {
        let children = self.dir_children(dir_entry)?;
        for child_ptr in &children {
            let child_entry = self.hash_ptr_read(child_ptr)?;
            if child_entry.status == 1 && Self::name_eq(&child_entry.name, child_name) {
                return Ok((*child_ptr, child_entry));
            }
        }
        Err("Entry not found".into())
    }

    fn dir_add_child(
        &mut self,
        dir_lba: u64,
        dir_link: &mut LinkEntry,
        child: HashPointer,
    ) -> Result<(), String> {
        // Try direct slot
        let mut dentry: DirectoryEntry = self.read_block(dir_lba)?;
        if let Some(idx) = Self::find_free_dentry_slot(&dentry) {
            dentry.entries[idx] = child;
            return self.write_block(dir_lba, &dentry);
        }

        // Try linked blocks
        let mut stack = vec![*dir_link];
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                let mut dentry: DirectoryEntry = self.read_block(entry.ptr)?;
                if let Some(idx) = Self::find_free_dentry_slot(&dentry) {
                    dentry.entries[idx] = child;
                    return self.write_block(entry.ptr, &dentry);
                }
            } else {
                let linkblock: LinkBlock = self.read_block(entry.ptr)?;
                for e in linkblock.entries { stack.push(e) }
            }
        }

        // Allocate new directory entry block
        let new_lba = self.alloc_blocks(1)?;
        let mut new_dentry = DirectoryEntry {
            entries: [HashPointer { lba: 0, offset: 0 }; HASHPTRS_PER_BLOCK],
            _pad: [0; DIRBLOCK_PADDING],
        };
        new_dentry.entries[0] = child;
        self.write_block(new_lba, &new_dentry)?;

        self.extend_link(dir_link, LinkEntry { ptr: new_lba, size: LBA_SIZE })
    }

    fn dir_remove_child(
        &mut self,
        dir_entry: &HashEntry,
        child_ptr: &HashPointer,
    ) -> Result<(), String> {
        // Try direct slots
        let mut dentry: DirectoryEntry = self.read_block(dir_entry.start_block)?;
        for slot in dentry.entries.iter_mut() {
            if slot.lba == child_ptr.lba && slot.offset == child_ptr.offset {
                *slot = HashPointer { lba: 0, offset: 0 };
                return self.write_block(dir_entry.start_block, &dentry);
            }
        }

        // Try linked blocks
        let link = LinkEntry { ptr: dir_entry.link_ptr, size: dir_entry.link_size };
        let mut stack = vec![link];
        while let Some(entry) = stack.pop() {
            if !entry.is_valid() { continue }
            if entry.is_direct() {
                let mut dentry: DirectoryEntry = self.read_block(entry.ptr)?;
                for slot in dentry.entries.iter_mut() {
                    if slot.lba == child_ptr.lba && slot.offset == child_ptr.offset {
                        *slot = HashPointer { lba: 0, offset: 0 };
                        return self.write_block(entry.ptr, &dentry);
                    }
                }
            } else {
                let linkblock: LinkBlock = self.read_block(entry.ptr)?;
                for e in linkblock.entries { stack.push(e) }
            }
        }

        Err("Child not found in directory".into())
    }

    // -- Path resolution --

    fn ensure_and_get_directory(&mut self, name: &str, parent: &HashEntry) -> Result<HashEntry, String> {
        let name_bytes = Self::name_to_bytes(name)?;
        if let Ok((_ptr, entry)) = self.find_entry(&name_bytes) {
            if entry.type_ != EntryType::Directory as u8 {
                return Err(format!("'{}' exists but is not a directory", name));
            }
            return Ok(entry);
        }

        let dir_lba = self.alloc_blocks(1)?;
        let empty = DirectoryEntry {
            entries: [HashPointer { lba: 0, offset: 0 }; HASHPTRS_PER_BLOCK],
            _pad: [0; DIRBLOCK_PADDING],
        };
        self.write_block(dir_lba, &empty)?;

        let dir_ptr = self.write_hash_entry(&name_bytes, EntryType::Directory, dir_lba, 0, 0, 0)?;

        // Add to parent directory
        let mut parent_link = LinkEntry { ptr: parent.link_ptr, size: parent.link_size };
        self.dir_add_child(parent.start_block, &mut parent_link, dir_ptr)?;

        // Read back the fresh directory entry
        let entry = self.hash_ptr_read(&dir_ptr)?;
        Ok(entry)
    }

    /// Returns (filename, parent_hash_ptr, parent_dir_entry).
    fn resolve_parent(&mut self, path: &str) -> Result<(String, HashPointer, HashEntry), String> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Err("Cannot write to root directory".into());
        }

        // Start from root directory
        let root_name = Self::name_to_bytes("/")?;
        let (root_ptr, mut parent_entry) = self.find_entry(&root_name)?;
        let mut parent_ptr = root_ptr;

        let mut filename: &str = "";
        let mut walked = false;

        for comp in path.split('/').filter(|c| !c.is_empty()) {
            if walked {
                let name_bytes = Self::name_to_bytes(comp)?;
                let (ptr, exists) = self.find_hash_slot(&name_bytes)?;
                if exists {
                    let entry = self.hash_ptr_read(&ptr)?;
                    if entry.type_ != EntryType::Directory as u8 {
                        return Err(format!("'{}' exists but is not a directory", comp));
                    }
                    parent_ptr = ptr;
                    parent_entry = entry;
                } else {
                    // Create directory and add to parent
                    let dir_lba = self.alloc_blocks(1)?;
                    let empty = DirectoryEntry {
                        entries: [HashPointer { lba: 0, offset: 0 }; HASHPTRS_PER_BLOCK],
                        _pad: [0; DIRBLOCK_PADDING],
                    };
                    self.write_block(dir_lba, &empty)?;

                    let dir_ptr = self.write_hash_entry(&name_bytes, EntryType::Directory, dir_lba, 0, 0, 0)?;

                    let mut parent_link = LinkEntry { ptr: parent_entry.link_ptr, size: parent_entry.link_size };
                    self.dir_add_child(parent_entry.start_block, &mut parent_link, dir_ptr)?;

                    // Update parent's link on disk if it changed
                    if parent_link.ptr != parent_entry.link_ptr || parent_link.size != parent_entry.link_size {
                        parent_entry.link_ptr = parent_link.ptr;
                        parent_entry.link_size = parent_link.size;
                        self.hash_ptr_write(&parent_ptr, &parent_entry)?;
                    }

                    parent_ptr = dir_ptr;
                    parent_entry = self.hash_ptr_read(&dir_ptr)?;
                }
            }
            filename = comp;
            walked = true;
        }

        Ok((filename.to_string(), parent_ptr, parent_entry))
    }

    /// Returns (filename, parent_dir_entry, root_dir_entry).
    fn resolve_root_and_parent(&mut self, path: &str) -> Result<(String, HashEntry, HashEntry), String> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Err("Cannot resolve root".into());
        }

        // Find root directory entry
        let root_name = Self::name_to_bytes("/")?;
        let (_root_ptr, root_entry) = self.find_entry(&root_name)?;

        let mut current = root_entry.clone();
        let mut filename = "";

        let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        if components.is_empty() {
            return Err("Empty path".into());
        }

        for (i, comp) in components.iter().enumerate() {
            if i == components.len() - 1 {
                filename = comp;
                break;
            }
            let (_child_ptr, child_entry) = self.dir_find_child(&current, comp)?;
            if child_entry.type_ != EntryType::Directory as u8 {
                return Err(format!("'{}' is not a directory", comp));
            }
            current = child_entry;
        }

        Ok((filename.to_string(), current, root_entry))
    }

    // -- Public API --

    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        let (filename, parent_ptr, mut parent_entry) = self.resolve_parent(path)?;
        let name_bytes = Self::name_to_bytes(&filename)?;

        // Check if file already exists
        let existing = self.find_entry(&name_bytes);
        let old_file = if let Ok((_ptr, entry)) = &existing {
            if entry.type_ == EntryType::Directory as u8 {
                return Err(format!("'{}' is a directory", filename));
            }
            Some(*entry)
        } else {
            None
        };

        let new_size = data.len() as u64;

        if let Some(old) = old_file {
            let old_lbas = old.file_size.div_ceil(LBA_SIZE);

            if new_size <= old.file_size {
                self.free_link_chain(LinkEntry { ptr: old.link_ptr, size: old.link_size })?;

                let new_lbas = new_size.div_ceil(LBA_SIZE);
                for i in 0..new_lbas {
                    let offset = (i * LBA_SIZE) as usize;
                    let end = std::cmp::min(offset + LBA_SIZE as usize, data.len());
                    let mut buf = vec![0u8; LBA_SIZE as usize];
                    buf[..end - offset].copy_from_slice(&data[offset..end]);
                    self.write_raw(old.start_block + i, &buf)?;
                }
                if new_lbas < old_lbas {
                    self.free_blocks(old.start_block + new_lbas, old_lbas - new_lbas)?;
                }

                self.write_hash_entry(&name_bytes, EntryType::File, old.start_block, new_size, 0, 0)?;
            } else {
                let new_lbas = new_size.div_ceil(LBA_SIZE);

                if let Ok(new_start) = self.alloc_blocks(new_lbas) {
                    self.free_link_chain(LinkEntry { ptr: old.link_ptr, size: old.link_size })?;
                    self.free_blocks(old.start_block, old_lbas)?;

                    for i in 0..new_lbas {
                        let offset = (i * LBA_SIZE) as usize;
                        let end = std::cmp::min(offset + LBA_SIZE as usize, data.len());
                        let mut buf = vec![0u8; LBA_SIZE as usize];
                        buf[..end - offset].copy_from_slice(&data[offset..end]);
                        self.write_raw(new_start + i, &buf)?;
                    }

                    self.write_hash_entry(&name_bytes, EntryType::File, new_start, new_size, 0, 0)?;
                } else {
                    let old_lbas_write = old.file_size.div_ceil(LBA_SIZE);
                    for i in 0..old_lbas_write {
                        let offset = (i * LBA_SIZE) as usize;
                        let end = std::cmp::min(offset + LBA_SIZE as usize, data.len());
                        let mut buf = vec![0u8; LBA_SIZE as usize];
                        buf[..end - offset].copy_from_slice(&data[offset..end]);
                        self.write_raw(old.start_block + i, &buf)?;
                    }

                    let overflow_size = new_size - old.file_size;
                    let overflow_lbas = overflow_size.div_ceil(LBA_SIZE);
                    let overflow_start = self.alloc_blocks(overflow_lbas)?;

                    let mut off = old.file_size as usize;
                    for i in 0..overflow_lbas {
                        let offset = off;
                        let end = std::cmp::min(offset + LBA_SIZE as usize, data.len());
                        let mut buf = vec![0u8; LBA_SIZE as usize];
                        let chunk = end - offset;
                        buf[..chunk].copy_from_slice(&data[offset..end]);
                        self.write_raw(overflow_start + i, &buf)?;
                        off += chunk;
                    }

                    let mut link = LinkEntry { ptr: old.link_ptr, size: old.link_size };
                    self.extend_link(&mut link, LinkEntry { ptr: overflow_start, size: overflow_size })?;

                    self.write_hash_entry(
                        &name_bytes, EntryType::File, old.start_block, new_size,
                        link.ptr, link.size,
                    )?;
                }
            }
        } else {
            // New file
            if data.is_empty() {
                self.write_hash_entry(&name_bytes, EntryType::File, self.superblock.data_start(), 0, 0, 0)?;
            } else {
                let blocks_needed = (data.len() as u64).div_ceil(LBA_SIZE);

                if let Ok(start_lba) = self.alloc_blocks(blocks_needed) {
                    for i in 0..blocks_needed {
                        let offset = (i * LBA_SIZE) as usize;
                        let end = std::cmp::min(offset + LBA_SIZE as usize, data.len());
                        let mut buf = vec![0u8; LBA_SIZE as usize];
                        buf[..end - offset].copy_from_slice(&data[offset..end]);
                        self.write_raw(start_lba + i, &buf)?;
                    }
                    self.write_hash_entry(&name_bytes, EntryType::File, start_lba, new_size, 0, 0)?;
                } else {
                    return Err("Disk full".into());
                }
            }

            // Add to parent directory
            let (child_ptr, _) = self.find_entry(&name_bytes)?;
            let mut parent_link = LinkEntry { ptr: parent_entry.link_ptr, size: parent_entry.link_size };
            self.dir_add_child(parent_entry.start_block, &mut parent_link, child_ptr)?;

            // Update parent's hash entry on disk if link chain changed
            if parent_link.ptr != parent_entry.link_ptr || parent_link.size != parent_entry.link_size {
                parent_entry.link_ptr = parent_link.ptr;
                parent_entry.link_size = parent_link.size;
                self.hash_ptr_write(&parent_ptr, &parent_entry)?;
            }
        }

        Ok(())
    }

    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Err("Cannot read root".into());
        }

        let (_filename, dir_entry, _root) = self.resolve_root_and_parent(path)?;
        let filename = path.rsplit('/').next().unwrap_or(path);

        let (_child_ptr, file_entry) = self.dir_find_child(&dir_entry, filename)?;
        if file_entry.type_ != EntryType::File as u8 {
            return Err(format!("'{}' is not a file", filename));
        }

        let mut data = Vec::new();
        let link = LinkEntry { ptr: file_entry.link_ptr, size: file_entry.link_size };
        self.read_link_data(&link, file_entry.start_block, file_entry.file_size, &mut data)?;
        Ok(data)
    }

    pub fn file_size(&mut self, path: &str) -> Result<u64, String> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Err("Cannot stat root".into());
        }

        let (_filename, dir_entry, _root) = self.resolve_root_and_parent(path)?;
        let filename = path.rsplit('/').next().unwrap_or(path);

        let (_child_ptr, file_entry) = self.dir_find_child(&dir_entry, filename)?;
        if file_entry.type_ != EntryType::File as u8 {
            return Err(format!("'{}' is not a file", filename));
        }

        let link = LinkEntry { ptr: file_entry.link_ptr, size: file_entry.link_size };
        self.total_file_size(file_entry.start_block, file_entry.file_size, &link)
    }

    pub fn list_directory(&mut self, path: &str) -> Result<Vec<String>, String> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            // List root
            let root_name = Self::name_to_bytes("/")?;
            let (_root_ptr, root_entry) = self.find_entry(&root_name)?;
            let children = self.dir_children(&root_entry)?;
            let mut names = Vec::new();
            for child_ptr in &children {
                let entry = self.hash_ptr_read(child_ptr)?;
                if entry.status == 1 {
                    names.push(Self::bytes_to_name(&entry.name));
                }
            }
            return Ok(names);
        }

        let (_filename, dir_entry, _root) = self.resolve_root_and_parent(path)?;
        let children = self.dir_children(&dir_entry)?;
        let mut names = Vec::new();
        for child_ptr in &children {
            let entry = self.hash_ptr_read(child_ptr)?;
            if entry.status == 1 {
                names.push(Self::bytes_to_name(&entry.name));
            }
        }
        Ok(names)
    }

    pub fn remove_file(&mut self, path: &str) -> Result<(), String> {
        let (filename, _parent_ptr, parent_entry) = self.resolve_parent(path)?;
        let name_bytes = Self::name_to_bytes(&filename)?;

        let (child_ptr, file_entry) = self.find_entry(&name_bytes)?;
        if file_entry.type_ != EntryType::File as u8 {
            return Err(format!("'{}' is not a file", filename));
        }

        // Remove from parent directory
        self.dir_remove_child(&parent_entry, &child_ptr)?;

        // Free data blocks
        let data_lbas = file_entry.file_size.div_ceil(LBA_SIZE);
        self.free_blocks(file_entry.start_block, data_lbas)?;

        // Free link chain
        self.free_link_chain(LinkEntry { ptr: file_entry.link_ptr, size: file_entry.link_size })?;

        // Tombstone hash entry
        self.tombstone(&child_ptr)
    }

    pub fn move_(&mut self, src: &str, dest: &str) -> Result<(), String> {
        let src = src.trim_start_matches('/');
        let dest = dest.trim_start_matches('/');
        if src.is_empty() { return Err("Cannot move root".into()); }

        // Resolve source
        let (_src_name, src_dir_entry, _root) = self.resolve_root_and_parent(src)?;
        let src_filename = src.rsplit('/').next().unwrap_or(src);
        let (src_ptr, src_entry) = self.dir_find_child(&src_dir_entry, src_filename)?;

        // Resolve dest: could be an existing directory or a rename
        let (dest_dir_entry, new_name) = {
            let root_name = Self::name_to_bytes("/")?;
            let (_root_ptr, root_entry) = self.find_entry(&root_name)?;

            let mut current = root_entry;
            let dest_components: Vec<&str> = dest.split('/').filter(|c| !c.is_empty()).collect();

            if dest_components.is_empty() {
                return Err("Invalid destination".into());
            }

            // Walk to parent of dest
            for comp in &dest_components[..dest_components.len() - 1] {
                let (_child_ptr, child_entry) = self.dir_find_child(&current, comp)?;
                if child_entry.type_ != EntryType::Directory as u8 {
                    return Err(format!("'{}' is not a directory", comp));
                }
                current = child_entry;
            }

            let dest_last = *dest_components.last().unwrap();

            // Check if dest_last is an existing directory
            if let Ok((_dp, de)) = self.dir_find_child(&current, dest_last) {
                if de.type_ == EntryType::Directory as u8 {
                    (de, src_filename.to_string())
                } else {
                    (current, dest_last.to_string())
                }
            } else {
                (current, dest_last.to_string())
            }
        };

        // No-op check
        if src_dir_entry.start_block == dest_dir_entry.start_block && new_name == src_filename {
            return Ok(());
        }

        // Remove from source parent
        self.dir_remove_child(&src_dir_entry, &src_ptr)?;

        // Rename if needed
        let new_name_bytes = Self::name_to_bytes(&new_name)?;
        let new_ptr = if new_name != src_filename {
            // Tombstone old, create new
            self.tombstone(&src_ptr)?;
            let new_ptr = self.find_free_slot(&new_name_bytes)?;
            let mut entry = src_entry;
            entry.name = new_name_bytes;
            entry.status = 1;
            self.hash_ptr_write(&new_ptr, &entry)?;
            new_ptr
        } else {
            src_ptr
        };

        // Add to dest directory
        let dest = dest_dir_entry;
        let mut dest_link = LinkEntry { ptr: dest.link_ptr, size: dest.link_size };
        self.dir_add_child(dest.start_block, &mut dest_link, new_ptr)?;

        Ok(())
    }
}
