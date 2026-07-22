//! Interrupt Descriptor Table (IDT) and exception handlers.
//!
//! Registers handlers for breakpoint, GPF, double fault, page fault
//! (with demand paging for ring 3), NM (FPU lazy restore), timer (naked
//! jump to scheduler), and NVMe I/O completion.

use crate::descriptors::gdt::DF_IST_INDEX;
use crate::drivers::storage::nvme_io_completion as COMPLETION_FLAG;
use crate::library::LateInit;
use crate::scheduling::scheduler;
use crate::scheduling;

use core::arch::naked_asm;
use core::sync::atomic::Ordering;
use x86_64::{PrivilegeLevel, VirtAddr};
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{EntryOptions, InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

pub static IDT: LateInit<InterruptDescriptorTable> = LateInit::new();

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    log_error!("EXCEPTION: Breakpoint\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT (code: {})\n{:#?}",
        error_code, stack_frame
    );
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame)
}

extern "x86-interrupt" fn page_fault_handler(stack_frame: InterruptStackFrame, error_code: PageFaultErrorCode) {
    let addr = Cr2::read_raw();
    if stack_frame.code_segment.rpl() == PrivilegeLevel::Ring3 {
        match scheduler::with_active_task(|task| {
            crate::memory::resolve_user_demand_page(&task.vmas, task.page_table, addr, error_code)
        }) {
            true => return,

            false => {
                let causer = scheduler::with_active_task(|task| task.id);
                log_error!("Task ID: {}, Segmentation Fault @ {:x}: {:#?}!", causer, addr, error_code);
                scheduling::remove_active_task();
                return;
            }
        }
    }

    panic!(
        "PAGE FAULT!
        Accessed Address: {:#?},
        Error Code: {:#?}
        Stack Frame: {:#?}",
        Cr2::read(),
        error_code,
        stack_frame
    )
}

/// # Safety: IDT must be initialized by the BSP before APs try to load it
pub fn load_idt_ap() {
    IDT.load();
}

pub fn init() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint_handler);
    idt.general_protection_fault.set_handler_fn(gpf_handler);
    idt.page_fault.set_handler_fn(page_fault_handler);
    idt.device_not_available.set_handler_fn(crate::fpu::nm_handler);

    idt[HardwareInterrupts::Timer.as_u8()].set_handler_fn(timer_handler);
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(DF_IST_INDEX as u16);
    }

    IDT.init(idt);
    IDT.load();
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum HardwareInterrupts {
    Timer = 36,
    NVMeIO,
    AhciIO
}

impl HardwareInterrupts {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

#[unsafe(naked)]
extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    naked_asm!(
        "jmp {sched}",
        sched = sym crate::scheduling::schedule
    )
}

pub fn add_idt_entry<'a>(f: extern "x86-interrupt" fn(InterruptStackFrame), index: u8) -> Option<&'a mut EntryOptions> {
    let idt = IDT.get_mut();
    if idt[index].handler_addr() != VirtAddr::zero() { return None; }

    Some(idt[index].set_handler_fn(f))
}