use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use elf::abi::{EM_X86_64, ET_DYN, ET_EXEC, PT_LOAD};
use elf::endian::AnyEndian;
use elf::file::Class;
use x86_64::VirtAddr;
use x86_64::structures::paging::PageTableFlags;

use crate::KResult;
use crate::fs::FileSystem;
use crate::memory::{MemoryError, PAGE_SIZE, PhysPage, VirtualRegion, new_user_allocator};

use super::{LoadedBinary, LoaderError};

pub fn load(fs: &mut Box<dyn FileSystem>, path: String) -> KResult<LoadedBinary> {
    let file_size = fs.file_size(path.clone())? as usize;
    let page = PhysPage::new(file_size.div_ceil(PAGE_SIZE))?;
    let bytes = fs.read(path, &page)?;

    let pointer = page.address().as_ptr::<u8>();

    // Safety: Convertion of a safe data range into a byte slice
    let data = unsafe { core::slice::from_raw_parts(pointer, bytes as usize) };

    let elf = elf::ElfBytes::<AnyEndian>::minimal_parse(data).map_err(LoaderError::ParseError)?;

    // Validation
    if elf.ehdr.class != Class::ELF64 { Err(LoaderError::BadClass)? }
    if elf.ehdr.e_machine != EM_X86_64 { Err(LoaderError::BadTarget)? }
    if elf.ehdr.e_type != ET_EXEC && elf.ehdr.e_type != ET_DYN { Err(LoaderError::BadType)? }

    let (mut allocator, l4_page) = new_user_allocator()?;
    let base: u64 = if elf.ehdr.e_type == elf::abi::ET_EXEC { 0 } else { 0x40_0000 };

    let phdrs = elf.segments().ok_or(LoaderError::NoSegments)?;
    let mut segments = Vec::new();

    for phdr in phdrs {
        if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 { continue } // Unsupported header

        // Get header info
        let virtaddr = base.checked_add(phdr.p_vaddr).ok_or(MemoryError::OutOfBounds)?;
        let size = phdr.p_filesz;
        let mem_size = phdr.p_memsz;
        let address = VirtAddr::try_new(virtaddr).map_err(|_| MemoryError::InvalidMapping)?;

        // Allocate page for header
        let page_start = address.align_down(PAGE_SIZE as u64);
        let page_end = (address + mem_size).align_up(PAGE_SIZE as u64);
        let count = (page_end - page_start).div_ceil(PAGE_SIZE as u64);
        let page = PhysPage::new(count as usize)?;

        // Copy header into page
        let offset = (address - page_start) as usize;

        unsafe {
            let dst = page.address().as_mut_ptr::<u8>().add(offset);
            let src = data.as_ptr().add(phdr.p_offset as usize);
            core::ptr::copy_nonoverlapping(src, dst, size as usize);
        }

        // Map segments
        let region = VirtualRegion { start: page_start, size: page_end - page_start };
        allocator.map(&region, page.phys(), page_flags(phdr.p_flags))?;
        segments.push(page);
    }

    Ok(LoadedBinary {
        entry: VirtAddr::new(elf.ehdr.e_entry + base),
        address_space: (allocator, l4_page),
        segments
    })
}

fn page_flags(flags: u32) -> PageTableFlags {
    let mut res = PageTableFlags::empty();
    if flags & 1 == 0 { res |= PageTableFlags::NO_EXECUTE }
    if (flags >> 1) & 1 == 1 { res |= PageTableFlags::WRITABLE }
    res | PageTableFlags::USER_ACCESSIBLE
}