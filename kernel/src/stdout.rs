use crate::boot::FrameBuffer;

use x86_64::instructions::interrupts::without_interrupts;
use core::fmt::{Write, Arguments, Result};
use lazy_static::lazy_static;
use psf2::{Font, Glyph};
use spin::Mutex;

struct Writer {
    framebuffer: FrameBuffer,
    bg_color: u32,
    color: u32,
    x: usize,
    y: usize
}

impl Writer {

    fn new(framebuffer: FrameBuffer, bg_color: u32, color: u32) -> Self {
        Self {
            framebuffer,
            bg_color,
            color,
            x: 0,
            y: 0
        }
    }

    fn draw_glyph(&mut self, g: Glyph) {
        for (row_idx, row) in g.enumerate() {
            for (col_idx, draw) in row.enumerate() {
                let px = self.x + col_idx;
                let py = self.y + row_idx;
                let color = match draw {
                    true => self.color,
                    false => self.bg_color
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
            let tab_jump = (FONT.width() * 8) as usize;
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
            None => FONT.get_ascii('?' as u8).unwrap()
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
            self.framebuffer.clear_rows(self.framebuffer.height - font_height, font_height);
            return;
        }
        self.y += FONT.height() as usize;
    }

    fn clear(&mut self) {
        self.framebuffer.fill_screen(self.bg_color);
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
        let font_data: &[u8] = include_bytes!("../lat0-16.psfu");
        Font::new(font_data).expect("Invalid PSF2 Font")
    };
}

static WRITER: Mutex<Option<Writer>> = Mutex::new(None);

pub fn init_writer(fb: FrameBuffer, bg_color: u32, color: u32) {
    let mut lock = WRITER.lock();
    *lock = Some(Writer::new(fb, bg_color, color));
}

pub fn clear() {
    WRITER.lock().as_mut().unwrap().clear();
}

#[doc(hidden)]
pub fn _print(args: Arguments) {
    without_interrupts(|| {
        let mut lock = WRITER.lock();
        if lock.is_none() {return;}
        lock.as_mut().unwrap().write_fmt(args).expect("");
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

pub unsafe fn force_unlock() {
    unsafe{WRITER.force_unlock();}
}