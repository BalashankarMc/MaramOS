use alloc::{collections::VecDeque, vec::Vec};

use crate::acpi::passed_nanos;

use super::{SCHEDULER, task::{IDLE_TASK, Task, TaskState}};

pub struct Scheduler {
    curr: Task,
    ready: VecDeque<Task>,
    terminate: Vec<Task>,
    sleep: Vec<Task>
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            curr: IDLE_TASK.get().clone(),
            ready: VecDeque::new(),
            terminate: Vec::new(),
            sleep: Vec::new()
        }
    }

    pub fn add_task(&mut self, t: Task) { self.ready.push_back(t) }
}

pub extern "C" fn sched_logic(old_sp: usize) -> usize {
    let mut sched = SCHEDULER.lock();

    sched.curr.sp = old_sp;

    // Wake sleeping tasks
    let mut wake_queue = Vec::new();
    for (i, task) in sched.sleep.iter().enumerate() {
        match task.state {
            TaskState::Wait(x) if x.to_nanos() > passed_nanos() => wake_queue.push(i),
            _ => wake_queue.push(i),
        }
    }

    for idx in wake_queue.into_iter().rev() {
        let task = sched.sleep.remove(idx);
        move_to_queue(&mut sched, task);
    }

    let mut next_task = sched.ready.pop_front();
    if next_task.is_none() && sched.curr.state == TaskState::Ready { return sched.curr.sp }
    if next_task.is_none() {
        if sched.curr.state == TaskState::Ready { return sched.curr.sp }
        next_task = Some(IDLE_TASK.clone());
    }

    // Unwrap: Guaranteed to be `Some` on line 52
    let next_task = next_task.unwrap();
    let old_task = core::mem::replace(&mut sched.curr, next_task);
    move_to_queue(&mut sched, old_task);

    sched.curr.sp
}

fn move_to_queue(scheduler: &mut Scheduler, task: Task) {
    match task.state {
        TaskState::Ready => scheduler.ready.push_back(task),
        TaskState::Terminate => scheduler.terminate.push(task),
        TaskState::Wait(_) => {
            scheduler.sleep.push(task);
        }
    }
}