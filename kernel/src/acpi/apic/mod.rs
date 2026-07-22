//! APIC (Advanced Programmable Interrupt Controller) subsystem.
//!
//! Initialises the I/O APIC for external interrupt redirection and the
//! Local APIC for timer, IPI, and EOI operations. The internal sub-module
//! extracts I/O APIC base addresses from MADT entries.

use x86_64::VirtAddr;

pub mod apic_internal;
pub mod ioapic;
pub mod lapic;

pub fn init(apic_page: VirtAddr) {
    unsafe {
        apic_internal::init(apic_page);
    }
    ioapic::init();
    lapic::init();
}
