use crate::KernelResult;
use crate::log_success;

mod gdt;
mod idt;
mod pic;

pub use gdt::SELECTORS;
pub use idt::HardwareInterrupts;
pub use idt::add_idt_entry;

/// Initializes a GDT, IDT and the PIC.
/// Also disables the PIC
pub fn init() -> KernelResult<()> {
    gdt::init()?;
    idt::init();
    pic::disable();

    log_success!("Descriptors Initialized!");
    Ok(())
}