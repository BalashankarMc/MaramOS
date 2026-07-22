//! AHCI / SATA storage driver.
//!
//! Probes AHCI ports via PCI, initialises command lists and FIS buffers,
//! and implements the [`StorageDrive`] trait for read, write, and zero
//! operations. Large transfers are chunked to stay within HBA limits.

use alloc::vec::Vec;
use x86_64::{PhysAddr, VirtAddr};

use super::{IOError, TIMEOUT, StorageDrive};

use crate::{
    drivers::pci::PCIDevice,
    memory::{DMABuffer, KMemory, PAGE_SIZE}
};

use registers::{HBARegisters, PortRegisters};
use commands::{CommandTableHeader, CommandHeader, PhysRegionDescTableEntry};
use fis::FISRegisterH2D;

mod fis;
mod registers;
mod commands;
mod port;

const H2DFIS_SIZE: usize = core::mem::size_of::<FISRegisterH2D>();
const PRD_MAX_BYTES: u32 = 1 << 22;
const MAX_SECTORS_PER_CMD: u64 = 65535;

#[derive(Debug)]
pub struct AHCIDrive {
    hba: VirtAddr,
    port_id: u8,
    command_list: DMABuffer,
    command_tables: DMABuffer,
    _fis_buffer: DMABuffer,
    block_size: u64,
    block_count: u64,
}

impl AHCIDrive {
    fn port(&self) -> PortRegisters {
        let base = (self.hba + 0x100 + self.port_id as u64 * 0x80).as_mut_ptr();
        PortRegisters { base }
    }

    fn execute_command(&mut self, fis: &FISRegisterH2D, data: PhysAddr, data_bytes: u32, is_write: bool) -> Result<(), IOError> {
        let mut port = self.port();
        let slot = match port::find_slot(&port) {
            Some(s) => s,
            None => return Err(IOError::CommandFailure)
        };

        let cmd_header = unsafe { &mut *(self.command_list.virt().as_mut_ptr::<CommandHeader>().add(slot as usize)) };
        unsafe { core::ptr::write_bytes(cmd_header, 0, 1) };

        cmd_header.fis_info.set_cmd_fis_len(5);
        cmd_header.fis_info.set_flow(is_write);

        let ct_phys = self.command_tables.phys().as_u64() + (slot as u64) * 0x100;
        cmd_header.cmd_table_base_addr_lower = ct_phys as u32;
        cmd_header.cmd_table_base_addr_upper = (ct_phys >> 32) as u32;

        let ct_virt = self.command_tables.virt() + (slot as u64) * 0x100;
        let ct = unsafe { &mut *(ct_virt.as_mut_ptr::<CommandTableHeader>()) };
        unsafe { core::ptr::write_bytes(ct, 0, 1); }

        let cmd_fis_ptr = core::ptr::addr_of_mut!(ct.command_fis) as *mut [u8; H2DFIS_SIZE];
        let fis_cmd = fis as *const FISRegisterH2D as *const [u8; H2DFIS_SIZE];
        unsafe {
            let rem_ptr = cmd_fis_ptr.cast::<u8>().add(H2DFIS_SIZE);
            core::ptr::write_volatile(cmd_fis_ptr, *fis_cmd);
            core::ptr::write_bytes(rem_ptr, 0, 64 - H2DFIS_SIZE);
        }

        let num_prds = if data_bytes > 0 { data_bytes.div_ceil(PRD_MAX_BYTES) } else { 0 };
        cmd_header.prd_table_len = num_prds as u16;

        let mut remaining = data_bytes as u64;
        let mut phys_offset = data.as_u64();
        for i in 0..num_prds {
            let chunk = remaining.min(PRD_MAX_BYTES as u64);
            let prd = unsafe {
                &mut *((ct_virt + 0x80 + i as u64 * 16).as_mut_ptr::<PhysRegionDescTableEntry>())
            };
            prd.data_base_addr_low = phys_offset as u32;
            prd.data_base_addr_high = (phys_offset >> 32) as u32;
            let mut dbc = unsafe { core::ptr::read(core::ptr::addr_of!(prd.dbc)) };
            dbc.set_descriptor_byte_count(chunk as u32);
            prd.dbc = dbc;
            remaining -= chunk;
            phys_offset += chunk;
        }

        port.set_cmd_issue(1 << slot);

        let deadline = crate::acpi::passed_nanos() + TIMEOUT;
        #[allow(clippy::while_immutable_condition)]
        while (port.cmd_issue() >> slot) & 1 == 1 {
            if crate::acpi::passed_nanos() >= deadline { return Err(IOError::Timeout); }
            core::hint::spin_loop();
        }

        if port.interrupt_status() & (1 << 30) != 0 {
            port.set_interrupt_status(1 << 30);
            return Err(IOError::CommandFailure);
        }

        Ok(())
    }
}

