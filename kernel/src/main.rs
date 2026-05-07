#![no_main]
#![no_std]

mod requests;
mod stdout;
mod boot;

use core::panic::PanicInfo;

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    boot::init();

    println!("Hello, World!");
    
    hcf()
}

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    println!("PANIC! : {:#?}", info);

    hcf()
}

pub fn hcf() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}