#![no_std]
#![no_main]
#![allow(dead_code, clippy::cast_possible_truncation)]
#![deny(clippy::all)]
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

#[macro_use]
mod stdout;

pub use prelude::*;

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {

    requests::init().expect("Failed to initialize requests!");

    // Framebuffer initialisation

    let fb_raw = FB_RESPONSE.framebuffers()[0];
    stdout::init(fb_raw);
    stdout::clear();
    log_success!("Framebuffer initialized!");

    memory::init().expect("Failed to setup memory!");
    descriptors::init().expect("Failed to setup descriptors");

    log_success!("Kernel ready");

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
