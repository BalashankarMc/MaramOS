use core::fmt::Write;

use crate::library::{FrameBuffer, LateInit};

use simple_psf::Psf;
use spin::Mutex;

const FONT_DATA: &[u8] = include_bytes!("../font.psf");

const FONT: Psf = match Psf::parse(FONT_DATA) {
    Ok(f) => f,
    Err(_) => panic!("Failed to get font data!")
};

static WRITER: LateInit<Mutex<Writer>> = LateInit::new();

pub struct Writer {
    framebuffer: FrameBuffer,
    x: usize,
    y: usize,
    fg_color: u32,
    bg_color: u32
}

impl Writer {
    pub fn new(fb: FrameBuffer, fg_color: u32, bg_color: u32) -> Self {
        Self {
            framebuffer: fb,
            x: 0,
            y: 0,
            fg_color,
            bg_color
        }
    }

    fn draw_glyph(&mut self, glyph_index: usize) {
        let pixels = get_glyph(glyph_index);

        for (i, pixel) in pixels.enumerate() {
            let col = i % FONT.glyph_width;
            let row = i / FONT.glyph_width;
            let color = if pixel { self.fg_color } else { self.bg_color };

            self.framebuffer.set_pixel(self.x * FONT.glyph_width + col, self.y * FONT.glyph_height + row, color);
        }
    }

    fn new_line(&mut self) { self.y += 1; self.x = 0 }

    pub fn draw_char(&mut self, c: char) {
        if self.x * FONT.glyph_width >= self.framebuffer.width { self.new_line() }

        if self.y * FONT.glyph_height >= self.framebuffer.height {
            self.framebuffer.scroll(FONT.glyph_height);
            self.y -= 1;
            self.x = 0;
        }

        if c == '\n' { self.new_line(); return }
        if c == '\t' { self.x += 8; return }

        self.draw_glyph(c as usize);
        self.x += 1;

    }

    pub fn write_string(&mut self, s: &str) { for c in s.chars() { self.draw_char(c) } }
}

pub fn init_writer(fb: FrameBuffer, fg_color: u32, bg_color: u32) {
    WRITER.init(Mutex::new(Writer::new(fb, fg_color, bg_color)));
}

pub fn clear() {
    let mut lock = WRITER.lock();
    let color = lock.bg_color;
    lock.framebuffer.fill_screen(color);
    lock.x = 0;
    lock.y = 0;
}

pub fn set_colors(fg: u32, bg: u32) {
    let mut lock = WRITER.lock();
    lock.fg_color = fg;
    lock.bg_color = bg
}

pub fn set_offsets(x: usize, y: usize) {
    let mut lock = WRITER.lock();
    lock.x = x;
    lock.y = y;
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    let _ = WRITER.lock().write_fmt(args);
}

pub fn panic_print(args: core::fmt::Arguments) {
    unsafe { WRITER.get().force_unlock() };
    let mut guard = WRITER.get().lock();
    let _ = guard.write_fmt(args);
}

pub fn _print_prefixed(prefix: &str, fg_prefix: u32, fg_msg: u32, args: core::fmt::Arguments) {
    let mut lock = WRITER.lock();
    lock.fg_color = fg_prefix;
    lock.write_string(prefix);
    lock.fg_color = fg_msg;
    let _ = lock.write_fmt(args);
}

pub fn _print_with_colors(fg: u32, bg: u32, args: core::fmt::Arguments) {
    let mut lock = WRITER.lock();
    lock.fg_color = fg;
    lock.bg_color = bg;
    let _ = lock.write_fmt(args);
}

impl Write for Writer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_string(s);
        Ok(())
    }
}

fn get_glyph(index: usize) -> impl Iterator<Item = bool> {
    FONT.get_glyph_pixels(index).unwrap_or(
        FONT.get_glyph_pixels(b'?' as usize).expect("Failed to get glyph data!")
    )
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
macro_rules! panic_println {
    () => ($crate::stdout::panic_print(format_args!("\n")));
    ($($arg:tt)*) => (
        $crate::stdout::panic_print(format_args!("{}\n", format_args!($($arg)*)))
    );
}

#[macro_export]
macro_rules! warn {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::stdout::_print_prefixed(
        "[WARNING] ", 0xFFFF00, 0xFFFFFF,
        format_args!("{}\n", format_args!($($arg)*))
    ));
}

#[macro_export]
macro_rules! log_error {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::stdout::_print_prefixed(
        "[ERROR] ", 0xFF0000, 0xFFFFFF,
        format_args!("{}\n", format_args!($($arg)*))
    ));
}

#[macro_export]
macro_rules! log_success {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::stdout::_print_prefixed(
        "[OK] ", 0x00FF00, 0xFFFFFF,
        format_args!("{}\n", format_args!($($arg)*))
    ));
}
