//! Per-CPU scheduler with priority-based task selection and lazy FPU save/restore.
//!
//! Each CPU owns a [`Scheduler`] holding ready, sleep, and terminate queues.
//! The [`schedule`] function (entered via timer IPI or explicit yield) saves
//! the current task's GPRs, picks the highest-priority ready task using
//! burst-based prediction, and performs the context switch.
//!
//! # FPU lazy save/restore protocol (x86-64)
//!
//! Instead of eagerly saving/restoring the full 512-byte FPU/SSE state on
//! every context switch, the scheduler sets CR0.TS (Task Switched) in the
//! [`schedule`] trampoline after restoring GPRs.  If the incoming task never
//! touches the FPU, no save/restore occurs.  On the first FPU instruction
//! the CPU raises #NM (vector 7); the handler saves the *previous* owner's
//! state (if any), loads the current task's saved state, and clears CR0.TS.
//!
//! The naked [`schedule`] entry pushes all callee-saved and caller-saved
//! GPRs (rax–r15, rbp) onto the outgoing task's stack, calls
//! [`schedule_logic`] with RSP as argument, restores GPRs from the *incoming*
//! task's stack, sets CR0.TS, and issues `iretq`.  The frame has the iretq
//! vector (SS, RSP, RFLAGS, CS, RIP) at the top so `iretq` naturally pops
//! the next instruction.
//!
//! Load balancing is triggered from the BSP's schedule path when other CPUs
//! are idle.

use super::{task::Task, task::TaskStatus, InterruptMutex};
use crate::{cpu::cpu_id, library::LateInit, memory::KMemory};

use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use x86_64::{
    registers::control::Cr3,
    instructions::tlb::Pcid,
    structures::paging::PhysFrame
};

use core::arch::naked_asm;


/// Per-CPU scheduler state.
#[repr(align(64))]
pub struct Scheduler {
    pub curr_task: Task,
    pub ready_queue: Vec<Task>,
    pub sleep_queue: Vec<Task>,
    pub terminate_queue: Vec<Task>,
}

impl Scheduler {
    /// Create a new scheduler with an idle task as the current task.
    pub fn new() -> Self {
        Self {
            curr_task: Task::idle(),
            ready_queue: Vec::with_capacity(10),
            sleep_queue: Vec::with_capacity(10),
            terminate_queue: Vec::with_capacity(5),
        }
    }
}

/// Global array of per-CPU schedulers.
pub static SCHEDULERS: LateInit<Vec<InterruptMutex<Scheduler>>> = LateInit::new();

