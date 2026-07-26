mod framebuffer;
pub(crate) mod terminal;

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

    pub const BLACK:   Color = Color::from_rgb(0x000000);
    pub const WHITE:   Color = Color::from_rgb(0xFFFFFF);
    pub const RED:     Color = Color::from_rgb(0xFF0000);
    pub const GREEN:   Color = Color::from_rgb(0x00FF00);
    pub const YELLOW:  Color = Color::from_rgb(0xFFFF00);
    pub const CYAN:    Color = Color::from_rgb(0x00FFFF);
    pub const MAGENTA: Color = Color::from_rgb(0xFF00FF);
}