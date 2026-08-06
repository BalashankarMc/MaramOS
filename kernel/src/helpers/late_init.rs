//! One-shot lazy initialization container.
//!
//! `LateInit` holds a `MaybeUninit<T>` behind an `AtomicBool` flag.
//! It is `Sync` when `T: Send + Sync`, allowing cross-thread initialization.
//! Panics if accessed before `init` or initialized twice.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::Deref;
use core::sync::atomic::{AtomicBool, Ordering};

pub struct LateInit<T> {
    init: AtomicBool,
    data: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send + Sync> Sync for LateInit<T> {}

impl<T> LateInit<T> {
    pub const fn new() -> Self {
        Self {
            init: AtomicBool::new(false),
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn init(&self, val: T) -> &T {
        assert!(
            self.init.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok(),
            "LateInit::init called twice"
        );

        let ptr = self.data.get();

        unsafe {
            (*ptr).write(val);
            (*ptr).assume_init_ref()
        }
    }

    pub fn get(&self) -> &T {
        assert!(self.init.load(Ordering::Acquire));
        unsafe { (*self.data.get()).assume_init_ref() }
    }

    pub fn try_get(&self) -> Option<&T> {
        if !self.init.load(Ordering::Acquire) {
            return None;
        }
        Some(self.get())
    }
}

impl<T> Deref for LateInit<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T> Drop for LateInit<T> {
    fn drop(&mut self) {
        if *self.init.get_mut() {
            unsafe { self.data.get_mut().assume_init_drop(); }
        }
    }
}