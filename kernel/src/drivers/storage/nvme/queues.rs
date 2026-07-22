//! NVMe queue pairs (submission + completion) and command descriptors.
//!
//! A [`QueuePair`] owns a submission queue (SQ) and a completion queue (CQ),
//! both allocated as physically-contiguous DMA buffers.  It provides helpers
//! for submitting a command, polling (or interrupt-driven) completion, and
//! managing the phase tag for CQ entry ownership.

use x86_64::PhysAddr;

use super::{IOError, TIMEOUT, registers::NVMeRegisters};

use crate::{acpi::passed_nanos as time, memory::DMABuffer};

/// A paired submission and completion queue for NVMe commands.
///
/// Tracks the SQ tail pointer (where the next command is written) and the
/// CQ head pointer (where the next completion is read).  The `phase` bit
/// toggles each wrap of the CQ to distinguish new completions from stale data.
#[derive(Debug)]
pub struct QueuePair {
    submission_queue: DMABuffer,
    completion_queue: DMABuffer,
    sq_tail: u16,
    cq_head: u16,
    phase: bool,
    queue_id: u16,
    sq_entries: u16,
    cq_entries: u16,
}

impl QueuePair {
    /// Allocate a queue pair in DMA-capable memory.
    ///
    /// Each SQ entry is 64 bytes; each CQ entry is 16 bytes.
    pub fn new(queue_id: u16, queue_size: u16) -> Self {
        let submission_queue = DMABuffer::new(queue_size as usize * 64);
        let completion_queue = DMABuffer::new(queue_size as usize * 16);

        Self {
            submission_queue,
            completion_queue,
            sq_tail: 0,
            cq_head: 0,
            phase: true,
            queue_id,
            sq_entries: queue_size,
            cq_entries: queue_size,
        }
    }

    /// Submit a command by writing it to the next SQ slot and ringing the
    /// submission queue doorbell.
    pub fn submit(&mut self, cmd: &SubmissionQueueEntry, regs: &NVMeRegisters) {
        let sq_virt = self.submission_queue.virt();
        let slot_addr = sq_virt + (self.sq_tail as u64 % self.sq_entries as u64) * 64;
        self.sq_tail = (self.sq_tail + 1) % self.sq_entries;

        unsafe {
            slot_addr
                .as_mut_ptr::<SubmissionQueueEntry>()
                .write_volatile(*cmd);
            regs.doorbell(self.queue_id, false)
                .write_volatile(self.sq_tail as u32);
        }
    }

    /// Non-blocking completion check — returns `Some` if a new CQE is
    /// available, `None` otherwise.
    pub fn try_complete(&mut self, regs: &NVMeRegisters) -> Option<CompletionQueueEntry> {
        let cq_virt = self.completion_queue.virt();
        let slot = cq_virt + (self.cq_head as u64 % self.cq_entries as u64) * 16;
        let cqe: CompletionQueueEntry =
            unsafe { slot.as_ptr::<CompletionQueueEntry>().read_volatile() };
        if cqe.phase() == self.phase {
            self.cq_head = (self.cq_head + 1) % self.cq_entries;
            if self.cq_head == 0 {
                self.phase = !self.phase;
            }
            unsafe {
                regs.doorbell(self.queue_id, true)
                    .write_volatile(self.cq_head as u32);
            }
            Some(cqe)
        } else {
            None
        }
    }

    /// Blocking poll for a completion — spins until one arrives or
    /// [`TIMEOUT`] elapses.
    pub fn complete_poll(&mut self, regs: &NVMeRegisters) -> Result<CompletionQueueEntry, IOError> {
        let cq_virt = self.completion_queue.virt();
        let deadline = time() + TIMEOUT;
        loop {
            if time() >= deadline {
                return Err(IOError::Timeout);
            }

            let slot = cq_virt + (self.cq_head as u64 % self.cq_entries as u64) * 16;
            let cqueue_entry: CompletionQueueEntry =
                unsafe { slot.as_ptr::<CompletionQueueEntry>().read_volatile() };
            if cqueue_entry.phase() == self.phase {
                self.cq_head = (self.cq_head + 1) % self.cq_entries;
                if self.cq_head == 0 {
                    self.phase = !self.phase;
                }
                unsafe {
                    regs.doorbell(self.queue_id, true)
                        .write_volatile(self.cq_head as u32);
                }
                return Ok(cqueue_entry);
            }
            core::hint::spin_loop();
        }
    }

    pub fn s_queue_phys(&self) -> PhysAddr {
        self.submission_queue.phys()
    }

    pub fn c_queue_phys(&self) -> PhysAddr {
        self.completion_queue.phys()
    }
}

/// 64-byte NVMe submission queue entry.
///
/// Fields follow the NVMe spec: DWord 0 contains the opcode, followed by
/// namespace ID, metadata pointer, PRP list entries, and 6 command-specific
/// DWords (cdw10–cdw15).
#[repr(C, packed)]
#[derive(Default, Clone, Copy)]
pub struct SubmissionQueueEntry {
    pub cdw0: u32,
    pub namespace_id: u32,
    pub cdw2: u32,
    pub cdw3: u32,
    pub mptr: u64,
    pub prp1: u64,
    pub prp2: u64,
    pub cdw10: u32,
    pub cdw11: u32,
    pub cdw12: u32,
    pub cdw13: u32,
    pub cdw14: u32,
    pub cdw15: u32,
}

impl SubmissionQueueEntry {
    /// Create a new command with the given opcode and namespace ID.
    /// All other fields are zeroed.
    pub fn new(opcode: u8, namespace_id: u32) -> Self {
        Self {
            cdw0: opcode as u32,
            namespace_id,
            ..Default::default()
        }
    }
}

/// 16-byte NVMe completion queue entry.
///
/// DWord 3 encodes the phase tag (bit 16), status code (bits 24:17), and
/// status type (bits 27:25).
#[repr(C, packed)]
pub struct CompletionQueueEntry {
    pub dw0: u32,
    pub dw1: u32,
    pub dw2: u32,
    pub dw3: u32,
}

impl CompletionQueueEntry {
    pub fn status_code(&self) -> u8 {
        ((self.dw3 >> 17) & 0xFF) as u8
    }

    pub fn status_type(&self) -> u8 {
        ((self.dw3 >> 25) & 0x7) as u8
    }

    pub fn phase(&self) -> bool {
        ((self.dw3 >> 16) & 1) == 1
    }

    pub fn is_success(&self) -> bool {
        self.status_code() == 0 && self.status_type() == 0
    }
}
