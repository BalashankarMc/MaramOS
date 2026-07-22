//! Kernel boot sequence and Limine protocol requests.
//!
//! Orchestrates the full initialisation order: memory, framebuffer, FPU,
//! ACPI, descriptors, LAPIC, IPI, SMP, PCI, interrupts, syscalls, storage,
//! GPT parsing, and LemonFS mount. Also owns the [`FrameBuffer`] wrapper
//! used by the console.

pub mod requests;

use crate::{
    boot::requests::*, drivers::storage, fs::{FileSystem, LemonFS}, gpt::PartitionType, library::LateInit
};

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use limine::framebuffer::Framebuffer;
use spin::Mutex;

#[derive(Clone, Copy, Debug)]
pub struct FrameBuffer {
    ptr: u64,
    pub stride: usize,
    pub width: usize,
    pub height: usize,
}

impl FrameBuffer {
    pub fn new(fb: &Framebuffer) -> Self {
        Self {
            ptr: fb.address() as u64,
            stride: fb.pitch as usize,
            width: fb.width as usize,
            height: fb.height as usize,
        }
    }

    pub fn fill_screen(&mut self, color: u32) {
        let stride_px = self.stride / 4;
        for y in 0..self.height {
            for x in 0..stride_px {
                unsafe {
                    (self.ptr as *mut u32)
                        .add(y * stride_px + x)
                        .write_volatile(color);
                }
            }
        }
    }

    pub fn draw_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = (y * self.stride / 4) + x;
        unsafe {
            (self.ptr as *mut u32).add(offset).write_volatile(color);
        }
    }

    pub fn read_pixel(&self, x: usize, y: usize) -> u32 {
        let offset = (y * self.stride / 4) + x;
        unsafe { (self.ptr as *mut u32).add(offset).read_volatile() }
    }

    pub fn clear_row(&mut self, y: usize) {
        for x in 0..self.width {
            self.draw_pixel(x, y, 0x0);
        }
    }

    pub fn clear_rows(&mut self, start_y: usize, rows: usize) {
        for y in start_y..(start_y + rows) {
            self.clear_row(y);
        }
    }

    pub fn scroll_up(&mut self, rows: usize) {
        let stride_px = self.stride / 4;
        let total_rows = self.height;
        for y in 0..total_rows.saturating_sub(rows) {
            for x in 0..stride_px {
                let pixel = self.read_pixel(x, y + rows);
                self.draw_pixel(x, y, pixel);
            }
        }
    }
}

pub static FS: LateInit<crate::fs::LemonFS> = LateInit::new();

pub fn init() {
    assert!(BASE_REVISION.is_supported());

    crate::memory::init();

    let mut framebuffer = FrameBuffer::new(FB_DATA.framebuffers()[0]);
    framebuffer.fill_screen(0x0);
    crate::stdout::init_writer(framebuffer, 0x0, 0x00FFFF);
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
