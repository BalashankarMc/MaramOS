//! COM1 serial port driver (16550 UART).
//!
//! Provides blocking byte-level I/O for debug output over the first
//! serial port. Only compiled with the `integration-test` feature.

use core::fmt::{Write, Arguments, Result};
use spin::Mutex;
use x86_64::instructions::port::Port;

pub struct SerialPort {
    data: Port<u8>,
    ier: Port<u8>,
    lcr: Port<u8>,
    mcr: Port<u8>,
    lsr: Port<u8>,
}

impl SerialPort {
    pub const fn new() -> Self {
        Self {
            data: Port::new(0x3F8),
            ier: Port::new(0x3F9),
            lcr: Port::new(0x3FB),
            mcr: Port::new(0x3FC),
            lsr: Port::new(0x3FD),
        }
    }

    pub fn init(&mut self) {
        unsafe {
            // Disable interrupts
            self.ier.write(0x00);

            // Set DLAB to program baud rate
            self.lcr.write(0x80);

            // Divisor = 115200 / 9600 = 12 → 0x000C
            self.data.write(0x0C); // LSB
            self.ier.write(0x00);  // MSB

            // Line control: 8 bits, no parity, 1 stop bit
            self.lcr.write(0x03);

            // Enable FIFO, clear buffers, 14-byte threshold
            let mut fcr: Port<u8> = Port::new(0x3FA);
            fcr.write(0xC7);

            // Modem control: DTR + RTS + Aux Out 2
            self.mcr.write(0x0B);
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        while unsafe { self.lsr.read() } & 0x20 == 0 {}
        unsafe { self.data.write(byte); }
    }
}

impl Write for SerialPort {
    fn write_str(&mut self, s: &str) -> Result {
        for &b in s.as_bytes() {
            if b == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(b);
        }
        Ok(())
    }
}

pub static SERIAL: Mutex<Option<SerialPort>> = Mutex::new(None);

pub fn init() {
    let mut lock = SERIAL.lock();
    let mut port = SerialPort::new();
    port.init();
    *lock = Some(port);
}

#[doc(hidden)]
pub fn _serial_print(args: Arguments) {
    use core::fmt::Write;
    let mut lock = SERIAL.lock();
    if let Some(ref mut port) = *lock {
        let _ = port.write_fmt(args);
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => ($crate::drivers::serial::_serial_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}
