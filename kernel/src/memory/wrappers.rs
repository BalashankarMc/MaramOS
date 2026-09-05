use core::marker::PhantomData;

use x86_64::{PhysAddr, VirtAddr, structures::paging::PageTableFlags};

use crate::{errors::MemoryError, memory::{PAGE_SIZE, virt::VirtualRegion}};

use super::virt::KERNEL_ALLOCATOR;

/// A representation of physical page ranges
#[derive(Debug)]
pub struct PhysPage {
    start: VirtAddr,
    // Number of 4KiB Pages in this range
    pub count: usize,
    phys: PhysAddr
}

impl PhysPage {
    /// Allocate a new range of zeroed `count` pages
    pub fn new(count: usize) -> Result<Self, MemoryError> {
        let start = super::physical::alloc_pages(count).ok_or(MemoryError::OutOfMemory)?;
        Ok(Self {
            start,
            count,
            phys: unsafe { super::virt_to_phys(start) }
        })
    }

    /// Read some data of type `T` from `offset` bytes into the page.
    /// Returns Some(&T) on success and None on OOB
    pub fn read_data<T>(&self, offset: usize) -> Option<T> {
        if self.count * PAGE_SIZE < offset + size_of::<T>() { return None }
        if !offset.is_multiple_of(align_of::<T>()) { return None }

        // Safety: We did bounds checking and address validation already. Safe
        Some(unsafe { (self.start + offset as u64).as_ptr::<T>().read() })
    }

    /// Write some data of type `T` into the page after `offset` bytes into the page
    /// Returns true on success and false on OOB
    pub fn write_data<T>(&self, offset: usize, data: T) -> bool {
        if self.count * PAGE_SIZE < offset + size_of::<T>() { return false }

        let ptr = self.start + offset as u64;
        if !offset.is_multiple_of(align_of::<T>()) { return false }

        // Safety: We did bounds checking and address validation already. Safe
        unsafe { ptr.as_mut_ptr::<T>().write(data) }

        true
    }

    /// Zero this page range
    pub fn zero(&self) {
        // We know this range is mapped. Safe
        unsafe { super::physical::zero_pages(self.start, self.count) };
    }

    pub const fn leak(self) -> (VirtAddr, PhysAddr, usize) {
        let res = (self.start, self.phys, self.count);
        core::mem::forget(self);
        res
    }

    pub const fn address(&self) -> VirtAddr { self.start }
    pub const fn phys(&self) -> PhysAddr { self.phys }
}

impl Drop for PhysPage {
    fn drop(&mut self) {
        super::physical::free_frames(self.phys, self.count);
    }
}

/// A helper for Memory-Mapped I/O
#[derive(Debug)]
pub struct MMIORegion(VirtualRegion);

impl MMIORegion {
    /// Map `pages` pages starting at `phys` with MMIO flags
    pub fn new(phys: PhysAddr, pages: usize) -> Option<Self> {
        let mut lock = KERNEL_ALLOCATOR.lock();
        let region = lock.alloc(pages)?;
        let flags = PageTableFlags::NO_CACHE | PageTableFlags::NO_EXECUTE | PageTableFlags::GLOBAL |
            PageTableFlags::WRITABLE | PageTableFlags::WRITE_THROUGH;
            
        if lock.map(&region, phys, flags).is_err() {
            let _ = lock.free(region);
            return None
        }

        Some(Self(region))
    }

    /// Volatile read of T at byte `offset` from the base
    /// 
    /// # Return
    /// Returns `Some(T)` on success
    /// and `None` on OOB
    pub fn read<T>(&self, offset: usize) -> Option<T> {
        if offset + size_of::<T>() > self.0.size as usize { return None }
        if !offset.is_multiple_of(align_of::<T>()) { return None }

        let ptr = self.0.start + offset as u64;

        // Safety: As long as `self` is in scope, the memory region is guaranteed to be backed. Safe.
        Some(unsafe { ptr.as_ptr::<T>().read_volatile() })
    }

    /// Volatile write of T at byte `offset` from the base
    /// 
    /// # Returns
    /// Returns `true` on success and `false` on OOB
    pub fn write<T>(&self, offset: usize, val: T) -> bool {
        if offset + size_of::<T>() > self.0.size as usize { return false }
        if !offset.is_multiple_of(align_of::<T>()) { return false }
        
        let ptr = self.0.start + offset as u64;

        // Safety: As long as `self` is in scope, the memory region is guaranteed to be backed. Safe.
        unsafe { ptr.as_mut_ptr::<T>().write_volatile(val) };
        true
    }

    pub fn register<T>(&self, offset: usize) -> Option<MMIORegister<'_, T>> {
        if !offset.is_multiple_of(align_of::<T>()) { return None }
        if offset + size_of::<T>() > self.0.size as usize { return None }

        let ptr = self.0.start + offset as u64;
        Some(MMIORegister { ptr: ptr.as_mut_ptr(), _marker: PhantomData })
    }
}

/// Wrapper around a single MMIO Register
pub struct MMIORegister<'a, T> {
    ptr: *mut T,
    _marker: PhantomData<&'a mut T>
}

impl<T> MMIORegister<'_, T> {
    pub fn read(&self) -> T { unsafe { self.ptr.read_volatile() } }
    pub fn write(&self, val: T) { unsafe { self.ptr.write_volatile(val) } }
}