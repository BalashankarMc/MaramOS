//! Kernel page table management.
//!
//! MMIO mapping/unmapping, user page mapping (4K/2M/1G pages), user page
//! table creation (copies kernel half), address space teardown, and page
//! fault checking for demand paging.

use super::{paging::alloc_frames, phys_to_virt, PhysPage};

use crate::library::LateInit;

use core::{sync::atomic::{AtomicU64, Ordering}};
use x86_64::{
    PhysAddr, VirtAddr,
    registers::control::Cr3,
    structures::paging::{
        FrameAllocator, Page, PageSize, PageTable, PageTableFlags, PhysFrame,
        Size4KiB, Size2MiB, Size1GiB,
        mapper::{MappedPageTable, Mapper, PageTableFrameMapping},
        page_table::PageTableEntry
    },
};

static KERNEL_L4: LateInit<PhysFrame> = LateInit::new();
static NEXT_PAGE_ID: AtomicU64 = AtomicU64::new(0);

const MMIO_START: u64 = 0xFFFF_FE00_0000_0000;
const MMIO_END: u64 = 0xFFFF_FFFF_8000_0000;
const MMIO_PAGES: u64 = (MMIO_END - MMIO_START) / Size4KiB::SIZE;

pub struct PMMFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for PMMFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> { Some(PhysFrame::containing_address(alloc_frames(0))) }
}

pub unsafe fn active_l4_table() -> &'static mut PageTable {
    let (frame, _) = Cr3::read();
    let phys = frame.start_address();
    let virt = phys_to_virt(phys);
    unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) }
}

pub struct PhysOffset(pub u64);

unsafe impl PageTableFrameMapping for PhysOffset {
    fn frame_to_pointer(&self, frame: PhysFrame) -> *mut PageTable {
        let virt = frame.start_address().as_u64() + self.0;
        virt as *mut PageTable
    }
}

#[allow(dead_code)]
pub enum PageMapping {
    Size4K(Page<Size4KiB>, PhysFrame<Size4KiB>),
    Size2M(Page<Size2MiB>, PhysFrame<Size2MiB>),
    Size1G(Page<Size1GiB>, PhysFrame<Size1GiB>),
}

pub unsafe fn map_sized_page(mapping: PageMapping, flags: PageTableFlags) {
    let l4 = unsafe { active_l4_table() };
    let mut mapper = unsafe { MappedPageTable::new(l4, PhysOffset(*super::PHYS_OFFSET)) };
    let mut allocator = PMMFrameAllocator;
    match mapping {
        PageMapping::Size4K(page, frame) => unsafe {
            mapper.map_to(page, frame, flags, &mut allocator).expect("map_to failed").flush();
        },
        PageMapping::Size2M(page, frame) => unsafe {
            mapper.map_to(page, frame, flags, &mut allocator).expect("map_to failed").flush();
        },
        PageMapping::Size1G(page, frame) => unsafe {
            mapper.map_to(page, frame, flags, &mut allocator).expect("map_to failed").flush();
        },
    }
}

pub unsafe fn map_mmio(phys: PhysAddr, pages: usize) -> VirtAddr {
    let offset = NEXT_PAGE_ID.fetch_add(pages as u64, Ordering::AcqRel);
    if offset >= MMIO_PAGES { panic!("Out of memory for MMIO!") }
    let virt_base = VirtAddr::new(MMIO_START + offset * Size4KiB::SIZE);

    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH;

    for i in 0..pages {
        let page: Page<Size4KiB> = Page::containing_address(virt_base + (i as u64) * Size4KiB::SIZE);
        let frame = PhysFrame::containing_address(phys + (i as u64) * Size4KiB::SIZE);
        unsafe { map_sized_page(PageMapping::Size4K(page, frame), flags) }
    }

    virt_base
}

pub unsafe fn unmap_page(page: Page<Size4KiB>) {
    let l4 = unsafe { active_l4_table() };
    let mut mapper = unsafe { MappedPageTable::new(l4, PhysOffset(*super::PHYS_OFFSET)) };

    mapper.unmap(page).expect("unmap failed").1.flush();
    crate::ipi::tlb_shootdown(page.start_address());
}

pub unsafe fn unmap_mmio(virt: VirtAddr, pages: usize) {
    for i in 0..pages {
        let page = Page::containing_address(virt + (i as u64) * Size4KiB::SIZE);
        unsafe { unmap_page(page) };
    }
}

pub fn new_user_page_table() -> PhysAddr {
    let new_l4_phys = alloc_frames(0);
    let l4_virt = phys_to_virt(new_l4_phys);
    let new_l4 = unsafe { &mut *l4_virt.as_mut_ptr::<PageTable>() };

    let kernel_l4_virt = phys_to_virt(KERNEL_L4.start_address());
    let kernel_l4 = unsafe { &*kernel_l4_virt.as_ptr::<PageTable>() };

    for i in 256..512 {
        new_l4[i] = kernel_l4[i].clone();
    }

    new_l4_phys
}

