//! The descriptors module
//!
//! Initializes the Global Descriptor Table (GDT), the Interrupt Descriptor Table (IDT) and TSS for Double Fault Stacks
//!
//! # Dependencies: Memory Manager
//!
//! # Safety: Loads the descriptors directly to the CPU. but unlikely to fail

pub mod gdt;
pub mod interrupts;

/// Initializes the descriptors for IDT and GDT and loads them to the BSP
pub fn init() {
    gdt::init(unsafe { &mut (*crate::cpu::this_cpu()) });
    interrupts::init();
}
