use core::sync::atomic::{AtomicU64, Ordering};

use alloc::{sync::Arc, vec::Vec};
use x86_64::VirtAddr;
use x86_64::structures::paging::PageTableFlags;

use crate::elf::LoadedBinary;
use crate::memory::VirtualRegion;
use crate::smp::current_cpu;
use crate::{KResult, helpers::Time, memory::{PAGE_SIZE, PhysPage, Stack, VMemAllocator}};

static ID: AtomicU64 = AtomicU64::new(0);

const USER_STACK_PAGES: usize = 16;
const USER_STACK_TOP: VirtAddr = VirtAddr::new(0x7FFF_FFFF_F000);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskState {
    Ready,
    Wait(Time),
    Terminate(u64)
}

#[derive(Debug, Clone)]
pub enum TaskType {
    Kernel,
    User {
        address_space: Arc<(VMemAllocator, PhysPage)>,
        syscall_stack: Arc<Stack>,
        priv_stack: Arc<Stack>,
        parent_id: u64
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
    pub fn manual(entry: fn(), stack: PhysPage, cs: u16, ds: u16) -> Self {
        let entry = entry as *const () as usize;
        let rsp = stack.address().as_u64() + (stack.count * PAGE_SIZE) as u64;

        Self {
            id: next_id(),
            state: TaskState::Ready,
            type_: TaskType::Kernel,
            sp: write_stack(&stack, entry as u64, rsp, cs, ds) + stack.address().as_u64() as usize,
            stack: Arc::new(stack),
            pages: Vec::new(),
            entry,
        }
    }

    pub fn new_kernel(entry: fn(), stack: PhysPage) -> Self {
        let entry = entry as *const () as usize;
        let rsp = stack.address().as_u64() + (stack.count * PAGE_SIZE) as u64;
        let selectors = &current_cpu().selectors;
        let cs = selectors.kernel_code.0;
        let ds = selectors.kernel_data.0;

        Self {
            id: next_id(),
            state: TaskState::Ready,
            type_: TaskType::Kernel,
            sp: write_stack(&stack, entry as u64, rsp, cs, ds) + stack.address().as_u64() as usize,
            stack: Arc::new(stack),
            pages: Vec::new(),
            entry,
        }
    }

    pub fn new_user(entry: VirtAddr, rsp: VirtAddr, stack: PhysPage, address_space: (VMemAllocator, PhysPage), pid: u64) -> KResult<Self> {
        let selectors = &current_cpu().selectors;
        let cs = selectors.user_code.0;
        let ds = selectors.user_data.0;
        let stack = Arc::new(stack);
        let sp = write_stack(&stack, entry.as_u64(), rsp.as_u64(), cs, ds) + stack.address().as_u64() as usize; 

        Ok(Self {
            id: next_id(),
            state: TaskState::Ready,
            type_: TaskType::User {
                address_space: Arc::new(address_space),
                syscall_stack: Arc::new(Stack::new()?),
                priv_stack: Arc::new(Stack::new()?),
                parent_id: pid,
            },
            sp,
            stack,
            pages: Vec::new(),
            entry: entry.as_u64() as usize
        })
    }

    pub fn from_binary(mut bin: LoadedBinary, parent_id: u64) -> KResult<Self> {
        let kstack = Stack::new()?;
        let ustack = PhysPage::new(USER_STACK_PAGES)?;

        let size = (USER_STACK_PAGES * PAGE_SIZE) as u64;
        let region = VirtualRegion { start: USER_STACK_TOP - size, size };

        bin.address_space.0.map_fixed(
            region,
            ustack.phys(),
            PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE | PageTableFlags::USER_ACCESSIBLE
        )?;

        let mut task = Self::new_user(bin.entry, USER_STACK_TOP, kstack.0, bin.address_space, parent_id)?;
        task.pages.extend(bin.segments.into_iter().map(Arc::new));
        task.pages.push(Arc::new(ustack));

        Ok(task)
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