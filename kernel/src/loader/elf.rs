//! ELF binary loader.
//!
//! Reads an ELF file from LemonFS, validates the ELF header, maps LOAD
//! segments into a new user page table with demand-paged VMAs, and
//! sets up the user stack and syscall buffer.

use super::*;

use crate::{
    boot::FS,
    fs::{FSError, FileSystem},
    memory::{KMemory, PAGE_SIZE, PhysPage, VMABacking, VirtualMemoryArea}
};

use xmas_elf::{
    ElfFile,
    header::Type as HType,
    program::{Flags as PFlags, Type as PType},
};

use alloc::vec::Vec;
use x86_64::structures::paging::PageTableFlags;
use x86_64::VirtAddr;

fn load_file(path: &str) -> Result<(PhysPage, u64), FSError> {
    let fs = FS.get_mut();
    let file_size = fs.file_size(path)?;
    let file_pages = (file_size as usize).div_ceil(PAGE_SIZE);

    let mut pages = KMemory::alloc_pages(file_pages);
    fs.read_file(path, &mut pages)?;

    Ok((pages, file_size))
}

/// Reads the page(s) parses as ELF and checks validity as an executable
fn parse_elf(pages: &PhysPage, file_size: usize) -> Result<ElfFile<'_>, LoaderError> {
    let addr = pages.get_virt_addr().as_ptr::<u8>();
    let slice = unsafe { core::slice::from_raw_parts(addr, file_size) };
    let elf = ElfFile::new(slice).map_err(|_| LoaderError::InvalidFile)?;

    let t = elf.header.pt2.type_().as_type();
    if t != HType::Executable && t != HType::SharedObject {
        return Err(LoaderError::NotExecutable);
    }

    Ok(elf)
}

fn get_page_flags(elf_flags: PFlags) -> PageTableFlags {
    let mut page_flags = PageTableFlags::empty();

    if !elf_flags.is_execute() { page_flags |= PageTableFlags::NO_EXECUTE }
    if elf_flags.is_write() { page_flags |= PageTableFlags::WRITABLE }

    page_flags
}

pub fn load_elf(path: &str) -> Result<LoadedBinary, LoaderError> {
    let (elf_pages, file_size) = load_file(path).map_err(|_| LoaderError::IOError)?;

    let elf = parse_elf(&elf_pages, file_size as usize)?;

    let user_page_table = KMemory::new_user_page_table();
    let mut vmas = Vec::new();

    for ph in elf.program_iter() {
        if ph.get_type() != Ok(PType::Load) { continue }

        let virt_addr = ph.virtual_addr();
        let segment_base = virt_addr & !0xFFF;
        let segment_top = ((virt_addr + ph.mem_size()) + 0xFFF) & !0xFFF;

        let data_off = ph.virtual_addr() - segment_base;
        let cache_size = (data_off + ph.file_size() + 0xFFF) as usize / PAGE_SIZE;
        let cache = KMemory::alloc_pages(cache_size);

        if ph.file_size() == 0 {
            // BSS Segment
            vmas.push(VirtualMemoryArea {
                start: segment_base,
                end: segment_top,
                perms: get_page_flags(ph.flags()),
                backing: VMABacking::Anonymous,
            });
            continue;
        }

        let src = &elf.input[ph.offset() as usize..][..ph.file_size() as usize];
        let dst = cache.get_virt_addr().as_mut_ptr::<u8>();
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.as_ptr(),
                dst.add(data_off as usize),
                ph.file_size() as usize,
            );
        }

        vmas.push(VirtualMemoryArea {
            start: segment_base,
            end: segment_top,
            perms: get_page_flags(ph.flags()),
            backing: VMABacking::File {
                cache,
                data_offset: data_off,
                file_size: ph.file_size(),
            },
        });
    }

    vmas.push(VirtualMemoryArea {
        start: (USER_STACK_TOP - USER_STACK_SIZE + PAGE_SIZE) as u64,
        end: USER_STACK_TOP as u64,
        perms: PageTableFlags::WRITABLE,
        backing: VMABacking::Anonymous,
    });

    let syscall_page = KMemory::alloc_pages(SYSCALL_BUFFER_SIZE / PAGE_SIZE); // 2 MiB
    KMemory::map_user_page(
        user_page_table,
        VirtAddr::new(SYSCALL_BUF_ADDR),
        &syscall_page,
        PageTableFlags::WRITABLE
    );

    Ok(LoadedBinary {
        page_table: user_page_table,
        entry: elf.header.pt2.entry_point(),
        user_sp: USER_STACK_TOP as u64,
        vmas,
        syscall_buffer: syscall_page
    })
}
