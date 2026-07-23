use limine::framebuffer::Framebuffer;

#[derive(Debug)]
pub struct FrameBuffer {
    ptr: usize,
    pub height: usize,
    pub width: usize,
    pub stride: usize
}

impl FrameBuffer {
    pub fn new(fb: &Framebuffer) -> Self {
        Self {
            ptr: fb.address() as usize,
            height: fb.height as usize,
            width: fb.width as usize,
            stride: fb.pitch as usize / 4
        }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: u32) {
        let idx = match self.pos_to_idx(x, y) {
            Some(x) => x,
            None => return
        };

        let slice = unsafe { self.as_slice() };
        slice[idx] = color
    }

    pub fn fill_screen(&mut self, color: u32) {
        let slice = unsafe { self.as_slice() };
        slice.fill(color)
    }

    pub fn scroll(&mut self, rows: usize) {
        if rows == 0 || rows >= self.height { return }

        let stride = self.stride;
        let keep = self.height - rows;

        let slice = unsafe { self.as_slice() };
        slice.copy_within((rows * stride).., 0);
        slice[(keep * stride)..].fill(0);
    }

    pub fn read_pixel(&mut self, x: usize, y: usize) -> u32 {
        let idx = match self.pos_to_idx(x, y) {
            Some(x) => x,
            None => return 0
        };

        let slice = unsafe { self.as_slice() };
        slice[idx]
    }

    pub unsafe fn as_slice(&mut self) -> &mut [u32] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr as *mut u32, self.stride * self.height) }
    }

    fn pos_to_idx(&mut self, x: usize, y: usize) -> Option<usize> {
        if x >= self.width || y >= self.height { return None }
        Some(y * self.stride + x)
    }
}
