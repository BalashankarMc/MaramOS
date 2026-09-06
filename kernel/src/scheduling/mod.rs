use core::arch::naked_asm;

use x86_64::structures::idt::InterruptStackFrame;

use crate::{
    InterruptMutex, KResult, LateInit,
    acpi::{init_lapic_timer, lapic_eoi},
    descriptors::{HardwareInterrupts, add_idt_entry},
    helpers::Time,
    memory::PhysPage
};

pub use self::{scheduler::Scheduler, task::Task};

mod task;
mod scheduler;

static SCHEDULER: LateInit<InterruptMutex<Scheduler>> = LateInit::new();
pub const TIMER_TICK: Time = Time::Milliseconds(100);

pub fn init() -> KResult<()> {
    let stack = PhysPage::new(1)?;
    let halt_task = Task::new_kernel(|| crate::halt_loop(), stack);

    task::IDLE_TASK.init(halt_task);
    SCHEDULER.init(InterruptMutex::new(Scheduler::new()));

    // Register interrupt handler
    add_idt_entry(timer_handler, HardwareInterrupts::Timer.as_u8())?;
    // Unmask interrupt
    init_lapic_timer(HardwareInterrupts::Timer.as_u8(), TIMER_TICK);

    Ok(())
}

pub fn add_task(task: Task) { SCHEDULER.lock().add_task(task) }

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