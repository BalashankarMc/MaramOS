//! NVMe MMIO register block.
//!
//! Wraps the BAR0 MMIO region of an NVMe controller and provides typed
//! access to the standard NVMe registers (CAP, VS, CC, CSTS, AQA, ASQ, ACQ,
//! and doorbells).

use x86_64::VirtAddr;

use crate::memory::PAGE_SIZE;
// Register offsets within the BAR0 MMIO region.
const CAP_OFFSET: u64 = 0; // Controller capabilities.
const VS_OFFSET: u64 = 0x8; // Version.
const CC_OFFSET: u64 = 0x14; // Controller configuration.
const CSTS_OFFSET: u64 = 0x1C; // Controller status.
const AQA_OFFSET: u64 = 0x24; // Admin queue attributes.
const ASQ_OFFSET: u64 = 0x28; // Admin submission queue base address.
const ACQ_OFFSET: u64 = 0x30; // Admin completion queue base address.

/// Typed access to the NVMe register file.
#[derive(Debug)]
pub struct NVMeRegisters(VirtAddr);

impl NVMeRegisters {
    pub fn new(addr: VirtAddr) -> Self {
        Self(addr)
    }

    /// Volatile read of a value at `offset`.
    fn read<T: Sized>(&self, offset: u64) -> T {
        unsafe { (self.0 + offset).as_ptr::<T>().read_volatile() }
    }

    /// Volatile write of `val` at `offset`.
    fn write<T: Sized>(&mut self, offset: u64, val: T) {
        unsafe { (self.0 + offset).as_mut_ptr::<T>().write_volatile(val) }
    }

    pub fn cap(&self) -> u64 { self.read(CAP_OFFSET) }

    pub fn vs(&self) -> u32 { self.read(VS_OFFSET) }

    pub fn csts(&self) -> u32 { self.read(CSTS_OFFSET) }

    pub fn write_cc(&mut self, val: u32) { self.write(CC_OFFSET, val) }

    pub fn write_aqa(&mut self, val: u32) { self.write(AQA_OFFSET, val) }

    pub fn write_asq(&mut self, val: u64) { self.write(ASQ_OFFSET, val) }

    pub fn write_acq(&mut self, val: u64) { self.write(ACQ_OFFSET, val) }

    /// Return a mutable pointer to the doorbell register for `queue_id`.
    ///
    /// Set `is_completion` for the completion queue doorbell, or `false`
    /// for the submission queue doorbell.
    ///
    /// Doorbell stride (DSTRD) is read from CAP bits 35:32.
    pub fn doorbell(&self, queue_id: u16, is_completion: bool) -> *mut u32 {
        let stride = 2_u64.pow(2 + self.cap_doorbell_stride() as u32);
        let idx = (2 * queue_id + is_completion as u16) as u64;
        (self.0 + PAGE_SIZE as u64 + idx * stride).as_mut_ptr::<u32>()
    }

    pub fn cap_timeout(&self) -> u8 {
        (self.cap() >> 24) as u8
    }

    pub fn cap_doorbell_stride(&self) -> u8 {
        (self.cap() >> 32) as u8 & 0xF
    }

    pub fn csts_ready(&self) -> bool {
        (self.csts() & 1) == 1
    }

    pub fn version_major(&self) -> u16 {
        (self.vs() >> 16) as u16
    }
}
