//! Cross-CPU task load balancing.
//!
//! Counts ready tasks per CPU, computes surplus/deficit, and directly
//! migrates tasks between schedulers. Kicks idle CPUs via IPI. Rate-
//! limited to 500 ms intervals to avoid excessive balancing overhead.

use super::task::Task;
use crate::library::{InterruptMutex, LateInit};
use alloc::{vec::Vec, vec};
use core::sync::atomic::Ordering;

struct RebalanceScratch {
    counts: Vec<usize>,
    surplus: Vec<isize>,
}

static REBALANCE_SCRATCH: LateInit<InterruptMutex<RebalanceScratch>> = LateInit::new();

pub fn rebalance(bsp_ready: &mut Vec<Task>) {
    let now = crate::acpi::passed_nanos();
    let last = crate::cpu::ipi::LAST_REBALANCE.load(Ordering::Relaxed);
    if now - last < 500_000_000 {
        return;
    }
    crate::cpu::ipi::LAST_REBALANCE.store(now, Ordering::Relaxed);

    let cpu_count = super::scheduler::SCHEDULERS.len();
    if cpu_count <= 1 {
        return;
    }

    // Lazy-init scratch buffers sized exactly to cpu_count.
    if REBALANCE_SCRATCH.try_get().is_none() {
        REBALANCE_SCRATCH.init(InterruptMutex::new(RebalanceScratch {
            counts: vec![0; cpu_count],
            surplus: vec![0; cpu_count]
        }));
    }

    let mut scratch = REBALANCE_SCRATCH.lock();

    // Phase 1: count tasks on each CPU.
    let mut total = bsp_ready.len();
    scratch.counts[0] = total;
    for ap in 1..cpu_count {
        if let Some(guard) = super::scheduler::SCHEDULERS[ap].try_lock() {
            scratch.counts[ap] = guard.ready_queue.len();
            total += scratch.counts[ap];
        }
        else { scratch.counts[ap] = 0 }
    }
    if total == 0 { return }

    let (base, rem) = (total / cpu_count, total % cpu_count);
    for cpu in 0..cpu_count {
        let target = base + (cpu < rem) as usize;
        scratch.surplus[cpu] = scratch.counts[cpu] as isize - target as isize;
    }

    // Phase 2: move tasks directly from overloaded CPUs to underloaded CPUs.
    let (mut src, mut dst) = (0, 0);
    while src < cpu_count && dst < cpu_count {
        while src < cpu_count && scratch.surplus[src] <= 0 { src += 1 }
        while dst < cpu_count && scratch.surplus[dst] >= 0 { dst += 1 }
        if src >= cpu_count || dst >= cpu_count { break }

        let n = scratch.surplus[src].min(-scratch.surplus[dst]) as usize;
        for _ in 0..n {
            let task = if src == 0 {
                bsp_ready.pop().unwrap()
            } else {
                match super::scheduler::SCHEDULERS[src].try_lock() {
                    Some(mut g) => g.ready_queue.pop().unwrap(),
                    None => {
                        scratch.surplus[src] = 0;
                        break;
                    }
                }
            };

            if dst == 0 { bsp_ready.push(task) }
            else {
                match super::scheduler::SCHEDULERS[dst].try_lock() {
                    Some(mut g) => g.ready_queue.push(task),
                    None => {
                        if src == 0 { bsp_ready.push(task) }
                        else if let Some(mut g) = super::scheduler::SCHEDULERS[src].try_lock() {
                            g.ready_queue.push(task)
                        }
                        break
                    }
                }
            }
        }
        scratch.surplus[src] -= n as isize;
        scratch.surplus[dst] += n as isize;
    }

    // Phase 3: kick idle CPUs.
    let idle = crate::cpu::ipi::IDLE_CPUS.swap(0, Ordering::SeqCst);
    for ap in 1..cpu_count { if idle & (1 << ap) != 0 { crate::cpu::ipi::sched_kick(ap as u64) } }
}