pub fn init(device: &'static dyn PCIDevice) -> Result<Vec<AHCIDrive>,  IOError> {
    device.enable_bus_master();
    device.enable_mmio();

    let bar = device.bar(5).ok_or(IOError::InitFailed)?;

    let addr = KMemory::map_mmio(PhysAddr::new(bar.address), bar.size.div_ceil(PAGE_SIZE as u64) as usize);

    let mut hba_regs = HBARegisters::from(addr);
    let mut global_regs = hba_regs.get_global_registers();

    global_regs.set_global_host_control(global_regs.global_host_control() | (1 << 31));

    let pi = global_regs.port_implemented();

    let mut drives = Vec::new();
    for port_num in 0..32 {
        if (pi >> port_num) & 1 != 1 { continue }
        let mut port = hba_regs.get_port_registers(port_num).ok_or(IOError::InitFailed)?;
        
        let sig = match port::probe(&port) {
            Some(s) => s,
            None => continue,
        };

        if sig != 0x101 && sig != 0 {
            continue;
        }

        let cmd_list = DMABuffer::new(1024);
        let fis_buf = DMABuffer::new(256);
        let cmd_tables = DMABuffer::new(32 * 256);
        port::rebase(&mut port, cmd_list.phys(), fis_buf.phys())?;

        let mut drive = AHCIDrive {
            hba: addr,
            port_id: port_num,
            command_list: cmd_list,
            command_tables: cmd_tables,
            _fis_buffer: fis_buf,
            block_size: 512,
            block_count: 0,
        };

        let id_buf = DMABuffer::new(512);
        let fis = FISRegisterH2D::new(0x80, 0xEC, 0, 0, 0, 0, 0);

        drive.execute_command(&fis, id_buf.phys(), 512, false).map_err(|_| IOError::InitFailed)?;
        drive.block_count = unsafe { (id_buf.virt() + 200).as_ptr::<u64>().read_volatile() };

        drives.push(drive);
    }

    Ok(drives)

}

impl StorageDrive for AHCIDrive {
    fn capacity(&self) -> u64 {
        self.block_count * self.block_size
    }

    fn read_lbas(&mut self, lba: u64, count: u64, dest: &mut crate::memory::PhysPage) -> Result<(), IOError> {
        if self.block_count < lba + count { return Err(IOError::OutOfBoundsAccess) }

        let mut offset = 0;
        while offset < count {
            let chunks = (count - offset).min(MAX_SECTORS_PER_CMD);
            let fis = FISRegisterH2D::new(0x80, 0x25, 0, lba + offset, chunks as u16, 0, 0);
            let bytes = (chunks * 0x200) as u32;
            let addr = dest.get_phys_address() + offset * 0x200;
            self.execute_command(&fis, addr, bytes, false)?;
            offset += chunks;
        }
        Ok(())
    }

    fn write_lbas(&mut self, lba: u64, count: u64, src: &crate::memory::PhysPage) -> Result<(), IOError> {
        if self.block_count < lba + count { return Err(IOError::OutOfBoundsAccess) }

        let mut offset = 0;
        while offset < count {
            let chunks = (count - offset).min(MAX_SECTORS_PER_CMD);
            let fis = FISRegisterH2D::new(0x80, 0x35, 0, lba + offset, chunks as u16, 0, 0);
            let bytes = (chunks * 0x200) as u32;
            let addr = src.get_phys_address() + offset * 0x200;
            self.execute_command(&fis, addr, bytes, true)?;
            offset += chunks;
        }
        Ok(())
    }

    fn zero_lbas(&mut self, lba: u64, count: u64) -> Result<(), IOError> {
        if self.block_count < lba + count { return Err(IOError::OutOfBoundsAccess) }

        let mut offset = 0;
        while offset < count {
            let chunks = (count - offset).min(MAX_SECTORS_PER_CMD);
            let size = (chunks as usize * 512).div_ceil(PAGE_SIZE) * PAGE_SIZE;
            let zero_buf = DMABuffer::new(size);
            let fis = FISRegisterH2D::new(0x80, 0x35, 0, lba + offset, chunks as u16, 0, 0);
            let bytes = (chunks * 512) as u32;
            self.execute_command(&fis, zero_buf.phys(), bytes, true)?;
            offset += chunks;
        }
        Ok(())
    }
}
