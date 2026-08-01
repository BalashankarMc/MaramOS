use limine::framebuffer::Framebuffer;

pub struct FrameBuffer<'a>(&'a Framebuffer);

impl<'a> FrameBuffer<'a> {
    pub const fn new(fb: &'a Framebuffer) -> Self { Self(fb) }

    fn ptr(&self) -> *mut u32 {
        self.0.address().cast::<u32>()
    }

    const fn position(&self, x: usize, y: usize) -> usize {
        ((self.0.pitch / 4) as usize * y) + x
    }

    pub const fn width(&self) -> usize {
        self.0.width as usize
    }

    pub const fn height(&self) -> usize {
        self.0.height as usize
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: u32) {
        unsafe { self.ptr().add(self.position(x, y)).write_volatile(color) }
    }

    pub fn read_pixel(&self, x: usize, y: usize) -> u32 {
        unsafe { self.ptr().add(self.position(x, y)).read_volatile() }
    }

    pub fn set_row(&mut self, y: usize, row: &[u32]) {
        let width = self.0.width as usize;
        for (x, &pixel) in row.iter().enumerate().take(width) {
            self.set_pixel(x, y, pixel);
        }
    }

    pub fn fill_screen(&self, color: u32) {
        let ptr = self.ptr();
        for i in 0..self.0.size() / 4 {
            unsafe { ptr.add(i).write_volatile(color) }
        }
    }
}