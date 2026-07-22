//! AHCI HBA register definitions.
//!
//! MMIO wrappers for global registers (GHC, PI) and per-port registers
//! (CLB, FIS, IS, CMD, SIG, SSTS, SERR, SACT, CI) with volatile access.

use x86_64::VirtAddr;
use core::ptr::{read_volatile, write_volatile};

const GC_GHC: usize = 1;
const GC_PI: usize = 0x0C / 4;

const P_CLB: usize = 0;
const P_CLBU: usize = 1;
const P_FB: usize = 0x08 / 4;
const P_FBU: usize = 0x0C / 4;
const P_IS: usize = 0x10 / 4;
const P_IE: usize = 0x14 / 4;
const P_CMD: usize = 0x18 / 4;
const P_SIG: usize = 0x24 / 4;
const P_SSTS: usize = 0x28 / 4;
const P_SERR: usize = 0x30 / 4;
const P_SACT: usize = 0x34 / 4;
const P_CI: usize = 0x38 / 4;

pub struct GlobalRegisters {
    pub(crate) base: *mut u32,
}

impl GlobalRegisters {
    fn read(&self, reg: usize) -> u32 {
        unsafe { read_volatile(self.base.add(reg)) }
    }

    fn write(&mut self, reg: usize, val: u32) {
        unsafe { write_volatile(self.base.add(reg), val) }
    }

    pub fn global_host_control(&self) -> u32 { self.read(GC_GHC) }
    pub fn set_global_host_control(&mut self, val: u32) { self.write(GC_GHC, val); }
    pub fn port_implemented(&self) -> u32 { self.read(GC_PI) }
}

pub struct PortRegisters {
    pub(crate) base: *mut u32,
}

impl PortRegisters {
    fn read(&self, reg: usize) -> u32 {
        unsafe { read_volatile(self.base.add(reg)) }
    }

    fn write(&mut self, reg: usize, val: u32) {
        unsafe { write_volatile(self.base.add(reg), val) }
    }

    pub fn set_cmd_list_base_low(&mut self, val: u32) { self.write(P_CLB, val); }
    pub fn set_cmd_list_base_high(&mut self, val: u32) { self.write(P_CLBU, val); }
    pub fn set_fis_base_low(&mut self, val: u32) { self.write(P_FB, val); }
    pub fn set_fis_base_high(&mut self, val: u32) { self.write(P_FBU, val); }
    pub fn interrupt_status(&self) -> u32 { self.read(P_IS) }
    pub fn set_interrupt_status(&mut self, val: u32) { self.write(P_IS, val); }
    pub fn cmd(&self) -> u32 { self.read(P_CMD) }
    pub fn set_cmd(&mut self, val: u32) { self.write(P_CMD, val); }
    pub fn signature(&self) -> u32 { self.read(P_SIG) }
    pub fn sata_status(&self) -> u32 { self.read(P_SSTS) }
    pub fn set_sata_err(&mut self, val: u32) { self.write(P_SERR, val); }
    pub fn sata_err(&self) -> u32 { self.read(P_SERR) }
    pub fn sata_active(&self) -> u32 { self.read(P_SACT) }
    pub fn cmd_issue(&self) -> u32 { self.read(P_CI) }
    pub fn set_cmd_issue(&mut self, val: u32) { self.write(P_CI, val); }
    pub fn interrupts_enabled(&self) -> bool { self.read(P_IE) != 0 }
    pub fn set_interrupts_enabled(&mut self, val: u32) { self.write(P_IE, val);  }
    
}

pub struct HBARegisters {
    hba_addr: VirtAddr,
}

impl HBARegisters {
    pub fn from(addr: VirtAddr) -> Self {
        Self { hba_addr: addr }
    }

    pub fn get_global_registers(&mut self) -> GlobalRegisters {
        GlobalRegisters { base: self.hba_addr.as_mut_ptr() }
    }

    pub fn get_port_registers(&mut self, port: u8) -> Option<PortRegisters> {
        if port >= 32 { return None }
        let pi = unsafe { read_volatile((self.hba_addr + 0x0C).as_ptr::<u32>()) };
        if (pi >> port) & 1 == 0 { return None }
        let port_base = self.hba_addr + 0x100 + port as u64 * 0x80;
        Some(PortRegisters { base: port_base.as_mut_ptr() })
    }
}
