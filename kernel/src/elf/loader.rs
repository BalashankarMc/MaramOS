use core::ops::Range;

use alloc::string::String;
use alloc::vec::Vec;
use elf::ElfBytes;
use elf::abi::{EM_X86_64, ET_DYN, ET_EXEC, PT_LOAD};
use elf::endian::AnyEndian;
use elf::file::Class;
use x86_64::VirtAddr;
use x86_64::structures::paging::PageTableFlags;

use crate::KResult;
use crate::fs::FileSystem;
use crate::memory::MemoryError::{InvalidMapping, OutOfBounds};
use crate::memory::{MemoryError, PAGE_SIZE, PhysPage, VMemAllocator, VirtualRegion, new_user_allocator};

use super::{LoadedBinary, LoaderError};

const USER_END: usize = 0x0000_8000_0000_0000;

struct Segment {
    virt_addr: usize,
    start: usize,
    end: usize,
    file_range: Range<usize>,
    mem_size: usize,
    page_offset: usize,
    flags: PageTableFlags,
    executable: bool
}

pub fn load(fs: &mut dyn FileSystem, path: String) -> KResult<LoadedBinary> {
    let (page, data_len) = read_file(fs, path)?;
    let data = &page.as_byte_slice()[..data_len as usize];
    let elf = ElfBytes::<AnyEndian>::minimal_parse(data).map_err(LoaderError::ParseError)?;

    let base = parse_header(&elf)?;
    let mut segments = parse_segments(&elf, data.len(), base)?;
    check_no_overlap(&mut segments)?;
    let entry = validate_entry(elf.ehdr.e_entry as usize, base, &segments)?;

    let mut space = new_user_allocator()?;
    let mut pages = Vec::with_capacity(segments.len());

    for seg in segments { pages.push(map_segment(&mut space.0, &seg, data)?); }

    Ok(LoadedBinary { entry, address_space: space, segments: pages })
}

fn map_segment(allocator: &mut VMemAllocator, seg: &Segment, data: &[u8]) -> KResult<PhysPage> {
    let count = (seg.end - seg.start).div_ceil(PAGE_SIZE);
    let mut page = PhysPage::new(count)?;

    let dst = page.as_byte_slice_mut();
    let src = &data[seg.file_range.clone()];
    dst[seg.page_offset..seg.page_offset + src.len()].copy_from_slice(src);

    let region = VirtualRegion { start: VirtAddr::new(seg.start as u64), size: (seg.end - seg.start) as u64 };
    allocator.map_fixed(region, page.phys(), seg.flags)?;
    Ok(page)
}

fn validate_entry(entry: usize, base: usize, segments: &[Segment]) -> Result<VirtAddr, LoaderError> {
    let entry = base.checked_add(entry).ok_or(MemoryError::OutOfBounds)?;
    let in_exec = segments.iter().any(|s| s.executable && (s.virt_addr..s.virt_addr + s.mem_size).contains(&entry));

    if !in_exec { return Err(LoaderError::MemoryError(InvalidMapping)) }

    let virt_addr = VirtAddr::try_new(entry as u64).map_err(|_| MemoryError::InvalidMapping)?;
    Ok(virt_addr)
}

fn check_no_overlap(segments: &mut [Segment]) -> Result<(), LoaderError> {
    segments.sort_unstable_by_key(|s| s.start);

    for pair in segments.windows(2) {
        if pair[0].end > pair[1].start { return Err(LoaderError::OverlappingSegments) }
    }

    Ok(())
}

fn parse_segments(elf: &ElfBytes<AnyEndian>, data_len: usize, base: usize) -> Result<Vec<Segment>, LoaderError> {
    let phdrs = elf.segments().ok_or(LoaderError::NoSegments)?;
    let mut res = Vec::new();

    for phdr in phdrs {
        if phdr.p_type != PT_LOAD { continue } 
        if phdr.p_filesz > phdr.p_memsz { return Err(LoaderError::BadSegments) }
        if phdr.p_memsz == 0 { continue }

        let file_start = phdr.p_offset as usize;
        let file_end = file_start
            .checked_add(phdr.p_filesz as usize)
            .ok_or(LoaderError::MemoryError(OutOfBounds))?;

        if file_end > data_len { return Err(LoaderError::MemoryError(OutOfBounds)) }

        let virt_addr = base.checked_add(phdr.p_vaddr as usize).ok_or(MemoryError::OutOfBounds)?;
        let mem_end = virt_addr.checked_add(phdr.p_memsz as usize).ok_or(MemoryError::OutOfBounds)?;
        let start = (virt_addr / PAGE_SIZE) * PAGE_SIZE;
        let end = mem_end.checked_next_multiple_of(PAGE_SIZE).ok_or(MemoryError::OutOfBounds)?;

        if end > USER_END { return Err(LoaderError::MemoryError(OutOfBounds)) }

        let file_range = file_start..file_end;
        let mem_size = phdr.p_memsz as usize;
        let page_offset = virt_addr - start;
        let flags = page_flags(phdr.p_flags);
        let executable = !flags.contains(PageTableFlags::NO_EXECUTE);

        res.push(Segment { virt_addr, start, end, file_range, mem_size, page_offset, flags, executable });
    }

    if res.is_empty() { return Err(LoaderError::NoSegments) }

    Ok(res)
}

fn parse_header(elf: &ElfBytes<AnyEndian>) -> Result<usize, LoaderError> {
    if elf.ehdr.class != Class::ELF64 { return Err(LoaderError::BadClass) }
    if elf.ehdr.e_machine != EM_X86_64 { return Err(LoaderError::BadTarget) }
    if ![ET_EXEC, ET_DYN].contains(&elf.ehdr.e_type) { return Err(LoaderError::BadType) }

    if elf.ehdr.e_type == ET_EXEC { Ok(0) } else { Ok(0x400_000) }
}

fn read_file(fs: &mut dyn FileSystem, path: String) -> KResult<(PhysPage, u64)> {
    let file_size = fs.file_size(path.clone())? as usize;
    let page = PhysPage::new(file_size.div_ceil(PAGE_SIZE))?;
    let bytes_read = fs.read(path, &page)?;

    Ok((page, bytes_read))
}

fn page_flags(flags: u32) -> PageTableFlags {
    let mut res = PageTableFlags::empty();
    if flags & 1 == 0 { res |= PageTableFlags::NO_EXECUTE }
    if (flags >> 1) & 1 == 1 { res |= PageTableFlags::WRITABLE }
    res | PageTableFlags::USER_ACCESSIBLE
}