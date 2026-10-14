mod syscall_internal;

use core::{arch::naked_asm, mem::offset_of};

use x86_64::{VirtAddr, registers::{control::{Efer, EferFlags}, model_specific::{LStar, SFMask, Star}, rflags::RFlags}};

use crate::smp::{CPUData, current_cpu};

pub fn init() {
    let selectors = current_cpu().selectors;
    unsafe { Efer::update(|flags| flags.set(EferFlags::SYSTEM_CALL_EXTENSIONS, true)) }; // Enable Syscall and Sysret

    if Star::write(
        selectors.user_code_intel,
        selectors.user_data,
        selectors.kernel_code,
        selectors.kernel_data,
    ).is_err() { panic!("Failed to init Syscalls: STAR Failed") }

    LStar::write(VirtAddr::new(entry as *const () as u64)); // Write the dispatch address
    SFMask::write(RFlags::INTERRUPT_FLAG | RFlags::DIRECTION_FLAG); // Mask IF and DF on syscall
}

#[unsafe(naked)]
extern "C" fn entry() {
    naked_asm!(
        "xchg gs:[{offset}], rsp",
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

        "xchg gs:[{offset}], rsp",
        "sysretq",

        dispatch = sym syscall_internal::dispatch,
        offset = const offset_of!(CPUData, syscall_stack)
    )
}
