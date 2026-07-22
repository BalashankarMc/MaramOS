//! Shared kernel utilities.
//!
//! Provides one-shot lazy initialization ([`LateInit`]), time unit
//! conversions ([`Time`]), an interrupt-safe spin mutex
//! ([`InterruptMutex`]), and a CRC32 IEEE 802.3 implementation.

mod interrupt_mutex;
mod late_init;
mod time;
mod crc32;

pub use late_init::LateInit;
pub use time::Time;
pub use interrupt_mutex::*;
pub use crc32::crc32;