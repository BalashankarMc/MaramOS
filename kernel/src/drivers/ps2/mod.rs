//! Implements the PS2 driver for `MaramOS`.

use core::hint::spin_loop;
use x86_64::instructions::port::Port;

use crate::{InterruptMutex, log_error, log_success};

mod keyboard;

const DATA_PORT: u16 = 0x60;
const CMD_PORT: u16 = 0x64;

static DPORT: InterruptMutex<Port<u8>> = InterruptMutex::new(Port::new(DATA_PORT));
static CPORT: InterruptMutex<Port<u8>> = InterruptMutex::new(Port::new(CMD_PORT));

pub fn init() -> Result<(), ()> {
    if read_status() == 0xFF {
        log_error!("Floating PS/2 Controller!");
        return Err(())
    }

    // Disable keyboard and mouse
    write_command(0xAD);
    write_command(0xA7);

    // Drain stale data
    while read_status() & 1 != 0 { read_data(); }

    // Controller self test
    write_command(0xAA);
    if read_data() != 0x55 { return Err(()) }

    // Mask both clocks
    update_config(|c| c | (3 << 4));

    write_command(0xAE);
    write_command(0xA8);

    log_success!("Initialized PS/2 Controller!");
    
    keyboard::init()?;
    log_success!("Initialized PS/2 Keyboard!");

    Ok(())
}

fn write_command(cmd: u8) {
    while read_status() & 2 != 0 { spin_loop() } // TODO: Add timeout here
    unsafe { CPORT.lock().write(cmd) }
}

fn write_data(data: u8) {
    while read_status() & 2 != 0 { spin_loop() }
    unsafe { DPORT.lock().write(data) }
}

fn read_status() -> u8 {
    unsafe { CPORT.lock().read() }
}

fn read_data() -> u8 {
    while read_status() & 1 == 0 { spin_loop() }
    unsafe { DPORT.lock().read() }
}

fn update_config(f: fn(u8) -> u8) {
    write_command(0x20); // Ask for current config
    let config = read_data();
    write_command(0x60); // Prepare to write a new config
    write_data(f(config));
}