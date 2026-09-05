//! PIC (Programmable Interrupt Controller) management module

use x86_64::instructions::port::Port;

/// Disables the legacy PIC
pub fn disable() {
    unsafe {
        Port::<u8>::new(0x20).write(0x11);
        Port::<u8>::new(0xA0).write(0x11);
    }
    
    let mut master = Port::<u8>::new(0x21);
    let mut slave = Port::<u8>::new(0xA1);

    unsafe {
        // ICW2: vector base for master/slave
        master.write(0x20);
        slave.write(0x28);

        // ICW3: cascade wiring
        master.write(4);
        slave.write(2);

        // ICW4: 8086 mode
        master.write(1);
        slave.write(1);

        // OCW1: mask ALL IRQs on both PICs (this is the "disable")
        master.write(0xFF);
        slave.write(0xFF);
    }
}