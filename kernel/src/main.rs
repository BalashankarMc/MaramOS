#![no_std]
#![no_main]
#![allow(dead_code, clippy::cast_possible_truncation)]
#![deny(clippy::all, unused_import_braces)]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod display;
mod helpers;
mod requests;
mod memory;
mod allocators;
mod descriptors;
mod errors;
mod prelude;
mod acpi;
mod drivers;

#[macro_use]
mod stdout;

pub use prelude::*;

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {

    requests::init().expect("Failed to initialize requests!");

    stdout::init();
    memory::init().expect("Failed to setup memory!");
    descriptors::init().expect("Failed to setup descriptors");
    acpi::init().unwrap();

    acpi::init_lapic_timer(descriptors::HardwareInterrupts::Timer.as_u8());

    x86_64::instructions::interrupts::enable();
    
    if let Err(e) = drivers::ps2::init() { log_warn!("{e}") }
    if let Err(e) = drivers::pci::init() { log_warn!("{e}") }

    log_success!("Kernel ready");

    loop {
        if let Some(key) = drivers::ps2::KEYBOARD_BUFFER.lock().pop_front() {
            print!("{:?}", key.code);
        }
    }

    halt_loop()
}

fn halt_loop() -> ! {
    loop { x86_64::instructions::hlt() }
}


#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log_error!("KERNEL PANIC!");

    println!("{}", info);

    halt_loop()
}
