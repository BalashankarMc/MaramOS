#![no_std]
#![no_main]
#![allow(dead_code, clippy::cast_possible_truncation)]
#![deny(clippy::all)]

extern crate alloc;

mod display;
mod helpers;
mod requests;
mod memory;
mod allocator;

#[macro_use]
mod stdout;

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    // Framebuffer initialisation

    let fb_raw = requests::FRAMEBUFFER_REQUEST.response().unwrap().framebuffers()[0];
    stdout::init(fb_raw);
    stdout::clear();
    log_success!("Framebuffer initialized!");

    memory::init();

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
