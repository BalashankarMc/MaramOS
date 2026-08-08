use crate::log_success;

mod gdt;
mod idt;
mod pic;

pub use idt::HardwareInterrupts;
pub use idt::add_idt_entry;

/// Initializes a GDT, IDT and the PIC.
/// Also disables the PIC
pub fn init() -> Result<(), ()> {
    gdt::init()?;
    idt::init();
    pic::disable();

    log_success!("Descriptors Initialized!");
    Ok(())
}