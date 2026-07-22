//! Syscall dispatch and buffer helpers.
//!
//! Implements the [`PRINT`] syscall (number 0) which reads a string from
//! the user's syscall buffer and prints it. Also provides typed and
//! string buffer read helpers for future syscall implementations.

use super::functions::*;
use alloc::string::{String, ToString};

const MAX_STRLEN: usize = 4096;

const PRINT: u64 = 0;
const TASK_MGMT: u64 = 1;
const MEMORY_MGMT: u64 = 2;

pub extern "C" fn dispatch(syscall_number: u64, sub_arg: u64) -> u64 {
    match syscall_number {
        PRINT => sys_print(sub_arg),
        TASK_MGMT => sys_task_manage(sub_arg),
        MEMORY_MGMT => sys_memory_manage(sub_arg),

        _ => u64::MAX
    }
}

pub fn read_from_buffer<T: Sized + Copy>(offset: usize) -> Option<T> {

    if core::mem::size_of::<T>() + offset > crate::loader::SYSCALL_BUFFER_SIZE { return None }

    Some(crate::scheduling::scheduler::with_active_task(
        |t| t.syscall_buffer.read_data(offset)
    ))
}

pub fn read_str_from_buffer(offset: usize) -> Option<String> {
    crate::scheduling::scheduler::with_active_task(|t|{

        if offset + MAX_STRLEN > crate::loader::SYSCALL_BUFFER_SIZE { return None }

        let mut virt = t.syscall_buffer.get_virt_addr();
        virt += offset as u64;

        let mut buffer = [0; MAX_STRLEN];
        let ptr = virt.as_ptr::<u8>();
        unsafe { core::ptr::copy_nonoverlapping(ptr, buffer.as_mut_ptr(), MAX_STRLEN) }

        if !buffer.contains(&0) {
            let s = core::str::from_utf8(&buffer).unwrap_or("");
            return Some(s.to_string())
        }

        let len = buffer.iter().position(|&x| x == 0).unwrap(); // Definetly has a zero so unwrap is ok
        let s = core::str::from_utf8(&buffer[..len]).unwrap_or("");
        Some(s.to_string())
    })
}