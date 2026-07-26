use core::fmt;
use crate::display::{Color, FrameBuffer, Terminal};
use crate::helpers::InterruptMutex;

static TERMINAL: InterruptMutex<Option<Terminal<'static>>> = InterruptMutex::new(None);

pub fn init(fb_raw: &'static limine::framebuffer::Framebuffer) -> &'static InterruptMutex<Option<Terminal<'static>>> {
    let fb = FrameBuffer::new(fb_raw);
    *TERMINAL.lock() = Some(Terminal::new(fb));
    &TERMINAL
}

pub fn clear() {
    if let Some(ref mut t) = *TERMINAL.lock() {
        t.clear();
    }
}

pub fn set_colors(fg: Color, bg: Color) {
    if let Some(ref mut t) = *TERMINAL.lock() {
        t.set_colors(fg, bg);
    }
}

pub fn _print(args: fmt::Arguments) {
    if let Some(ref mut t) = *TERMINAL.lock() {
        fmt::write(t, args).ok();
    }
}

pub fn _log_success(args: fmt::Arguments) {
    let mut guard = TERMINAL.lock();
    if let Some(ref mut t) = *guard {
        t.set_colors(Color::GREEN, Color::BLACK);
        t.print_str("[OK] ");
        t.set_colors(Color::WHITE, Color::BLACK);
        fmt::write(t, args).ok();
        t.print_str("\n");
    }
}

pub fn _log_warning(args: fmt::Arguments) {
    let mut guard = TERMINAL.lock();
    if let Some(ref mut t) = *guard {
        t.set_colors(Color::YELLOW, Color::BLACK);
        t.print_str("[WARN] ");
        t.set_colors(Color::WHITE, Color::BLACK);
        fmt::write(t, args).ok();
        t.print_str("\n");
    }
}

pub fn _log_error(args: fmt::Arguments) {
    let mut guard = TERMINAL.lock();
    if let Some(ref mut t) = *guard {
        t.set_colors(Color::RED, Color::BLACK);
        t.print_str("[ERROR] ");
        t.set_colors(Color::WHITE, Color::BLACK);
        fmt::write(t, args).ok();
        t.print_str("\n");
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::stdout::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ({
        $crate::print!($($arg)*);
        $crate::print!("\n");
    })
}

#[macro_export]
macro_rules! log_success {
    ($($arg:tt)*) => ($crate::stdout::_log_success(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => ($crate::stdout::_log_warning(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => ($crate::stdout::_log_error(format_args!($($arg)*)));
}
