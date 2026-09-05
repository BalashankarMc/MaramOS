use core::sync::atomic::{AtomicU64, Ordering};

use alloc::{sync::Arc, vec::Vec};

use crate::{LateInit, descriptors::SELECTORS, helpers::Time, memory::{PAGE_SIZE, PhysPage}};

static ID: AtomicU64 = AtomicU64::new(0);
pub static IDLE_TASK: LateInit<Task> = LateInit::new();

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskState {
    Ready,
    Wait(Time),
    Terminate
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: u64,
    pub state: TaskState,
    pub stack: Arc<PhysPage>,
    pub pages: Vec<Arc<PhysPage>>,
    pub entry: usize,
    pub sp: usize
}

impl Task {
    pub fn new(entry: fn(), stack: Arc<PhysPage>) -> Self {
        let entry = entry as *const () as usize;
        Self {
            id: next_id(),
            state: TaskState::Ready,
            sp: write_stack(&stack, entry as u64) + stack.address().as_u64() as usize,
            stack,
            pages: Vec::new(),
            entry
        }
    }
}

fn next_id() -> u64 { ID.fetch_add(1, Ordering::SeqCst) }

fn write_stack(stack: &Arc<PhysPage>, entry: u64) -> usize {
    let mut sp = stack.count * PAGE_SIZE;
    let rsp = stack.address().as_u64() + sp as u64;

    sp -= 8;
    stack.write_data(sp, u64::from(SELECTORS.kernel_data.0));

    sp -= 8;
    stack.write_data(sp, rsp);

    sp -= 8;
    stack.write_data::<u64>(sp, 0x202);

    sp -= 8;
    stack.write_data(sp, u64::from(SELECTORS.kernel_code.0));

    sp -= 8;
    stack.write_data(sp, entry);

    sp -= 8 * 15;

    sp
}