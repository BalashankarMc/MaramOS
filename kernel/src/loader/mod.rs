//! User-space binary loader.
//!
//! Loads ELF executables from LemonFS into a new user address space with
//! demand-paged segments, a user stack, and a syscall buffer. The loaded
//! binary is returned as a [`LoadedBinary`] ready for the scheduler.

use alloc::vec::Vec;
use x86_64::PhysAddr;

use crate::memory::{VirtualMemoryArea, PhysPage};

mod elf;

const USER_STACK_SIZE: usize = 0x100_000;
const USER_STACK_TOP: usize = 0x7FF_FFF_000_000;
pub const SYSCALL_BUF_ADDR: u64 = 0x7FF_FFC_000_000;
pub const SYSCALL_BUFFER_SIZE: usize = 0x200_000;

pub struct LoadedBinary {
    pub page_table: PhysAddr,
    pub entry: u64,
    pub user_sp: u64,
    pub vmas: Vec<VirtualMemoryArea>,
    pub syscall_buffer: PhysPage
}

#[derive(Debug)]
pub enum LoaderError {
    InvalidFile,
    IOError,
    NotExecutable,
}

pub use elf::load_elf as load_bin;
