//! Framebuffer text console with PSF2 font rendering.
//!
//! Provides a global [`Writer`] that draws glyphs directly to a linear
//! framebuffer with foreground/background color support. Exports
//! `print!`, `println!`, `warn!`, `log_error!`, and `log_success!`
//! macros that write through this console.

use crate::{boot::FrameBuffer, library::LateInit};

use core::fmt::{Arguments, Display, Result, Write};
use lazy_static::lazy_static;
use psf2::{Font, Glyph};
use x86_64::instructions::interrupts::without_interrupts;

#[repr(u32)]
pub enum FGColor {
    Black   = 0x000000,
    White   = 0xFFFFFF,
    Red     = 0xFF0000,
    Green   = 0x00FF00,
    Blue    = 0x0000FF,
    Yellow  = 0xFFFF00,
    Cyan    = 0x00FFFF,
    Magenta = 0xFF00FF,
    Orange  = 0xFF8000,
    Gray    = 0x808080,
    Custom(u32)
}

#[repr(u32)]
pub enum BGColor {
    Black   = 0x000000,
    White   = 0xFFFFFF,
    Red     = 0xFF0000,
    Green   = 0x00FF00,
    Blue    = 0x0000FF,
    Yellow  = 0xFFFF00,
    Cyan    = 0x00FFFF,
    Magenta = 0xFF00FF,
    Orange  = 0xFF8000,
    Gray    = 0x808080,
    Custom(u32)
}

impl Display for FGColor {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> Result {
        Ok(())
    }
}

impl Display for BGColor {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> Result {
        Ok(())
    }
}

#[derive(Debug)]
struct Writer {
    framebuffer: FrameBuffer,
    bg_color: u32,
    color: u32,
    x: usize,
    y: usize,
}

impl Writer {
    fn new(framebuffer: FrameBuffer, bg_color: u32, color: u32) -> Self {
        Self {
            framebuffer,
            bg_color,
            color,
            x: 0,
            y: 0,
        }
    }

    fn draw_glyph(&mut self, g: Glyph) {
        for (row_idx, row) in g.enumerate() {
            for (col_idx, draw) in row.enumerate() {
                let px = self.x + col_idx;
                let py = self.y + row_idx;
                let color = match draw {
                    true => self.color,
                    false => self.bg_color,
                };
                self.framebuffer.draw_pixel(px, py, color);
            }
        }
    }

    fn write_char(&mut self, c: char) {
        if c == '\n' {
            self.new_line();
            return;
        }

        if c == '\t' {
            let tab_jump = FONT.width() as usize * 8;
            if self.x + tab_jump > self.framebuffer.stride / 4 {
                self.new_line();
                return;
            }
            self.x += tab_jump;
            return;
        }

        if self.x + FONT.width() as usize > self.framebuffer.stride / 4 {
            self.new_line();
        }

        let glyph = match FONT.get_ascii(c as u8) {
            Some(glyph) => glyph,
            None => FONT.get_ascii(b'?').expect("Failed to get font data for ?"),
        };
        self.draw_glyph(glyph);
        self.x += FONT.width() as usize;
    }

    pub fn write_string(&mut self, s: &str) {
        for c in s.chars() {
            self.write_char(c);
        }
    }

    fn new_line(&mut self) {
        let font_height = FONT.height() as usize;
        self.x = 0;
        if self.y + font_height * 2 > self.framebuffer.height {
            self.framebuffer.scroll_up(font_height);
            self.framebuffer
                .clear_rows(self.framebuffer.height - font_height, font_height);
            return;
        }
        self.y += FONT.height() as usize;
    }

    fn set_screen(&mut self, color: u32) {
        self.framebuffer.fill_screen(color);
        self.x = 0;
        self.y = 0;
    }
}

impl Write for Writer {
    fn write_str(&mut self, s: &str) -> Result {
        self.write_string(s);
        Result::Ok(())
    }
}

lazy_static! {
    static ref FONT: Font<&'static [u8]> = {
        let font_data: &[u8] = include_bytes!("lat0-16.psfu");
        Font::new(font_data).expect("Invalid PSF2 Font")
    };
}

static WRITER: LateInit<Writer> = LateInit::new();

pub fn init_writer(fb: FrameBuffer, bg_color: u32, color: u32) {
    WRITER.init(Writer::new(fb, bg_color, color));
}

pub fn set_colors(fg: u32, bg: u32) {
    let lock = WRITER.get_mut();
    lock.bg_color = bg;
    lock.color = fg;
}

pub fn set_offsets(x: usize, y: usize) {
    let lock = WRITER.get_mut();
    lock.x = x * FONT.width() as usize;
    lock.y = y * FONT.height() as usize;
}

pub fn fill(color: u32) {
    WRITER.get_mut().set_screen(color);
}

pub fn clear() {
    WRITER.get_mut().set_screen(0);
}

#[doc(hidden)]
pub fn _print(args: Arguments) {
    without_interrupts(|| {
        let lock = WRITER.get_mut();
        lock.write_fmt(args)
            .expect("Failed to use write_fmt on writer");
    })
}

pub fn _print_prefixed(prefix: &str, fg_prefix: u32, fg_msg: u32, args: Arguments) {
    without_interrupts(|| {
        let lock = WRITER.get_mut();
        lock.color = fg_prefix;
        lock.write_string(prefix);
        lock.color = fg_msg;
        lock.write_fmt(args)
            .expect("Failed to use write_fmt on writer");
    })
}

pub fn _print_with_colors(fg: u32, bg: u32, args: Arguments) {
    without_interrupts(|| {
        let lock = WRITER.get_mut();
        lock.color = fg;
        lock.bg_color = bg;
        lock.write_fmt(args)
            .expect("Failed to use write_fmt on writer");
    })
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::stdout::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

#[macro_export]
macro_rules! warn {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::stdout::_print_prefixed("[WARNING] ", 0xFFFF00, 0xFFFFFF, format_args!("{}\n", format_args!($($arg)*))));
}

#[macro_export]
macro_rules! log_error {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::stdout::_print_prefixed("[ERROR] ", 0xFF0000, 0xFFFFFF, format_args!("{}\n", format_args!($($arg)*))));
}

#[macro_export]
macro_rules! log_success {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::stdout::_print_prefixed("[OK] ", 0x00FF00, 0xFFFFFF, format_args!("{}\n", format_args!($($arg)*))));
}