/// Core scheduling logic invoked from the assembly [`schedule`] trampoline.
///
/// Saves the outgoing task's stack pointer, wakes expired sleepers, selects
/// the highest-priority ready task, performs the context switch (including
/// optional page-table swap), and triggers load balancing when other CPUs
/// are idle.
pub extern "C" fn schedule_logic(old_sp: usize) -> usize {

    let scheds = match SCHEDULERS.try_get() {
        Some(s) => s,
        None => { crate::acpi::lapic_eoi(); return old_sp }
    };

    if scheds.len() <= cpu_id() as usize {
        crate::acpi::lapic_eoi();
        return old_sp;
    }

    let mut sched = scheds[cpu_id() as usize].lock();
    sched.curr_task.sp = old_sp;

    unsafe {
        let cpu = &mut *crate::cpu::this_cpu();
        if cpu.fpu_owner_id == Some(sched.curr_task.id as u64) { crate::fpu::fxsave(&mut sched.curr_task.fpu_state) }
        cpu.fpu_owner_id = None;
    }

    // Wake sleeping tasks
    let now = crate::acpi::passed_nanos();
    let mut i = sched.sleep_queue.len();
    while i > 0 {
        i -= 1;
        if sched.sleep_queue[i].wake_time <= now {
            let mut task = sched.sleep_queue.remove(i);
            task.status = TaskStatus::Ready;
            sched.ready_queue.push(task);
        }
    }

    // Find task with top priority
    let mut top_prior = [0, 0];
    for (i, task) in sched.ready_queue.iter().enumerate() {
        let prior = calculate_priority(task) as usize;
        if top_prior[0] < prior {
            top_prior[0] = prior;
            top_prior[1] = i;
        }
    }

    // Signal an EOI
    crate::acpi::lapic_eoi();

    if sched.ready_queue.is_empty() {
        if crate::cpu::cpu_id() != 0 {
            crate::cpu::ipi::notify_bsp_idle()
        }
        match sched.curr_task.status {
            TaskStatus::Wait => {
                let t = core::mem::replace(&mut sched.curr_task, Task::idle());
                sched.sleep_queue.push(t)
            }
            TaskStatus::Terminate => sched.curr_task = Task::idle(),
            TaskStatus::Ready => {}
        }
        return sched.curr_task.sp;
    }

    let now = crate::acpi::passed_nanos();
    sched.curr_task.last_burst = (now - sched.curr_task.last_exec) as f64;
    predict_burst(&mut sched.curr_task);

    let new_task = sched.ready_queue.remove(top_prior[1]);
    let last_task = core::mem::replace(&mut sched.curr_task, new_task);

    let next_pcid = Pcid::new(sched.curr_task.pcid).unwrap();
    let next_frame = PhysFrame::containing_address(sched.curr_task.page_table);
    if sched.curr_task.last_cpu == Some(cpu_id()) { unsafe { Cr3::write_pcid_no_flush(next_frame, next_pcid) } }
    else { unsafe { Cr3::write_pcid(next_frame, next_pcid) } }

    sched.curr_task.last_cpu = Some(cpu_id());

    match last_task.status {
        TaskStatus::Ready => sched.ready_queue.push(last_task),
        TaskStatus::Wait => sched.sleep_queue.push(last_task),
        TaskStatus::Terminate => sched.terminate_queue.push(last_task),
    }

    let kstack_top = sched.curr_task.kstack.get_virt_addr();
    unsafe {
        let cpu = &mut *crate::cpu::this_cpu();
        cpu.kernel_rsp = kstack_top.as_u64();
        cpu.tss.privilege_stack_table[0] = kstack_top;
    }

    let kernel_l4 = KMemory::kernel_l4().start_address();
    let dead = sched.terminate_queue.drain(..);

    for task in dead {
        unsafe {
            let cpu = &mut *crate::cpu::this_cpu();
            if cpu.fpu_owner_id == Some(task.id as u64) { cpu.fpu_owner_id = None }
        }
        if task.page_table != kernel_l4 { KMemory::unmap_user_page_table(task.page_table) }
    }

    let sp = sched.curr_task.sp;
    sched.curr_task.last_exec = now;

    if cpu_id() == 0 && SCHEDULERS.len() > 1 {
        let idle = crate::cpu::ipi::IDLE_CPUS.load(Ordering::SeqCst);
        if idle != 0 { super::load_balancing::rebalance(&mut sched.ready_queue) }
    }

    sp
}

/// Enqueue a task on the current CPU's ready queue.
pub fn add_task(t: Task) {
    let mut sched = SCHEDULERS[cpu_id() as usize].lock();
    sched.ready_queue.push(t);
}

/// Mark the currently running task for termination.
pub fn remove_active_task() {
    let mut sched = SCHEDULERS[cpu_id() as usize].lock();
    sched.curr_task.status = TaskStatus::Terminate;
}

/// Run a closure with mutable access to the current task.
pub fn with_active_task<T>(f: impl FnOnce(&mut Task) -> T) -> T {
    let mut sched = SCHEDULERS[cpu_id() as usize].lock();
    f(&mut sched.curr_task)
}

#[unsafe(naked)]
pub extern "C" fn schedule() {
    naked_asm!(
        "push rax", "push rbx", "push rcx", "push rdx",
        "push rsi", "push rdi", "push rbp",
        "push r8", "push r9", "push r10", "push r11",
        "push r12", "push r13", "push r14", "push r15",

        "mov rdi, rsp",
        "and rsp, -16",
        "call {sched_logic}",

        "mov rsp, rax",

        "pop r15", "pop r14", "pop r13", "pop r12",
        "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rbp", "pop rdi", "pop rsi", "pop rdx",
        "pop rcx", "pop rbx", "pop rax",

        "push rax",
        "mov rax, cr0",
        "or rax, 8",
        "mov cr0, rax",
        "pop rax",

        "iretq",

        sched_logic = sym schedule_logic
    );
}

/// Compute a dynamic priority for `task` based on its niceness and predicted
/// burst length. Higher values = more likely to be scheduled.
pub(crate) fn calculate_priority(task: &Task) -> u8 {
    let base = (127i16 - task.niceness as i16) as u8;
    let penalty = (task.predicted_burst / 1_000_000.0).min(127.0) as u8;
    base.saturating_sub(penalty)
}

/// Update the predicted burst duration using an exponential moving average.
/// `new_pred = (old_pred + 7 * last_burst) / 8`
pub(crate) fn predict_burst(task: &mut Task) {
    let a = 0.875;
    task.predicted_burst = a * task.last_burst + (1.0 - a) * task.predicted_burst;
}
