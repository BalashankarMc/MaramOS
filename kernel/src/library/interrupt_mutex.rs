//! Interrupt-safe spin mutex wrapper.
//!
//! [`InterruptMutex`] disables interrupts on lock acquisition and restores
//! the previous interrupt state on drop, preventing deadlocks from timer
//! interrupts while the lock is held.

use spin::{Mutex, MutexGuard};
use core::{mem::ManuallyDrop, ops::{Deref, DerefMut}};
use x86_64::instructions::interrupts;

/// An Interrupt-safe wrapper around a Spin::Mutex
pub struct InterruptMutex<T>(Mutex<T>);

/// An Interrupt-safe wrapper around a Spin::MutexGuard. Implements Drop and restores previous interrupt status on drop.
pub struct InterruptGuard<'a, T>(ManuallyDrop<MutexGuard<'a, T>>, bool);

impl<T> InterruptMutex<T> {
    /// Creates a new instance of InterruptMutex
    pub const fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    /// Locks the Mutex, disables interrupts and returns a MutexGuard
    pub fn lock(&self) -> InterruptGuard<'_, T> {
        let int_status = interrupts::are_enabled();
        interrupts::disable();
        InterruptGuard(ManuallyDrop::new(self.0.lock()), int_status)
    }

    /// Tries to lock the Mutex. If the lock is held, returns None
    pub fn try_lock(&self) -> Option<InterruptGuard<'_, T>> {
        let int_status = interrupts::are_enabled();
        interrupts::disable();
        match self.0.try_lock() {
            Some(guard) => Some(InterruptGuard(ManuallyDrop::new(guard), int_status)),
            None => {
                if int_status {
                    interrupts::enable();
                }
                None
            }
        }
    }
}

impl<T> Drop for InterruptGuard<'_, T> {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.0) };
        if self.1 {
            interrupts::enable()
        }
    }
}

impl<T> Deref for InterruptGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T> DerefMut for InterruptGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}