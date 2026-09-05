#![no_std]
#![no_main]
#![allow(dead_code, clippy::cast_possible_truncation)]
#![deny(clippy::all)]
#![warn(unused_import_braces)]
#![feature(abi_x86_interrupt, const_index, const_trait_impl)]

extern crate alloc;

mod fs;
mod acpi;
mod errors;
mod memory;
mod prelude;
mod display;
mod drivers;
mod helpers;
mod requests;
mod scheduling;
mod allocators;
mod descriptors;

#[macro_use]
mod stdout;

pub use prelude::*;

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    x86_64::instructions::interrupts::disable();

    requests::init().expect("Failed to initialize requests!");

    stdout::init();
    memory::init().expect("Failed to setup memory!");
    descriptors::init().expect("Failed to setup descriptors");
    acpi::init().expect("Failed to initialize ACPI Subsystems");
    scheduling::init().unwrap();

    x86_64::instructions::interrupts::enable();
    
    if let Err(e) = drivers::ps2::init() { log_warn!("{e}") }
    if let Err(e) = drivers::pci::init() { log_warn!("{e}") }
    if let Err(e) = drivers::storage::init() { log_warn!("{e}") }

    let drive = drivers::storage::claim_drive(|_| true).unwrap();
    let mut fs = fs::init(&drive).unwrap().pop().unwrap();

    let files = fs.list("/".into()).unwrap();
    println!("{:#?}", files);

    log_success!("Kernel ready");

    loop {
        if let Some(key) = drivers::ps2::KEYBOARD_BUFFER.lock().pop_front() {
            print!("{:?}", key.code);
        } else { x86_64::instructions::hlt() }
    }
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
