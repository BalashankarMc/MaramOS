use core::arch::naked_asm;
use x86_64::structures::idt::InterruptStackFrame;

use crate::smp::{CPUS, current_cpu};
use crate::{
    KResult,
    acpi::{init_lapic_timer, lapic_eoi},
    descriptors::{HardwareInterrupts, add_idt_entry},
    helpers::Time
};

use self::task::TaskState;
pub use self::{scheduler::Scheduler, task::Task};

mod task;
mod scheduler;

pub const TIMER_TICK: Time = Time::Milliseconds(100);

pub fn idle_task() {
    loop {
        // Lock own scheuler
        let mut lock = current_cpu().scheduler.lock();
        lock.terminate.clear();
        let task_count = lock.task_count().1;
        drop(lock); // `busiest_queue` locks all schedulers

        let Some(busiest) = busiest_queue() else { continue };
        if busiest.0 == current_cpu().id as usize || busiest.1 < 2 { continue }

        // Lock the busiest scheduler
        let mut busy_lock = CPUS[busiest.0].scheduler.lock();
        let take_count = busy_lock.task_count().1.saturating_sub(task_count) / 2;

        let Some(tasks) = busy_lock.take_user_tasks(take_count) else { continue };
        
        drop(busy_lock); // Never hold 2 locks at once

        let mut lock = current_cpu().scheduler.lock();
        lock.ready.extend(tasks);

        drop(lock);

        if take_count > 0 { yield_now() }

        core::hint::spin_loop();
    }
}

/// Returns the CPU ID of the busiest scheduler (one with most user tasks) and the task count
fn busiest_queue() -> Option<(usize, usize)> {
    let mut max = 0;
    let mut busy_cpu = None;

    for (id, cpu) in CPUS.iter().enumerate() {
        let count = cpu.scheduler.lock().task_count().1;

        if count > max {
            max = count;
            busy_cpu = Some(id);
        }
    }

    Some((busy_cpu?, max))
}

/// Returns the CPU ID of the least busiest scheduler
fn laziest_queue() -> Option<(usize, usize)> {
    let mut min = usize::MAX;
    let mut lazy_cpu = None;

    for (id, cpu) in CPUS.iter().enumerate() {
        let count = cpu.scheduler.lock().task_count().1;

        if count == 0 { return Some((id, 0)) }
        if count < min {
            min = count;
            lazy_cpu = Some(id);
        }
    }

    Some((lazy_cpu?, min))
}

pub fn init() -> KResult<()> {
    // Register interrupt handler
    add_idt_entry(timer_handler, HardwareInterrupts::Timer.as_u8())?;
    // Unmask interrupt
    init_lapic_timer(HardwareInterrupts::Timer.as_u8(), TIMER_TICK);

    Ok(())
}

pub fn kill_active_task(err_code: u64) {
    current_cpu().scheduler.lock().curr.state = TaskState::Terminate(err_code);
    yield_now();
}

/// Picks the most free cpu and gives it a new task
pub fn add_task(task: Task) {
    let cpuid = match laziest_queue() {
        Some((cpu, _)) => cpu,
        None => current_cpu().id as usize
    };

    CPUS[cpuid].scheduler.lock().add_task(task);
}

pub fn yield_now() { crate::acpi::trigger_interrupt(HardwareInterrupts::Timer) }

#[unsafe(naked)]
extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    naked_asm!(
        // Save GPRs
        "push rax", "push rbx", "push rcx",
        "push rdx", "push rsi", "push rdi",
        "push rbp", "push r8", "push r9",
        "push r10", "push r11", "push r12",
        "push r13", "push r14", "push r15",

        // Move RSP into arg 1
        "mov rdi, rsp",
        "sub rsp, 8", // Align to 16 bytes before calling functions

        "call {sched}",
        "mov rsp, rax", // Update RSP with the return value

        // Load GPRs
        "pop r15", "pop r14", "pop r13",
        "pop r12", "pop r11", "pop r10",
        "pop r9", "pop r8", "pop rbp",
        "pop rdi", "pop rsi", "pop rdx",
        "pop rcx", "pop rbx", "pop rax",

        // Send EOI
        "call {eoi}",
        
        "iretq",

        sched = sym scheduler::sched_logic,
        eoi = sym lapic_eoi
    );
}