pub unsafe fn map_user_page(l4_phys: PhysAddr, virt: VirtAddr, page: &PhysPage, mut flags: PageTableFlags) {
    let l4_virt = phys_to_virt(l4_phys);
    let l4 = unsafe { &mut *l4_virt.as_mut_ptr::<PageTable>() };

    let mut mapper = unsafe { MappedPageTable::new(l4, PhysOffset(*super::PHYS_OFFSET)) };
    let mut allocator = PMMFrameAllocator;

    let pages = page.size();
    let phys = page.get_phys_address();
    
    let mut rem = pages;
    let mut curr_virt = virt;
    let mut curr_phys = phys;

    flags |= PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;

    if rem >= 0x40000 && phys.is_aligned(Size1GiB::SIZE) {
        let count = (rem / 0x40000) as u64;
        for i in 0..count {
            let offset = i * Size1GiB::SIZE;
            let page = Page::<Size1GiB>::containing_address(curr_virt + offset);
            let frame = PhysFrame::<Size1GiB>::containing_address(curr_phys + offset);
            unsafe { mapper.map_to(page, frame, flags | PageTableFlags::HUGE_PAGE, &mut allocator)
                .expect("Failed to map User 1GiB Page").flush() }
        }

        let mapped = count as usize * 0x40000;
        rem -= mapped;
        curr_phys += (mapped as u64) * Size4KiB::SIZE;
        curr_virt += (mapped as u64) * Size4KiB::SIZE;
    }

    if rem >= 0x200 {
        let count = (rem / 0x200) as u64;
        for i in 0..count {
            let offset = i * Size2MiB::SIZE;
            let page = Page::<Size2MiB>::containing_address(curr_virt + offset);
            let frame = PhysFrame::<Size2MiB>::containing_address(curr_phys + offset);
            unsafe { mapper.map_to(page, frame, flags | PageTableFlags::HUGE_PAGE, &mut allocator)
                .expect("Failed to map User 2MiB Page").flush() }
        }

        let mapped = count as usize * 0x200;
        rem -= mapped;
        curr_phys += (mapped as u64) * Size4KiB::SIZE;
        curr_virt += (mapped as u64) * Size4KiB::SIZE;
    }

    for i in 0..rem as u64 {
        let offset = i * Size4KiB::SIZE;
        let page = Page::<Size4KiB>::containing_address(curr_virt + offset);
        let frame = PhysFrame::<Size4KiB>::containing_address(curr_phys + offset);
        unsafe { mapper.map_to(page, frame, flags, &mut allocator).expect("Failed to allocate user 4KiB page").flush() }
    }
}

pub fn is_user_page_mapped(addr: u64) -> bool {
    let l4 = unsafe { active_l4_table() };

    let l4_i = (addr >> 39) as usize & 0x1FF;
    let l3_i = (addr >> 30) as usize & 0x1FF;
    let l2_i = (addr >> 21) as usize & 0x1FF;
    let l1_i = (addr >> 12) as usize & 0x1FF;

    let l4e: &PageTableEntry = &l4[l4_i];
    if l4e.is_unused() { return false }
    let l3 = unsafe { &*phys_to_virt(l4e.addr()).as_ptr::<PageTable>() };

    let l3e = &l3[l3_i];
    if l3e.is_unused() { return false }
    if l3e.flags().contains(PageTableFlags::HUGE_PAGE) { return true }
    let l2 = unsafe { &*phys_to_virt(l3e.addr()).as_ptr::<PageTable>() };

    let l2e = &l2[l2_i];
    if l2e.is_unused() { return false }
    if l2e.flags().contains(PageTableFlags::HUGE_PAGE) { return true }
    let l1 = unsafe { &*phys_to_virt(l2e.addr()).as_ptr::<PageTable>() };

    !l1[l1_i].is_unused()
}

pub fn free_user_address_space(l4_phys: PhysAddr) {
    let l4_virt = phys_to_virt(l4_phys);
    let l4 = unsafe { &*l4_virt.as_ptr::<PageTable>() };

    for l4_i in 0..256 {
        let l4e = &l4[l4_i];
        if l4e.is_unused() { continue }
        let l3_virt = phys_to_virt(l4e.addr());
        let l3 = unsafe { &*l3_virt.as_ptr::<PageTable>() };

        for l3_i in 0..512 {
            let l3e = &l3[l3_i];
            if l3e.is_unused() { continue }
            if l3e.flags().contains(PageTableFlags::HUGE_PAGE) {
                super::paging::free_frames(l3e.addr(), 18);
                continue;
            }

            let l2_virt = phys_to_virt(l3e.addr());
            let l2 = unsafe { &*l2_virt.as_ptr::<PageTable>() };

            for l2_i in 0..512 {
                let l2e = &l2[l2_i];
                if l2e.is_unused() { continue }
                if l2e.flags().contains(PageTableFlags::HUGE_PAGE) {
                    super::paging::free_frames(l2e.addr(), 9);
                    continue;
                }

                let l1_virt = phys_to_virt(l2e.addr());
                let l1 = unsafe { &*l1_virt.as_ptr::<PageTable>() };

                for l1_i in 0..512 {
                    let l1e = &l1[l1_i];
                    if l1e.is_unused() { continue }
                    super::paging::free_frames(l1e.addr(), 0);
                }
                super::paging::free_frames(l2e.addr(), 0);
            }
            super::paging::free_frames(l3e.addr(), 0);
        }
        super::paging::free_frames(l4e.addr(), 0);
    }
    super::paging::free_frames(l4_phys, 0);
}

pub fn init() { let (frame, _) = Cr3::read(); KERNEL_L4.init(frame); }

pub fn get_kernel_l4<'a>() -> &'a PhysFrame { &KERNEL_L4 }
