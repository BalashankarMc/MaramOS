//! Set-associative block cache for storage devices.
//!
//! Implements a 4-way set-associative LRU-style cache over raw LBAs with
//! frequency-based LFU victim selection and epoch-based counter resets.
//! Wraps any [`StorageDrive`] to transparently accelerate repeated reads.

use alloc::boxed::Box;
use alloc::vec;

use super::LBA_SIZE;
use crate::memory::KMemory;

const CACHE_WAYS: usize = 4;
const CACHE_SETS: usize = 16;
const CACHE_SLOTS: usize = CACHE_SETS * CACHE_WAYS;  // 64
const CACHE_PAGES: usize = 8;
const FREQ_WAYS: usize = 4;
const FREQ_SETS: usize = 64;
const FREQ_TABLE_SIZE: usize = FREQ_SETS * FREQ_WAYS;  // 256

const EPOCH_ACCESSES: u16 = 256;
#[derive(Copy, Clone, Debug)]
#[repr(align(64))] 
struct CacheEntry {
    lba_number: Option<u64>,
}

#[derive(Debug)]
pub struct BlockCache {
    cache_page: crate::memory::PhysPage,
    slots: [CacheEntry; CACHE_SLOTS],
}

#[derive(Copy, Clone, Debug)]
#[repr(align(64))] 
struct BlockInfo {
    lba_number: Option<u64>,
    times_accessed: u8,
}

#[derive(Debug)]
pub struct BlockData {
    entries: Box<[BlockInfo]>,
    total: u16,
}

impl BlockCache {
    pub fn new() -> Self {
        BlockCache {
            cache_page: KMemory::alloc_pages(CACHE_PAGES),
            slots: [CacheEntry { lba_number: None }; CACHE_SLOTS],
        }
    }

    pub fn get_lba(&self, lba: u64) -> Option<[u8; LBA_SIZE as usize]> {
        let set = (lba as usize) % CACHE_SETS;
        let base = set * CACHE_WAYS;
        for way in 0..CACHE_WAYS {
            let idx = base + way;
            if self.slots[idx].lba_number == Some(lba) {
                let data_ptr = self.cache_page.get_virt_addr().as_ptr::<u8>();
                let mut data = [0u8; LBA_SIZE as usize];
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        data_ptr.add(idx * LBA_SIZE as usize),
                        data.as_mut_ptr(),
                        LBA_SIZE as usize,
                    );
                }
                return Some(data);
            }
        }
        None
    }

    fn copy_to_slot(&mut self, idx: usize, data: &[u8; LBA_SIZE as usize]) {
        let base = self.cache_page.get_virt_addr().as_mut_ptr::<u8>();
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr(),
                base.add(idx * LBA_SIZE as usize),
                LBA_SIZE as usize,
            );
        }
    }

    pub fn update_lba(&mut self, lba: u64, data: &[u8; LBA_SIZE as usize]) {
        let set = (lba as usize) % CACHE_SETS;
        let base = set * CACHE_WAYS;
        for way in 0..CACHE_WAYS {
            let idx = base + way;
            if self.slots[idx].lba_number == Some(lba) {
                self.copy_to_slot(idx, data);
                return;
            }
        }
    }

    pub fn invalidate(&mut self, lba: u64) {
        let set = (lba as usize) % CACHE_SETS;
        let base = set * CACHE_WAYS;
        for way in 0..CACHE_WAYS {
            let idx = base + way;
            if self.slots[idx].lba_number == Some(lba) {
                self.slots[idx].lba_number = None;
                return;
            }
        }
    }

    pub fn insert_lba(&mut self, lba: u64, data: &[u8; LBA_SIZE as usize], freqs: &BlockData) {
        let set = (lba as usize) % CACHE_SETS;
        let base = set * CACHE_WAYS;

        for way in 0..CACHE_WAYS {
            let idx = base + way;
            if self.slots[idx].lba_number == Some(lba) {
                self.copy_to_slot(idx, data);
                return;
            }
        }

        for way in 0..CACHE_WAYS {
            let idx = base + way;
            if self.slots[idx].lba_number.is_none() {
                self.slots[idx].lba_number = Some(lba);
                self.copy_to_slot(idx, data);
                return;
            }
        }

        let mut victim = base;
        let mut min_count = u8::MAX;
        for way in 0..CACHE_WAYS {
            let idx = base + way;
            if let Some(cached_lba) = self.slots[idx].lba_number {
                let cnt = freqs.get_count(cached_lba);
                if cnt < min_count {
                    min_count = cnt;
                    victim = idx;
                }
            }
        }

        let new_count = freqs.get_count(lba);
        if min_count == 0 || new_count > min_count {
            self.slots[victim].lba_number = Some(lba);
            self.copy_to_slot(victim, data);
        }
    }
}

impl BlockData {
    pub fn new() -> Self {
        let entry = BlockInfo {
            lba_number: None,
            times_accessed: 0,
        };
        BlockData {
            entries: vec![entry; FREQ_TABLE_SIZE].into_boxed_slice(),
            total: 0,
        }
    }

    pub fn record_access(&mut self, lba: u64) {
        let set = (lba as usize) % FREQ_SETS;
        let base = set * FREQ_WAYS;

        // Phase 1: hit
        for way in 0..FREQ_WAYS {
            let idx = base + way;
            if self.entries[idx].lba_number == Some(lba) {
                self.entries[idx].times_accessed = self.entries[idx].times_accessed.saturating_add(1);
                self.total = self.total.saturating_add(1);
                if self.total >= EPOCH_ACCESSES {
                    self.reset();
                }
                return;
            }
        }

        // Phase 2: empty slot
        for way in 0..FREQ_WAYS {
            let idx = base + way;
            if self.entries[idx].lba_number.is_none() {
                self.entries[idx].lba_number = Some(lba);
                self.entries[idx].times_accessed = 1;
                self.total = self.total.saturating_add(1);
                if self.total >= EPOCH_ACCESSES {
                    self.reset();
                }
                return;
            }
        }

        // Phase 3: all 4 occupied by other LBAs → evict the least-used in this set
        let mut victim = base;
        let mut min_count = u8::MAX;
        for way in 0..FREQ_WAYS {
            let idx = base + way;
            if self.entries[idx].times_accessed < min_count {
                min_count = self.entries[idx].times_accessed;
                victim = idx;
            }
        }
        self.entries[victim].lba_number = Some(lba);
        self.entries[victim].times_accessed = 1;
        self.total = self.total.saturating_add(1);
        if self.total >= EPOCH_ACCESSES {
            self.reset();
        }
    }

    pub fn get_count(&self, lba: u64) -> u8 {
        let set = (lba as usize) % FREQ_SETS;
        let base = set * FREQ_WAYS;
        for way in 0..FREQ_WAYS {
            let idx = base + way;
            if self.entries[idx].lba_number == Some(lba) {
                return self.entries[idx].times_accessed;
            }
        }
        0
    }

    fn reset(&mut self) {
        for entry in &mut self.entries {
            entry.lba_number = None;
            entry.times_accessed = 0;
        }
        self.total = 0;
    }
}
