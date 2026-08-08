mod interrupt_mutex;
mod late_init;

pub use interrupt_mutex::*;
pub use late_init::LateInit;

pub fn wait_for<F: Fn() -> bool>(f: F) {
    while f() { core::hint::spin_loop() }
}