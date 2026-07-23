//! Kernel boot sequence and Limine protocol requests.
//!
//! Orchestrates the full initialisation order: memory, framebuffer, FPU,
//! ACPI, descriptors, LAPIC, IPI, SMP, PCI, interrupts, syscalls, storage,
//! GPT parsing, and LemonFS mount.

pub mod requests;

use crate::{
    boot::requests::*, drivers::storage, fs::{FileSystem, LemonFS}, gpt::PartitionType, library::{FrameBuffer, LateInit}
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use spin::Mutex;

pub static FS: LateInit<crate::fs::LemonFS> = LateInit::new();

pub fn init() {
    assert!(BASE_REVISION.is_supported());

    crate::memory::init();

    let mut framebuffer = FrameBuffer::new(FB_DATA.framebuffers()[0]);
    framebuffer.fill_screen(0x0);
    crate::stdout::init_writer(framebuffer, 0x00FFFF, 0x0);
    log_success!("Initialized Framebuffer");

    crate::fpu::init();
    crate::acpi::init();
    crate::cpu::init_bsp_data();
    crate::descriptors::init();
    crate::acpi::lapic_init_timer(crate::descriptors::interrupts::HardwareInterrupts::Timer.as_u8());
    crate::cpu::ipi::init();
    crate::cpu::smp::init_aps();
    crate::drivers::pci::init();
    log_success!("Initialized PCI Bus");

    x86_64::instructions::interrupts::enable();

    crate::syscalls::init();

    let mut raw_drives = storage::init_storage();
    let mut drives = Vec::new();

    if raw_drives.is_empty() { panic!("Failed to find Storage Drives") }

    while let Some(drive) = raw_drives.pop() {
        let drive_arc = Arc::new(Mutex::new(drive));
        match crate::gpt::parse_gpt(drive_arc.clone()) {
            Ok(mut v) => drives.append(&mut v),
            Err(_) => log_error!("Failed to read GPT!")
        }
    }

    let partition = drives.into_iter().find(|drive|drive.type_ == PartitionType::LemonFS);
    if partition.is_none() { panic!("No LEMONFS Parition Found!") }

    let drive = Box::new(partition.unwrap());
    let fs = match LemonFS::init(drive) {
        Ok(fs) => fs,
        Err(_) => panic!("Failed to initialize FS!")
    };

    FS.init(fs);
}
