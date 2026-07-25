#![no_std]
#![no_main]

#[unsafe(no_mangle)]
fn kmain() -> ! {
    halt_loop()
}

fn halt_loop() -> ! {
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    halt_loop()
}