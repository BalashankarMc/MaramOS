#![no_std]
#![no_main]
#![allow(dead_code)]
#![deny(clippy::all)]

mod boot;
mod display;
mod helpers;
mod requests;
mod stdout;

#[unsafe(no_mangle)]
fn kmain() -> ! {
    boot::init();
    log_success!("Kernel ready");
    halt_loop()
}

fn halt_loop() -> ! {
    loop { x86_64::instructions::hlt() }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    halt_loop()
}
