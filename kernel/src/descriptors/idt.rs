use x86_64::{registers::control::Cr2, structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode}};

use crate::{helpers::LateInit, log_warn};

static IDT: LateInit<InterruptDescriptorTable> = LateInit::new();

pub fn init() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(breakpoint);
    idt.page_fault.set_handler_fn(page_fault);
    idt.general_protection_fault.set_handler_fn(general_protection_fault);
    idt.invalid_opcode.set_handler_fn(invalid_opcode);
    idt.divide_error.set_handler_fn(division_error);

    unsafe {
        idt.double_fault.set_handler_fn(double_fault).set_stack_index(0);
        idt.non_maskable_interrupt.set_handler_fn(non_maskable_interrupt).set_stack_index(1);
        idt.machine_check.set_handler_fn(machine_check).set_stack_index(2);
    }

    IDT.init(idt).load();
}

extern "x86-interrupt" fn double_fault(stack_frame: InterruptStackFrame, err_code: u64) -> ! {
    panic!("DOUBLE FAULT!\nError Code: {err_code}\nStack Frame: {stack_frame:#?}")
}

extern "x86-interrupt" fn breakpoint(stack_frame: InterruptStackFrame) {
    log_warn!("Breakpoint!\nStack frame: {:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault(stack_frame: InterruptStackFrame, err_code: PageFaultErrorCode) {
    let addr = Cr2::read().unwrap().as_u64();
    panic!("Page Fault\nAccessed address: 0x{addr:2X}\nError Code: {err_code:?}\nStack frame:{stack_frame:#?}");
}

extern "x86-interrupt" fn general_protection_fault(stack_frame: InterruptStackFrame, err_code: u64) {
    panic!("General Protection Fault!\nError Code: {err_code}\nStack Frame: {stack_frame:?}")
}

extern "x86-interrupt" fn invalid_opcode(_stack_frame: InterruptStackFrame) {
    panic!("Unknown opcode!");
}

extern "x86-interrupt" fn division_error(_stack_frame: InterruptStackFrame) {
    panic!("Division Error!");
}

extern "x86-interrupt" fn non_maskable_interrupt(_stack_frame: InterruptStackFrame) {
    log_warn!("Non Maskable Interrupt Triggered!");
}

extern "x86-interrupt" fn machine_check(_stack_frame: InterruptStackFrame) -> ! {
    panic!("Machine Check!")
}