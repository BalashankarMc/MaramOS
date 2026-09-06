use core::sync::atomic::{AtomicU64, Ordering};

use alloc::{sync::Arc, vec::Vec};
use x86_64::VirtAddr;

use crate::{KResult, LateInit, descriptors::SELECTORS, helpers::Time, memory::{PAGE_SIZE, PhysPage, Stack, VMemAllocator}};

static ID: AtomicU64 = AtomicU64::new(0);
pub static IDLE_TASK: LateInit<Task> = LateInit::new();

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskState {
    Ready,
    Wait(Time),
    Terminate
}

#[derive(Debug, Clone)]
pub enum TaskType {
    Kernel,
    User {
        address_space: Arc<(VMemAllocator, PhysPage)>,
        syscall_stack: Arc<Stack>,
        priv_stack: Arc<Stack>
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    pub id: u64,
    pub type_: TaskType,
    pub state: TaskState,
    pub stack: Arc<PhysPage>, // Use `PhysPage` for arbitrary size
    pub pages: Vec<Arc<PhysPage>>,
    pub entry: usize,
    pub sp: usize
}

impl Task {
    pub fn new_kernel(entry: fn(), stack: PhysPage) -> Self {
        let entry = entry as *const () as usize;
        let rsp = stack.address().as_u64() + (stack.count * PAGE_SIZE) as u64;
        let cs = SELECTORS.kernel_code.0;
        let ds = SELECTORS.kernel_data.0;

        Self {
            id: next_id(),
            state: TaskState::Ready,
            type_: TaskType::Kernel,
            sp: write_stack(&stack, entry as u64, rsp, cs, ds) + stack.address().as_u64() as usize,
            stack: Arc::new(stack),
            pages: Vec::new(),
            entry
        }
    }

    pub fn new_user(entry: VirtAddr, rsp: VirtAddr, stack: PhysPage, address_space: (VMemAllocator, PhysPage)) -> KResult<Self> {
        let cs = SELECTORS.user_code.0;
        let ds = SELECTORS.user_data.0;
        let stack = Arc::new(stack);
        let sp = write_stack(&stack, entry.as_u64(), rsp.as_u64(), cs, ds) + stack.address().as_u64() as usize; 

        Ok(Self {
            id: next_id(),
            state: TaskState::Ready,
            type_: TaskType::User {
                address_space: Arc::new(address_space),
                syscall_stack: Arc::new(Stack::new()?),
                priv_stack: Arc::new(Stack::new()?)
            },
            sp,
            stack,
            pages: Vec::new(),
            entry: entry.as_u64() as usize
        })
    }
}

fn next_id() -> u64 { ID.fetch_add(1, Ordering::SeqCst) }

/// Even though `PhysPage::write_data` returns a `Result`, we can safely ignore it as we know
/// that the offset `sp` is always in-bounds                                                        
fn write_stack(stack: &PhysPage, entry: u64, rsp: u64, cs: u16, ds: u16) -> usize {
    let mut sp = stack.count * PAGE_SIZE;
    sp -= 8;
    let _ = stack.write_data(sp, u64::from(ds));

    sp -= 8;
    let _ = stack.write_data(sp, rsp);

    sp -= 8;
    let _ = stack.write_data::<u64>(sp, 0x202);

    sp -= 8;
    let _ = stack.write_data(sp, u64::from(cs));

    sp -= 8;
    let _ = stack.write_data(sp, entry);

    sp -= 8 * 15;

    sp
}