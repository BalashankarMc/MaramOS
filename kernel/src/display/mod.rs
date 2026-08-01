mod framebuffer;
mod terminal;

pub use framebuffer::FrameBuffer;
pub use terminal::Terminal;

pub struct Color(u32);

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        let mut res = 0;
        res |= (r as u32) << 16;
        res |= (g as u32) << 8;
        res |= b as u32;

        Self(res)
    }

    pub const fn from_rgb(hex: u32) -> Self {
        Self(hex)
    }

    pub const fn as_u32(&self) -> u32 { self.0 }

    pub const BLACK:   Self = Self::from_rgb(0x00_00_00);
    pub const WHITE:   Self = Self::from_rgb(0xFF_FF_FF);
    pub const RED:     Self = Self::from_rgb(0xFF_00_00);
    pub const GREEN:   Self = Self::from_rgb(0x00_FF_00);
    pub const YELLOW:  Self = Self::from_rgb(0xFF_FF_00);
    pub const CYAN:    Self = Self::from_rgb(0x00_FF_FF);
    pub const MAGENTA: Self = Self::from_rgb(0xFF_00_FF);
}