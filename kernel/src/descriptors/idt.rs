#![allow(clippy::manual_assert)]

use x86_64::{PrivilegeLevel, VirtAddr};
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

use crate::{KResult, helpers::LateInit, log_warn};

pub static IDT: LateInit<InterruptDescriptorTable> = LateInit::new();

pub fn init() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint);
    idt.page_fault.set_handler_fn(page_fault);
    idt.general_protection_fault.set_handler_fn(general_protection_fault);
    idt.invalid_opcode.set_handler_fn(invalid_opcode);
    idt.divide_error.set_handler_fn(division_error);

    // These handlers need IST Stacks
    // Safety: We know that each stack inedx is valid and not use by other interrupts. Safe
    unsafe {
        idt.double_fault.set_handler_fn(double_fault).set_stack_index(0);
        idt.non_maskable_interrupt.set_handler_fn(non_maskable_interrupt).set_stack_index(1);
        idt.machine_check.set_handler_fn(machine_check).set_stack_index(2);
    }

    // Safety: IDT always stays at the same addr and is never dropped. Safe.
    unsafe { IDT.init(idt).load_unsafe() };    
}

extern "x86-interrupt" fn double_fault(stack_frame: InterruptStackFrame, err_code: u64) -> ! {
    panic!("DOUBLE FAULT!\nError Code: {err_code}\nStack Frame: {stack_frame:#?}")
}

extern "x86-interrupt" fn breakpoint(stack_frame: InterruptStackFrame) {
    log_warn!("Breakpoint!\nStack frame: {:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault(stack_frame: InterruptStackFrame, err_code: PageFaultErrorCode) {
    let addr = Cr2::read().unwrap().as_u64();

    if stack_frame.code_segment.rpl() == PrivilegeLevel::Ring0 {
        panic!("Page Fault\nAccessed address: 0x{addr:2X}\nError Code: {err_code:?}\nStack frame:{stack_frame:#?}");
    }

    // Fault from user task
    crate::scheduling::kill_active_task(u64::MAX);
}

extern "x86-interrupt" fn general_protection_fault(stack_frame: InterruptStackFrame, err_code: u64) {
    if stack_frame.code_segment.rpl() == PrivilegeLevel::Ring0 {
        panic!("General Protection Fault!\nError Code: {err_code}\nStack Frame: {stack_frame:#?}")
    }

    // User Fault
    crate::scheduling::kill_active_task(u64::MAX - 1);
}

extern "x86-interrupt" fn invalid_opcode(stack_frame: InterruptStackFrame) {
    if stack_frame.code_segment.rpl() == PrivilegeLevel::Ring0 {
        panic!("Unknown opcode!");
    }

    crate::scheduling::kill_active_task(u64::MAX - 2);
}

extern "x86-interrupt" fn division_error(stack_frame: InterruptStackFrame) {
    if stack_frame.code_segment.rpl() == PrivilegeLevel::Ring0 {
        panic!("Division Error!");
    }

    crate::scheduling::kill_active_task(u64::MAX - 3);
}

extern "x86-interrupt" fn non_maskable_interrupt(_stack_frame: InterruptStackFrame) {
    log_warn!("Non Maskable Interrupt Triggered!");
}

extern "x86-interrupt" fn machine_check(_stack_frame: InterruptStackFrame) -> ! {
    panic!("Machine Check!")
}

// No HardwareInterrupts vector should cross 0xFC. 0xFC is the MAX
#[derive(Clone, Copy)]
pub enum HardwareInterrupts {
    Timer = 32,
    Keyboard,
    Ahci
}

impl HardwareInterrupts {
    pub const fn as_u8(self) -> u8 { self as u8 }
}

pub fn add_idt_entry(f: extern "x86-interrupt" fn(InterruptStackFrame), index: u8) -> KResult<()> {
    let ptr = &raw const IDT as usize as *mut LateInit<InterruptDescriptorTable>;
    // Safety: Dereference of a safe pointer. Safe
    let reference = unsafe { &mut *ptr };
    if reference[index].handler_addr() != VirtAddr::zero() { return Err(crate::KernelError::IDTRegisterError(index)) }

    reference.get_mut()[index].set_handler_fn(f);
    Ok(())
}