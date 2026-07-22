//! Syscall entry and dispatch.
//!
//! # Syscall ABI (x86-64)
//!
//! User programs invoke syscalls via the `syscall` instruction.
//!
//! | Register | Role |
//! |----------|------|
//! | `rax`    | Syscall number |
//! | `rbx`    | Argument 1 |
//! | `rdx`    | Argument 2 |
//! All other arguments must be passed in through the syscall buffer.
//!
//! Return value is placed in `rax`.  The kernel preserves all registers
//! except `rax` and `r11` (RFLAGS).
//!
//! Init sets `LSTAR` (entry point), `STAR` (segment selectors), and
//! `SF_MASK` (mask IF/DF on entry).  The kernel stack is prepared via
//! the per-CPU `kernel_rsp`.

use core::arch::naked_asm;

use x86_64::{
    VirtAddr,
    registers::{
        model_specific::{Efer, EferFlags, LStar, SFMask, Star},
        rflags::RFlags,
    },
};

mod syscall_internal;
mod functions;

pub fn init() {
    let selectors = unsafe { crate::cpu::this_cpu().read().selectors };
    unsafe { Efer::update(|flags| flags.set(EferFlags::SYSTEM_CALL_EXTENSIONS, true)) }; // Enable Syscall and Sysret

    if Star::write(
        selectors.user_code_intel,
        selectors.user_data,
        selectors.kernel_code,
        selectors.kernel_data,
    ).is_err() { panic!("Failed to init Syscalls: STAR Failed") };

    LStar::write(VirtAddr::new(entry as *const () as u64)); // Write the dispatch address
    SFMask::write(RFlags::INTERRUPT_FLAG | RFlags::DIRECTION_FLAG); // Mask IF and DF on syscall
}

#[unsafe(naked)]
extern "C" fn entry() {
    naked_asm!(
        "xchg gs:[0x08], rsp",
        "push rcx",
        "push r11",
        "push rbx", "push rdx",
        "push rsi", "push rdi", "push rbp",
        "push r8", "push r9", "push r10",
        "push r12", "push r13", "push r14", "push r15",

        "mov rdi, rax",
        "mov rsi, rbx",
        // Arg 3 is rdx, which maps directly to the SYSV64 ABI

        "call {dispatch}",

        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r10", "pop r9", "pop r8",
        "pop rbp", "pop rdi", "pop rsi",
        "pop rdx", "pop rbx", "pop r11",
        "pop rcx",

        "xchg gs:[0x08], rsp",
        "sysretq",

        dispatch = sym syscall_internal::dispatch
    )
}
