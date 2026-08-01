use crate::log_success;

mod gdt;
mod idt;

pub fn init() {
    gdt::init();
    idt::init();

    log_success!("Descriptors Initialized!");
}