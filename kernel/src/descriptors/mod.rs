use crate::log_success;

mod gdt;
mod idt;

pub fn init() -> Result<(), ()> {
    gdt::init()?;
    idt::init();

    log_success!("Descriptors Initialized!");
    Ok(())
}