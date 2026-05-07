use limine::framebuffer::Framebuffer;

use crate::println; 
use crate::requests::*;

#[derive(Clone, Copy)]
pub struct FrameBuffer {
    ptr: u64,
    pub stride: usize,
    pub width: usize,
    pub height: usize
}

impl FrameBuffer {

    pub fn new(fb: &Framebuffer) -> Self {
        Self {
            ptr: fb.address() as u64,
            stride: fb.pitch as usize,
            width: fb.width as usize,
            height: fb.height as usize
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
        if x >= self.width || y >= self.height { return; }
        let offset = (y * self.stride/4) + x;
        unsafe {
            (self.ptr as *mut u32).add(offset).write_volatile(color);
        }
    }

    pub fn read_pixel(&self, x: usize, y: usize) -> u32 {
        let offset = (y * self.stride/4) + x;
        unsafe {
            (self.ptr as *mut u32).add(offset).read_volatile()
        }
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

pub fn init() {
    assert!(BASE_REVISION.is_supported());

    let mut framebuffer = FrameBuffer::new(FRAMEBUFFER_REQUEST.response().unwrap().framebuffers()[0]);
    framebuffer.fill_screen(0x0);

    crate::stdout::init_writer(framebuffer, 0x0, 0xFFFFFF);
}
