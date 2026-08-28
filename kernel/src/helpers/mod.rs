mod interrupt_mutex;
mod late_init;
mod crc;
mod time;
mod hash;

pub use interrupt_mutex::*;
pub use late_init::LateInit;
pub use time::Time;
pub use crc::*;
pub use hash::fnv1a;

pub fn wait_while<F: Fn() -> bool>(f: F) {
    while f() { core::hint::spin_loop() }
}

pub fn wait_timeout<F: Fn() -> bool>(f: F, timeout: &Time) -> bool {
    let start = crate::acpi::passed_nanos();
    while f() {
        if crate::acpi::passed_nanos() - start > timeout.to_nanos() { return false }
        core::hint::spin_loop();
    }
    true
}

pub fn wait(time: &Time) {
    let start = crate::acpi::passed_nanos();
    while crate::acpi::passed_nanos() - start < time.to_nanos() { core::hint::spin_loop() }
}