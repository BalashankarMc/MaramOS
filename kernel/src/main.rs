//! Maram OS kernel.
//!
//! An x86_64 monolithic kernel that boots via the Limine bootloader on
//! UEFI firmware. Provides SMP-enabled preemptive multitasking, demand-
//! paged user-space processes, a custom filesystem (LemonFS), GPT
//! partition parsing, AHCI/NVMe storage drivers, and a framebuffer text
//! console.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt, const_trait_impl, const_ops)]
#![deny(clippy::all)]
#![allow(dead_code)]

extern crate alloc;

#[macro_use]
mod stdout;

mod allocator;
mod acpi;
mod boot;
mod cpu;
mod descriptors;
mod drivers;
mod fpu;
mod fs;
mod library;
mod loader;
mod memory;
mod scheduling;
mod syscalls;
mod gpt;



#[cfg(feature = "integration-test")]
mod tests;

use crate::boot::init;
use crate::cpu::ipi;

use core::panic::PanicInfo;
use x86_64::instructions::hlt;

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    init();
    stdout::clear();

    log_success!("Kernel ready");

    if let Ok(binary) = loader::load_bin("/bin.elf") {
        log_success!("Found user binary");
        scheduling::add_task(binary);
    } else {
        warn!("No user binary found");
    }

    halt_loop()
}

fn halt_loop() -> ! {
    loop { hlt() }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("Panic!");
    println!("{}", info);
    ipi::halt_other_cpus();
    halt_loop();
}
