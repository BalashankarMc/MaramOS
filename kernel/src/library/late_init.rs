//! One-shot lazy initialization container.
//!
//! [`LateInit`] holds a `MaybeUninit<T>` behind an `AtomicBool` flag.
//! It is `Sync` when `T: Send`, allowing cross-thread initialization.
//! Panics if accessed before `init` or initialized twice.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct LateInit<T> {
    init: AtomicBool,
    data: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send> Sync for LateInit<T> {}

#[allow(clippy::mut_from_ref)]
impl<T> LateInit<T> {
    pub const fn new() -> Self {
        Self {
            init: AtomicBool::new(false),
            data: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn init(&self, val: T) -> &mut T {
        assert!(!self.init.load(Ordering::Acquire));
        unsafe {
            (*self.data.get()).write(val);
        }
        self.init.store(true, Ordering::Release);
        unsafe { &mut *(*self.data.get()).as_mut_ptr() }
    }

    pub fn get(&self) -> &T {
        assert!(self.init.load(Ordering::Acquire));
        unsafe { (*self.data.get()).assume_init_ref() }
    }

    pub fn get_mut(&self) -> &mut T {
        assert!(self.init.load(Ordering::Acquire));
        unsafe { &mut *(*self.data.get()).as_mut_ptr() }
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

impl<T> DerefMut for LateInit<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.get_mut()
    }
}