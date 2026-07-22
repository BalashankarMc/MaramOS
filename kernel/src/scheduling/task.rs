//! Kernel task representation and stack initialisation.
//!
//! Provides [`Task`], [`TaskStatus`], and the logic to create a new kernel
//! task with a properly set-up interrupt stack frame ready for its first
//! context switch.

use crate::{fpu::FpuState, memory::PhysPage};

use crate::memory::{KMemory, PAGE_SIZE, VirtualMemoryArea};

use alloc::vec::Vec;
use x86_64::PhysAddr;
static ID: super::InterruptMutex<usize> = super::InterruptMutex::new(0);

/// Execution status of a task.
#[derive(PartialEq, PartialOrd)]
pub enum TaskStatus {
    Ready,
    Wait,
    Terminate,
}

#[repr(align(64))]
pub struct Task {
    pub id: usize,
    pub page_table: PhysAddr,
    pub kstack: PhysPage,
    pub vmas: Vec<VirtualMemoryArea>,
    pub sp: usize,
    pub entry: fn(),
    pub status: TaskStatus,
    pub wake_time: u64,
    pub niceness: i8,
    pub last_exec: u64,
    pub last_burst: f64,
    pub predicted_burst: f64,
    pub fpu_state: FpuState,
    pub syscall_buffer: PhysPage,
    pub pcid: u16,
    pub last_cpu: Option<u64>,
    pub pages: Vec<PhysPage>
}

impl Task {
    /// Create a new instance of the IDLE task (halt loop)
    pub fn idle() -> Self {
        let id = get_id();

        let mut stack_pages = KMemory::alloc_page();
        let stack_top = stack_pages.get_virt_addr().as_u64();

        let selectors = unsafe { &(*crate::cpu::this_cpu()).selectors };

        let sp = write_interrupt_stack(
            &mut stack_pages,
            None,
            stack_top + PAGE_SIZE as u64,
            selectors.kernel_data.0,
            selectors.kernel_code.0,
        );

        let mut task = Self {
            id,
            page_table: KMemory::kernel_l4().start_address(),
            kstack: KMemory::alloc_page(),
            vmas: Vec::new(),
            sp: stack_top as usize + sp,
            entry: || crate::halt_loop(),
            status: TaskStatus::Ready,
            wake_time: 0,
            niceness: i8::MAX,
            last_exec: crate::acpi::passed_nanos(),
            last_burst: 1.0,
            predicted_burst: 1.0,
            fpu_state: FpuState::new(),
            syscall_buffer: KMemory::alloc_page(),
            pcid: (id & 0xFFF).max(1) as u16,
            last_cpu: None,
            pages: Vec::new()
        };
        task.fpu_state.init_state();
        task
    }

    /// Adds a new User task to the scheduler
    pub fn new_task(binary: crate::loader::LoadedBinary) -> Self {
        let id = get_id();
        let mut kernel_stack_pages = KMemory::alloc_pages(32);
        let stack_base = kernel_stack_pages.get_virt_addr().as_u64();

        let selectors = unsafe { &(*crate::cpu::this_cpu()).selectors };

        let sp = write_interrupt_stack(
            &mut kernel_stack_pages,
            Some(binary.entry),
            binary.user_sp,
            selectors.user_data.0,
            selectors.user_code.0,
        );

        let mut task = Self {
            id,
            page_table: binary.page_table,
            kstack: kernel_stack_pages,
            vmas: binary.vmas,
            sp: stack_base as usize + sp,
            entry: || {},
            status: TaskStatus::Ready,
            wake_time: 0,
            niceness: 0,
            last_exec: crate::acpi::passed_nanos(),
            last_burst: 1.0,
            predicted_burst: 1.0,
            fpu_state: FpuState::new(),
            syscall_buffer: binary.syscall_buffer,
            pcid: (id & 0xFFF).max(1) as u16,
            last_cpu: None,
            pages: Vec::new()
        };
        task.fpu_state.init_state();
        task
    }
}

/// Wrapper that looks up the active task's entry function, calls it, then
/// terminates the task.
fn task_wrapper() {
    let function = super::get_active_task_entry();
    function();

    super::remove_active_task();
    crate::halt_loop()
}

/// Atomically allocate a new, unique task ID.
fn get_id() -> usize {
    let mut id_lock = ID.lock();
    let id = *id_lock;
    *id_lock += 1;

    id
}

/// Lay out an `iretq` interrupt frame at the top of `stack` so the first
/// context switch returns into `entry` (or [`task_wrapper`] if `entry` is
/// `None`).
///
/// Frame layout (bottom to top): 15 zeroed GPR slots, RIP, CS, RFlags, RSP, SS.
/// Returns the offset from the stack base to the final stack pointer.
//
fn write_interrupt_stack(stack: &mut PhysPage, entry: Option<u64>, rsp: u64, ss: u16, cs: u16) -> usize {
    let mut sp = stack.size() * PAGE_SIZE;

    let entry = match entry {
        Some(p) => p,
        None => task_wrapper as *const () as u64,
    };

    sp -= 8;
    stack.write_data(sp, ss as u64); // SS

    sp -= 8;
    stack.write_data(sp, rsp); // Stack Pointer

    sp -= 8;
    stack.write_data(sp, 0x202_u64); // RFlags (Enable Interrupts)

    sp -= 8;
    stack.write_data(sp, cs as u64); // CS

    sp -= 8;
    stack.write_data(sp, entry); // Instruction Pointer

    for _ in 0..15 {
        sp -= 8;
        stack.write_data(sp, 0_u64); // Set GPRs (r15-rax) to 0
    }

    sp
}
