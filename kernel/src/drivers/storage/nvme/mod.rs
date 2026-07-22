//! NVMe (NVM Express) driver.
//!
//! Implements the [`StorageDrive`] trait for NVMe controllers discovered via
//! the PCI subsystem.  Initialisation creates an admin queue pair and one I/O
//! queue pair, identifies the first namespace, and exposes block-level read /
//! write / zero commands.
//!
//! # Interrupt-driven I/O
//!
//! I/O completion is signalled via [`COMPLETION_FLAG`], a module-level static
//! `AtomicBool` set by the NVMe interrupt handler and consumed by the I/O
//! functions.  Only one I/O transaction may be in flight at a time; this is
//! acceptable for a single-controller hobby OS.

use core::sync::atomic::AtomicBool;
use x86_64::{PhysAddr, structures::idt::InterruptStackFrame};

use super::{
    super::pci::{PCIDevice, PCIFunction},
    TIMEOUT,
    StorageDrive,
    IOError
};


use crate::{
    acpi::passed_nanos as time, descriptors::interrupts::{HardwareInterrupts, add_idt_entry}, memory::{DMABuffer, KMemory, PAGE_SIZE, PhysPage},
};

/// Completion flag set by the NVMe I/O completion interrupt handler.
pub static COMPLETION_FLAG: AtomicBool = AtomicBool::new(false);

mod admin;
mod io;
mod queues;
mod registers;

use queues::QueuePair;
use registers::NVMeRegisters;

/// An initialised NVMe controller and its first namespace.
///
/// Holds the MMIO register block, admin and I/O queue pairs, a PRP-list
/// scratch buffer, and the geometry of namespace 1.
#[allow(dead_code)] // Keep Admin Queue
#[derive(Debug)]
pub struct NVMeDrive {
    registers: NVMeRegisters,
    admin_queue: QueuePair,
    io_queue: QueuePair,
    prp2_buffer: DMABuffer,
    namespace_id: u32,
    block_size: u32,
    block_count: u64,
}

impl NVMeDrive {
    /// Probe and initialise the NVMe controller at `pci`.
    ///
    /// The init sequence is:
    ///   1. Enable MMIO and bus mastering on the PCI function.
    ///   2. Map the BAR0 MMIO region and check the NVMe version.
    ///   3. Program MSI-X entry 0 for I/O completion interrupts.
    ///   4. Disable the controller (write CC.EN = 0) and wait for CSTS.RDY = 0.
    ///   5. Set up the admin submission/completion queue pair (64 entries).
    ///   6. Enable the controller and wait for CSTS.RDY = 1.
    ///   7. Issue an Identify Namespace command to get block count and LBA format.
    ///   8. Create the I/O queue pair (64 entries).
    ///   9. Register the I/O queue's completion flag in [`IO_COMPLETION_TARGET`].
    pub fn new(pci: &'static PCIFunction) -> Result<Self, IOError> {
        pci.enable_bus_master();
        pci.enable_mmio();

        let bar = match pci.bar(0) {
            Some(b) => b,
            None => return Err(IOError::InitFailed),
        };

        let regs_virt = KMemory::map_mmio(PhysAddr::new(bar.address),(bar.size as usize).div_ceil(PAGE_SIZE));

        let mut registers = NVMeRegisters::new(regs_virt);

        if registers.version_major() < 1 { return Err(IOError::IncompatibleVersion) }

        if !super::super::pci::msix::program(pci, 0, HardwareInterrupts::NVMeIO) { return Err(IOError::InitFailed) }

        let _ = add_idt_entry(nvme_io_completion, HardwareInterrupts::NVMeIO.as_u8());

        // Step 4: quiesce the controller.
        registers.write_cc(0);
        let deadline = time() + TIMEOUT;
        while registers.csts_ready() {
            if time() >= deadline {
                return Err(IOError::Timeout);
            }
            core::hint::spin_loop();
        }

        // Step 5: program admin queues (QID 0, 64 entries each).
        let mut admin_queue = QueuePair::new(0, 64);
        let asq_phys = admin_queue.s_queue_phys().as_u64();
        let acq_phys = admin_queue.c_queue_phys().as_u64();
        registers.write_asq(asq_phys);
        registers.write_acq(acq_phys);
        registers.write_aqa(63 | (63 << 16));

        // Step 6: enable controller with 4-byte MPS, NVM command set.
        registers.write_cc(1 | (6 << 16) | (4 << 20));
        let timeout = registers.cap_timeout() as u64 * 500_000_000;
        let deadline = time() + timeout;
        while !registers.csts_ready() {
            if time() >= deadline { return Err(IOError::Timeout) }
            core::hint::spin_loop();
        }

        // Step 7: identify namespace 1.
        let namespace_buffer = admin::identify_namespace(&mut admin_queue, 1, &registers)?;
        let namespace_virt = namespace_buffer.virt();

        let block_count = unsafe { namespace_virt.as_ptr::<u64>().read_volatile() };
        let formatted_lba_size = unsafe { namespace_virt.as_ptr::<u8>().add(26).read_volatile() };
        let format_idx = (formatted_lba_size & 0xF) as usize;
        let lbaf_offset = 128 + format_idx * 4;
        let format_descriptor = unsafe {
            namespace_virt
                .as_ptr::<u32>()
                .add(lbaf_offset / 4)
                .read_volatile()
        };

        let lbads = (format_descriptor >> 16) & 0xFF;
        let block_size: u32 = 1 << lbads;

        // Step 8: create I/O queue pair (QID 1, 64 entries).
        let io_queue = QueuePair::new(1, 64);
        admin::create_io_queues(&mut admin_queue, 1, 64, 0, &io_queue, &registers)?;

        Ok(Self {
            registers,
            admin_queue,
            io_queue,
            prp2_buffer: DMABuffer::new(4096),
            namespace_id: 1,
            block_size,
            block_count,
        })
    }
}

impl StorageDrive for NVMeDrive {
    fn capacity(&self) -> u64 {
        self.block_count * self.block_size as u64
    }

    fn zero_lbas(&mut self, lba: u64, count: u64) -> Result<(), IOError> {
        if self.block_count < lba + count { return Err(IOError::OutOfBoundsAccess) }
        io::write_zeroes(self, lba, count)
    }

    fn read_lbas(&mut self, lba: u64, count: u64, dest: &mut PhysPage) -> Result<(), IOError> {
        if self.block_count < lba + count { return Err(IOError::OutOfBoundsAccess) }
        io::read(self, lba, count, dest)
    }

    fn write_lbas(&mut self, lba: u64, count: u64, src: &PhysPage) -> Result<(), IOError> {
        if self.block_count < lba + count { return Err(IOError::OutOfBoundsAccess) }
        io::write(self, lba, count, src)
    }
}

extern "x86-interrupt" fn nvme_io_completion(_stack_frame: InterruptStackFrame) {
    COMPLETION_FLAG.store(true, core::sync::atomic::Ordering::Release);
    crate::acpi::lapic_eoi();
}
