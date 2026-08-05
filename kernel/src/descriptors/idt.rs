use x86_64::{registers::control::Cr2, structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode}};

use crate::{helpers::LateInit, log_error};

static IDT: LateInit<InterruptDescriptorTable> = LateInit::new();

pub fn init() {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint.set_handler_fn(bp_handler);
    idt.page_fault.set_handler_fn(pf_handler);
    IDT.init(idt).load();
}

extern "x86-interrupt" fn bp_handler(stack_frame: InterruptStackFrame) {
    log_error!("Breakpoint!\nStack frame: {:#?}", stack_frame);
}

extern "x86-interrupt" fn pf_handler(stack_frame: InterruptStackFrame, err_code: PageFaultErrorCode) {
    let addr = Cr2::read().unwrap().as_u64();

    panic!("Page Fault\nAccessed address: 0x{addr:2X}\nError Code: {err_code:?}\nStack frame:{stack_frame:#?}");
}