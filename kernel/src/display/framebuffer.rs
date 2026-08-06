use limine::framebuffer::Framebuffer;

pub struct FrameBuffer {
    address: usize,
    height: usize,
    width: usize,
    pitch: usize,
    size: usize
}

impl FrameBuffer {
    pub fn new(fb: &Framebuffer) -> Self {
        Self {
            address: fb.address() as usize,
            height: fb.height as usize,
            width: fb.width as usize,
            pitch: fb.pitch as usize,
            size: fb.size()
        }
    }

    const fn ptr(&self) -> *mut u32 {
        self.address as *mut u32
    }

    const fn position(&self, x: usize, y: usize) -> usize {
        ((self.pitch / 4) * y) + x
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: u32) {
        unsafe { self.ptr().add(self.position(x, y)).write_volatile(color) }
    }

    pub fn read_pixel(&self, x: usize, y: usize) -> u32 {
        unsafe { self.ptr().add(self.position(x, y)).read_volatile() }
    }

    pub fn set_row(&mut self, y: usize, row: &[u32]) {
        for (x, &pixel) in row.iter().enumerate().take(self.width) {
            self.set_pixel(x, y, pixel);
        }
    }

    pub fn fill_screen(&self, color: u32) {
        let ptr = self.ptr();
        for i in 0..self.size / 4 {
            unsafe { ptr.add(i).write_volatile(color) }
        }
    }

    pub const fn width(&self) -> usize { self.width }
    pub const fn height(&self) -> usize { self.height }
}