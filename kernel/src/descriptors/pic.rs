//! PIC (Programmable Interrupt Controller) management module

use x86_64::instructions::port::Port;

/// Disables the legacy PIC
pub fn disable() {
    unsafe {
        Port::new(0x20).write(0x11_u8);
        Port::new(0xA0).write(0x11_u8);
    }
    
    let mut master = Port::new(0x21);
    let mut slave = Port::new(0xA1);

    unsafe {
        // ICW2: vector base for master/slave
        master.write(0x20_u8);
        slave.write(0x28_u8);

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