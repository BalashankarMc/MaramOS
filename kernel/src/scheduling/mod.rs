//! Task scheduling and context-switching.
//!
//! Provides per-CPU schedulers, an interrupt-safe mutex, and primitives for
//! creating kernel tasks, sleeping, yielding, and load balancing across APs.
//!
//! # Safety
//! Adding a malicious task or passing a bad stack pointer can lead to a fault.
use alloc::vec;

use self::{scheduler::Scheduler, task::Task};
use crate::library::InterruptMutex;

mod load_balancing;
pub(crate) mod scheduler;
pub(crate) mod task;

use crate::library::Time;

/// Publicly re-export the core schedule logic to be used by the timer interrupt handler
pub use scheduler::schedule;

/// Initialize a scheduler for a single CPU
pub fn init_scheduler() {
    let sched = InterruptMutex::new(Scheduler::new());
    if scheduler::SCHEDULERS.try_get().is_none() {
        scheduler::SCHEDULERS.init(vec![sched]);
    } else {
        scheduler::SCHEDULERS.get_mut().push(sched);
    }
}

/// Add a task (userspace) to the scheduler
pub fn add_task(binary: crate::loader::LoadedBinary) {
    let t = Task::new_task(binary);
    scheduler::add_task(t);
}

/// Removes the active task in the scheduler
pub fn remove_active_task() {
    scheduler::remove_active_task();
    yield_now();
}

fn get_active_task_entry() -> fn() {
    scheduler::with_active_task(|t| t.entry)
}

/// Sets the currently executing task's state to Wait and sets it wake time
pub fn task_sleep(duration: Time) {
    let wake_time = crate::acpi::passed_nanos() + duration.to_nanos();
    scheduler::with_active_task(|t| {
        t.status = task::TaskStatus::Wait;
        t.wake_time = wake_time;
    });
    yield_now();
}

pub fn yield_now() {
    crate::acpi::trigger_interrupt(crate::descriptors::interrupts::HardwareInterrupts::Timer);
}
