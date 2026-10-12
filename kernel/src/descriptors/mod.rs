mod gdt;
mod idt;
mod pic;

pub use idt::{IDT, HardwareInterrupts, add_idt_entry};
pub use gdt::{Selectors, new_gdt};

pub fn init() {
    idt::init();
    pic::disable();
}
