//! FPU/SSE state management and lazy context switching.
//!
//! Initialises x87 and SSE via CR0/CR4, provides FXSAVE/FXRSTOR wrappers,
//! and implements the `#NM` (Device Not Available) interrupt handler for
//! lazy FPU state save/restore. CR0.TS is set after each context switch
//! so the FPU is only saved when actually used.

use core::arch::asm;

use x86_64::structures::idt::InterruptStackFrame;

/// FPU/SSE state buffer (FXSAVE/FXRSTOR format).
///
/// 512 bytes, 16-byte aligned as required by `fxsave`/`fxrstor`.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub struct FpuState {
    raw: [u8; 512],
}

impl FpuState {
    /// Zeroed state (not usable until initialized).
    pub const fn new() -> Self {
        FpuState { raw: [0u8; 512] }
    }

    /// Overwrite self with the current CPU's FPU/SSE state.
    ///
    /// # Safety
    /// Self is 16-byte aligned (guaranteed by `#[repr(align(16))]`).
    #[inline]
    pub unsafe fn save(&mut self) {
        unsafe { fxsave(self) }
    }

    /// Initialise this buffer to a clean x87+SSE default state.
    pub fn init_state(&mut self) {
        unsafe {
            asm!("fninit");
            let mxcsr: u32 = 0x1F80;
            asm!("ldmxcsr [{}]", in(reg) &mxcsr, options(nostack, preserves_flags));
            self.save();
        }
    }
}

/// Enable FPU + SSE on the current CPU and set a known initial state.
pub fn init() {
    // CR0: clear EM (emulation), set MP (monitor coprocessor), set NE (native error).
    unsafe {
        asm!(
            "mov {tmp}, cr0",
            "and {tmp}, {clear}",
            "or  {tmp}, {set}",
            "mov cr0, {tmp}",
            clear = const !(1u64 << 2),      // ~EM
            set   = const (1u64 << 1) | (1u64 << 5), // MP | NE
            tmp   = out(reg) _,
            options(nostack, preserves_flags),
        );
    }

    // CR4: set OSFXSR (SSE + FXSAVE/FXRSTOR), OSXMMEXCPT (#XM handler).
    unsafe {
        asm!(
            "mov {tmp}, cr4",
            "or  {tmp}, {set}",
            "mov cr4, {tmp}",
            set = const (3_u64 << 9) | (1_u64 << 17),
            tmp = out(reg) _,
            options(nostack, preserves_flags),
        );
    }

    // Initialise x87 + MXCSR to known state.
    unsafe {
        asm!("fninit");
        let mxcsr: u32 = 0x1F80;
        asm!("ldmxcsr [{}]", in(reg) &mxcsr, options(nostack, preserves_flags));
    }
}

/// Execute `fxsave` to the given 16-byte-aligned buffer.
///
/// # Safety
/// `state` must be 16-byte aligned and point to at least 512 bytes of
/// writable memory.
#[inline]
pub(crate) unsafe fn fxsave(state: &mut FpuState) {
    unsafe {
        asm!("fxsave [{}]", in(reg) state, options(nostack, preserves_flags));
    }
}

#[inline]
pub(crate) unsafe fn fxrestore(state: &FpuState) {
    unsafe {
        asm!("fxrstor [{}]", in(reg) state, options(nostack, preserves_flags));
    }
}


pub fn set_cr0_ts() {
    unsafe { asm!(
        "mov {tmp}, cr0",
        "or {tmp} 8",
        "mov cr0, {tmp}",
        tmp = out(reg) _,
        options(nostack, preserves_flags)
    )}
}

pub fn clear_cr0_ts() {
    unsafe { asm!(
        "mov {tmp}, cr0",
        "and {tmp}, -9",
        "mov cr0, {tmp}",
        tmp = out(reg) _,
        options(nostack, preserves_flags)
    )}
}

pub fn is_cr0_ts_set() -> bool {
    let r: u64;
    unsafe { asm!(
        "mov {tmp}, cr0",
        tmp = out(reg) r,
        options(nostack, preserves_flags)
    )}
    r & 8 != 0
}

pub extern "x86-interrupt" fn nm_handler(_stack_frame: InterruptStackFrame) {
    clear_cr0_ts();

    let task_id = crate::scheduling::scheduler::with_active_task(|task| {
        unsafe { fxrestore(&task.fpu_state) }
        task.id
    });

    unsafe {
        let cpu = &mut *crate::cpu::this_cpu();
        cpu.fpu_owner_id = Some(task_id as u64);
    }
}