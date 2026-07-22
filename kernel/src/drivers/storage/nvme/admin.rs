//! NVMe admin commands.
//!
//! Provides helpers for the admin commands needed during initialisation:
//! `Identify Namespace` and `Create I/O Completion/Submission Queue`.

use crate::memory::DMABuffer;

use super::{IOError, queues::*, registers::NVMeRegisters};

/// Admin command opcodes used by this driver.
#[repr(u8)]
enum Commands {
    Identify = 0x6,
    CreateIOCompQueue = 0x5,
    CreateIOSubQueue = 0x1,
}

/// Submit a command on the admin queue and busy-wait for completion.
///
/// Returns `Ok(())` only when the completion status is success.
fn exec_admin(admin_queue: &mut QueuePair, cmd: &SubmissionQueueEntry, regs: &NVMeRegisters) -> Result<(), IOError> {
    admin_queue.submit(cmd, regs);

    let cqueue_entry = admin_queue.complete_poll(regs)?;
    if cqueue_entry.is_success() {
        Ok(())
    } else {
        Err(IOError::CommandFailure)
    }
}

/// Issue an Identify Namespace command and return a 4 KiB buffer with the
/// namespace data (struct ns_id or ns_id_ns in NVMe terms).
pub fn identify_namespace(admin_queue: &mut QueuePair, namespace_id: u32, regs: &NVMeRegisters) -> Result<DMABuffer, IOError> {
    let buffer = DMABuffer::new(4096);
    let mut cmd = SubmissionQueueEntry::new(Commands::Identify as u8, namespace_id);
    cmd.prp1 = buffer.phys().as_u64();
    exec_admin(admin_queue, &cmd, regs)?;
    Ok(buffer)
}

/// Create an I/O completion queue and its paired submission queue.
///
/// `queue_id` is the numeric QID (must be ≥ 1).  `iv` is the interrupt vector
/// to use (0 is typical for a single-queue configuration).
pub fn create_io_queues(admin_queue: &mut QueuePair, queue_id: u16, queue_size: u16, iv: u16,
    queues: &QueuePair, regs: &NVMeRegisters) -> Result<(), IOError> {

    let cdw10 = queue_id as u32 | ((queue_size - 1) as u32) << 16;

    let mut cq_cmd = SubmissionQueueEntry::new(Commands::CreateIOCompQueue as u8, 0);
    cq_cmd.prp1 = queues.c_queue_phys().as_u64();
    cq_cmd.cdw10 = cdw10;
    cq_cmd.cdw11 = 3 | (iv as u32) << 16;
    exec_admin(admin_queue, &cq_cmd, regs)?;

    let mut sq_cmd = SubmissionQueueEntry::new(Commands::CreateIOSubQueue as u8, 0);
    sq_cmd.prp1 = queues.s_queue_phys().as_u64();
    sq_cmd.cdw10 = cdw10;
    sq_cmd.cdw11 = 1 | (queue_id as u32) << 16;
    exec_admin(admin_queue, &sq_cmd, regs)?;
    Ok(())
}
