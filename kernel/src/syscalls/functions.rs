use x86_64::{VirtAddr, structures::paging::PageTableFlags};

use crate::{library::Time, loader::SYSCALL_BUF_ADDR, memory::{KMemory, PAGE_SIZE}, scheduling::scheduler::with_active_task};

use super::syscall_internal::{read_str_from_buffer, read_from_buffer};

const USER_PAGE_START: VirtAddr = VirtAddr::new(0x1000);

const PRINT_STRING: u64 = 0;
const PRINT_COLOR: u64 = 1;
const CLEAR_SCREEN: u64 = 2;
const SET_CURSOR: u64 = 3;

pub fn sys_print(arg1: u64) -> u64 {
    match arg1 {
        PRINT_STRING => {
            let string = match read_str_from_buffer(0) {
                Some(s) => s,
                None => return 1
            };

            println!("{}", string);
            0
        },

        PRINT_COLOR => {
            let string = match read_str_from_buffer(0) {
                Some(s) => s,
                None => return 1
            };

            let fg_raw: u32 = match read_from_buffer(string.len() + 1) {
                Some(fg) => fg,
                None => return 2
            };

            let bg_raw: u32 = match read_from_buffer(string.len() + size_of::<u32>() + 1) {
                Some(bg) => bg,
                None => return 2
            };

            crate::stdout::_print_with_colors(fg_raw, bg_raw, format_args!("{}\n", string));

            0
        },

        CLEAR_SCREEN => {
            crate::stdout::clear();
            0
        },

        SET_CURSOR => {
            let offset_x: u64 = match read_from_buffer(0) {
                Some(x) => x,
                None => return 1
            };

            let offset_y: u64 = match read_from_buffer(size_of::<u64>()) {
                Some(y) => y,
                None => return 1
            };

            crate::stdout::set_offsets(offset_x as usize, offset_y as usize);

            0
        }

        _ => u64::MAX / 2
    }
}

const YIELD: u64 = 0;
const SLEEP: u64 = 1;
const TERMINATE: u64 = 2;

pub fn sys_task_manage(arg1: u64) -> u64 {
    match arg1 {
        YIELD => {
            crate::scheduling::yield_now();
            0
        },

        SLEEP => {
            let duration: Time = match read_from_buffer::<u64>(0) {
                Some(x) => Time::Nanoseconds(x),
                None => return 1
            };

            crate::scheduling::task_sleep(duration);
            0
        },

        TERMINATE => {
            crate::scheduling::remove_active_task();
            0
        },

        _ => u64::MAX / 2
    }
}

const ALLOCATE: u64 = 0;

pub fn sys_memory_manage(arg1: u64) -> u64 {
    match arg1 {

        ALLOCATE => {
            let size = match read_from_buffer::<u64>(0) {
                Some(s) => s as usize,
                None => return 0
            };

            let pages = size.div_ceil(PAGE_SIZE);
            let page = KMemory::alloc_pages(pages);

            let virt = with_active_task(|t| {
                let mapped: usize = t.pages.iter().map(|p| p.size()).sum();
                let virt = USER_PAGE_START + (PAGE_SIZE * mapped) as u64;
                if virt.as_u64() > SYSCALL_BUF_ADDR { return None; }
                KMemory::map_user_page(t.page_table, virt, &page, PageTableFlags::WRITABLE);
                t.pages.push(page);
                Some(virt)
            });
            
            if virt.is_none() { return 0 }

            virt.unwrap().as_u64()
        }

        _ => u64::MAX / 2
    }
}