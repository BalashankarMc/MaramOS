//! NVMe I/O commands (read, write, write-zeroes).
//!
//! Each function builds a [`SubmissionQueueEntry`], computes the PRP chain
//! (up to a single PRP list for large transfers), submits it via the I/O
//! queue pair, and waits for completion (interrupt-driven with a polling
//! fallback on timeout).
//!
//! # Limitations
//!
//! * PRP list size is capped at 512 entries, limiting a single I/O to
//!   ~513 × 4 KiB = ~2 MiB (≈ 4096 LBAs at 512 B).  Larger transfers
//!   silently truncate the PRP list.
//! * LBA size is hard-coded to 512 bytes in `lbas_to_pages`.

use super::{
    IOError, NVMeDrive, TIMEOUT,
    queues::{QueuePair, SubmissionQueueEntry},
    registers::NVMeRegisters,
};

use crate::{
    acpi::passed_nanos as time,
    memory::{DMABuffer, PAGE_SIZE, PhysPage},
};

use core::sync::atomic::Ordering;

use super::COMPLETION_FLAG;

/// I/O command opcodes.
#[repr(u8)]
enum Commands {
    Read = 0x2,
    Write = 0x1,
    WriteZero = 0x8,
}

/// Build the PRP2 (second PRP / PRP list pointer) for a transfer.
///
///   * 1 page → PRP2 = 0 (single PRP, no list).
///   * 2 pages → PRP2 = PRP1 + PAGE_SIZE (contiguous physical pages).
///   * 3+ pages → PRP2 points to a physical page containing a PRP list
///     of `pages - 1` entries.
fn calc_prp2(prp1: u64, blocks: u64, prp_list: &mut DMABuffer) -> u64 {
    let pages = super::super::lbas_to_pages(blocks);

    match pages {
        0 => unreachable!(),
        1 => 0,
        2 => prp1 + PAGE_SIZE as u64,
        _ => {
            let entries = pages - 1;
            let list_virt = prp_list.virt().as_mut_ptr::<u64>();
            for i in 0..entries.min(512) as u64 {
                unsafe {
                    list_virt
                        .add(i as usize)
                        .write_volatile(prp1 + (i + 1) * PAGE_SIZE as u64);
                }
            }
            prp_list.phys().as_u64()
        }
    }
}

/// Submit `cmd` on `queue` and wait for completion.
///
/// Normally returns when the interrupt handler sets `completion_pending`.
/// If that doesn't happen within [`TIMEOUT`], falls back to polling the CQ
/// directly to avoid losing completions.
fn exec_io(queue: &mut QueuePair, cmd: &SubmissionQueueEntry, regs: &NVMeRegisters) -> Result<(), IOError> {
    use x86_64::instructions::interrupts;

    interrupts::disable();
    COMPLETION_FLAG.store(false, Ordering::Release);
    queue.submit(cmd, regs);
    interrupts::enable();

    let deadline = time() + TIMEOUT;
    while !COMPLETION_FLAG.load(Ordering::Acquire) {
        if time() >= deadline {
            let cqe = queue.complete_poll(regs)?;
            COMPLETION_FLAG.store(false, Ordering::Release);
            return if cqe.is_success() {
                Ok(())
            } else {
                Err(IOError::CommandFailure)
            };
        }
        core::hint::spin_loop();
    }

    COMPLETION_FLAG.store(false, Ordering::Release);
    match queue.try_complete(regs) {
        Some(cqe) if cqe.is_success() => Ok(()),
        _ => Err(IOError::CommandFailure),
    }
}

/// Read `blocks` LBAs starting at `lba` into the physical page `dest`.
pub fn read(drive: &mut NVMeDrive, lba: u64, blocks: u64, dest: &mut PhysPage) -> Result<(), IOError> {
    if blocks == 0 {
        return Ok(());
    }
    let mut cmd = SubmissionQueueEntry::new(Commands::Read as u8, drive.namespace_id);
    cmd.cdw10 = lba as u32;
    cmd.cdw11 = (lba >> 32) as u32;
    cmd.cdw12 = (blocks - 1) as u32;
    cmd.prp1 = dest.get_phys_address().as_u64();
    cmd.prp2 = calc_prp2(cmd.prp1, blocks, &mut drive.prp2_buffer);

    exec_io(&mut drive.io_queue, &cmd, &drive.registers)
}

/// Write `blocks` LBAs starting at `lba` from the physical page `src`.
pub fn write(drive: &mut NVMeDrive, lba: u64, blocks: u64, src: &PhysPage) -> Result<(), IOError> {
    if blocks == 0 {
        return Ok(());
    }
    let mut cmd = SubmissionQueueEntry::new(Commands::Write as u8, drive.namespace_id);
    cmd.cdw10 = lba as u32;
    cmd.cdw11 = (lba >> 32) as u32;
    cmd.cdw12 = (blocks - 1) as u32;
    cmd.prp1 = src.get_phys_address().as_u64();
    cmd.prp2 = calc_prp2(cmd.prp1, blocks, &mut drive.prp2_buffer);

    exec_io(&mut drive.io_queue, &cmd, &drive.registers)
}

/// Deallocate and zero `blocks` LBAs starting at `lba` (NVMe Write Zeroes).
pub fn write_zeroes(drive: &mut NVMeDrive, lba: u64, blocks: u64) -> Result<(), IOError> {
    if blocks == 0 {
        return Ok(());
    }
    let mut cmd = SubmissionQueueEntry::new(Commands::WriteZero as u8, drive.namespace_id);
    cmd.cdw10 = lba as u32;
    cmd.cdw11 = (lba >> 32) as u32;
    cmd.cdw12 = (blocks - 1) as u32;

    exec_io(&mut drive.io_queue, &cmd, &drive.registers)
}
