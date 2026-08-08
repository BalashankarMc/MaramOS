//! Implements a driver for PS/2 Keyboards

use alloc::collections::vec_deque::VecDeque;
use pc_keyboard::{KeyEvent, ScancodeSet1, layouts};
use x86_64::structures::idt::InterruptStackFrame;

use pc_keyboard::PS2Keyboard as Keyboard;

use crate::{InterruptMutex, acpi::lapic_eoi, descriptors::{HardwareInterrupts, add_idt_entry}};
use super::{update_config, write_data, read_data};

static DECODER: InterruptMutex<Keyboard<layouts::Us104Key, ScancodeSet1>> = InterruptMutex::new(
    Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, pc_keyboard::HandleControl::Ignore)
);

pub static KEY_BUFFER: InterruptMutex<VecDeque<KeyEvent>> = InterruptMutex::new(VecDeque::new());

pub fn init() -> Result<(), ()> {
    // Enable keyboard clock and IRQ
    update_config(|c| c & !(1 << 4) | ((1 << 6) | 1));

    // Reset keyboard
    write_data(0xFF);
    if read_data() != 0xFA { return Err(()) }
    if read_data() != 0xAA { return Err(()) }

    // Enable scanning
    write_data(0xF4);
    if read_data() != 0xFA { return Err(()) }

    // Add the Interrupt Handler
    add_idt_entry(keyboard_handler, HardwareInterrupts::Keyboard.as_u8());

    // Route interrupts
    crate::acpi::redirect_ioapic(1, HardwareInterrupts::Keyboard.as_u8(), 0, 0).map_err(|_| ())?;
    crate::acpi::unmask_ioapic(1).map_err(|_| ())?;

    Ok(())
}

extern "x86-interrupt" fn keyboard_handler(_stack_frame: InterruptStackFrame) {
    let scancode = super::read_data();
    let mut decoder = DECODER.lock();
    if let Ok(Some(key)) = decoder.add_byte(scancode) { KEY_BUFFER.lock().push_back(key) }
    lapic_eoi();
}