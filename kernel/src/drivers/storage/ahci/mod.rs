//! Implements the SATA AHCI driver for `MaramOS`
//! DOES NOT SUPPORT PORT MULTIPLICATION

use core::ops::Add;

use alloc::{sync::Arc, vec::Vec};
use spin::Mutex;
use x86_64::PhysAddr;

use crate::{drivers::pci::PCIFunction, helpers::{Time, wait_timeout}, log_success, log_warn, memory::{MMIORegion, PAGE_SIZE, PhysPage}};
use super::{StorageDrive, StorageError, TIMEOUT, BLOCK_SIZE};

mod fis;
mod port;
mod commands;
mod registers;

use registers::{GlobalRegisters, PortRegisters};
use commands::{CommandHeader, CommandTableHeader, PhysRegionDescTableEntry};
use fis::FISRegisterH2D;

const PROBE_TIMEOUT: Time = Time::Milliseconds(500);
const PRD_MAX_BYTES: usize = 1 << 21;
const PRDS_PER_TABLE: usize = 16;
const PRD_STRIDE: u64 = 0x80 + PRDS_PER_TABLE as u64 * 16;
const MAX_SECTORS_PER_CMD: u64 = u16::MAX as u64;
const FATAL_PXIS_MASK: u32 = 0xF << 27;

#[derive(Debug)]
pub struct MappedPCI(PCIFunction, MMIORegion);

#[derive(Debug)]
pub struct AHCIDrive {
    mapped_device: Arc<MappedPCI>,
    port_id: u32,
    command_list: PhysPage,
    command_tables: PhysPage,
    _fis_buffer: PhysPage,
    block_size: u64,
    block_count: u64,
    ncs: u8,
    lock: Mutex<()>
}

impl AHCIDrive {
    fn send_command(&self, fis: &FISRegisterH2D, data: PhysAddr, data_bytes: usize, is_write: bool) -> Result<(), StorageError> {
        use StorageError::CommandFailed;
        
        // Get port specific registers
        let port = PortRegisters::new(&self.mapped_device.1, self.port_id)
            .ok_or(CommandFailed)?;

        // Find free slot
        let slot = port::find_slot(&port, self.ncs).ok_or(CommandFailed)?;

        // Initialize command header
        let mut cmd_header = CommandHeader::zeroed();
        cmd_header.fis_info.set(5, is_write);
        
        let ct_phys = self.command_tables.phys().as_u64() + u64::from(slot) * PRD_STRIDE;
        cmd_header.cmd_table_base_addr_lower = ct_phys as u32;
        cmd_header.cmd_table_base_addr_upper = (ct_phys >> 32) as u32;

        // Zero command table header
        let offset = usize::from(slot) * PRD_STRIDE as usize;
        self.command_tables.write_data(offset, CommandTableHeader::zeroed());

        // Write FIS
        let ct = unsafe {
            self.command_tables.address().add(offset as u64).as_mut_ptr::<CommandTableHeader>().as_mut().ok_or(CommandFailed)?
        };

        ct.command_fis[0..size_of::<FISRegisterH2D>()].copy_from_slice(fis.to_bytes());
        ct.command_fis[size_of::<FISRegisterH2D>()..].fill(0);

        // Set PRDs
        let num_prds = data_bytes.div_ceil(PRD_MAX_BYTES);
        cmd_header.prd_table_len = num_prds as u16;

        let mut rem = data_bytes as u64;
        let mut phys_offset = data.as_u64();

        for i in 0..num_prds {
            let chunk = rem.min(PRD_MAX_BYTES as u64);
            let prd = unsafe {
                self.command_tables.address().add(u64::from(slot) * PRD_STRIDE + 0x80 + i as u64 * 16)
                    .as_mut_ptr::<PhysRegionDescTableEntry>()
                    .as_mut().ok_or(CommandFailed)?
            };

            prd.data_base_addr = phys_offset;
            
            let mut dbc = unsafe { (&raw const prd.dbc).read_unaligned() };
            dbc.set_descriptor_byte_count(chunk as u32);
            prd.dbc = dbc;

            rem -= chunk;
            phys_offset += chunk;
        }

        // Write command header
        let offset = size_of::<CommandHeader>() * usize::from(slot);
        if !self.command_list.write_data(offset, cmd_header) { return Err(CommandFailed) }

        // Clear IS
        port.is.write(u32::MAX);

        // Wait for BSY / DRQ to clear
        wait_timeout(|| port.tfd.read() & ((1 << 7) | (1 << 3)) != 0, &TIMEOUT).ok_or(StorageError::CommandFailed)?;

        // Write CI (Command Issued) flag
        port.ci.write(1 << slot);

        wait_timeout(|| (port.ci.read() >> slot) & 1 == 1, &TIMEOUT).ok_or(StorageError::Timeout)?;

        let is = port.is.read();
        port.is.write(u32::MAX);
        if is & FATAL_PXIS_MASK != 0 { return Err(CommandFailed) }

        if port.tfd.read() & 1 != 0 { return Err(CommandFailed) }

        Ok(())
    }
}


pub fn init(device: PCIFunction) -> Result<Vec<AHCIDrive>, StorageError> {
    use StorageError::{InitFailed, PCIeError};
    
    // Initialize the PCI device
    device.enable_bus_master().ok_or(PCIeError)?;
    device.enable_mmio().ok_or(PCIeError)?;

    // Map BAR 5 (ABAR)
    let abar = device.bar(5).ok_or(PCIeError)?;
    let address = PhysAddr::new(abar.address);
    let page_count = (abar.size as usize).div_ceil(PAGE_SIZE);
    let bar = MMIORegion::new(address, page_count).ok_or(InitFailed)?;

    let global_registers = GlobalRegisters::new(&bar).ok_or(InitFailed)?;

    // Read capabilities
    let cap = global_registers.cap.read();
    let ports_count = (cap & 0x1F) + 1;
    let ncs = ((cap >> 8) & 0x1F) as u8;

    if (cap >> 31) & 1 != 1 { return Err(InitFailed) } // No 64-bit addressing :(

    if global_registers.cap2.read() & 2 == 2 { // Handoff supported
        let bohc = global_registers.bohc.read();
        global_registers.bohc.write(bohc | 2); // Request Ownership

        // Wait for ownership / timeout
        wait_timeout(|| !global_registers.bohc.read().is_multiple_of(2), &TIMEOUT).ok_or(StorageError::Timeout)?;
    }

    // Enable AHCI Mode
    let ghc = global_registers.ghc.read();
    global_registers.ghc.write(ghc | (1 << 31));

    // Controller reset
    let ghc = global_registers.ghc.read();
    global_registers.ghc.write(ghc | 1);
    wait_timeout(|| global_registers.ghc.read() & 1 == 1, &TIMEOUT).ok_or(StorageError::Timeout)?;

    // Enable AHCI Mode again
    let ghc = global_registers.ghc.read();
    global_registers.ghc.write(ghc | (1 << 31));

    // Get the PI(Ports Implemented). We do this here to prevent borrow checking problems with `mapped`
    let pi = global_registers.pi.read();    

    // Store the mapped PCI Device and get a new reference to its mapped ABAR
    let mapped = Arc::new(MappedPCI(device, bar));
    let bar = &mapped.1;

    let mut drives = Vec::new(); // Return value containing the initialized drives

    // Probe
    for port_id in 0..ports_count {
        if (pi >> port_id) & 1 == 0 { continue } // Port not implemented

        let Some(port_registers) = PortRegisters::new(bar, port_id) else {
            log_warn!("Failed to initialize port registers on port {port_id}!");
            continue
        };

        let det = port_registers.ssts.read() & 0xF;
        if det != 1 && det != 3 { continue } // No device on port

        // Write SUD (Boot drive if it uses staggered spin-up) and POD
        let cmd = port_registers.cmd.read();
        port_registers.cmd.write(cmd | 6);
        if !wait_timeout(|| port_registers.ssts.read() & 0xF != 3, &PROBE_TIMEOUT) { // Wait for drive boot
            log_warn!("AHCI: Device on port {port_id} did not boot in time!");
            continue
        }

        let ssts = port_registers.ssts.read();
        let ipm = (ssts >> 8) & 0xF;

        if ipm == 0 { // No device (IPM is 0)
            log_warn!("AHCI: No device on port {port_id}");
            continue
        }

        if ipm != 1 && !port::com_reset(&port_registers) { // Power up the device (COMRESET)
            log_warn!("AHCI: Device on port {port_id} did not COMRESET");
            continue
        }

        port_registers.serr.write(u32::MAX); // Clear SERR

        // Allocate memory for DMA Buffers
        let command_list = PhysPage::new(1).ok_or(InitFailed)?;
        let fis_buffer = PhysPage::new(1).ok_or(InitFailed)?;
        let command_tables = PhysPage::new(3).ok_or(InitFailed)?;

        // Update port with correct buffers
        port::rebase(&port_registers, command_list.phys(), fis_buffer.phys())?;

        if !wait_timeout(|| port_registers.cmd.read() & 0x4000 == 0, &TIMEOUT) {
            log_warn!("Device at port {port_id} did not reboot in time!");
            continue
        }

        let sig = port_registers.sig.read();
        if sig != 0x101 { // Not a SATA Drive
            log_warn!("AHCI: Device on port {port_id} is not a SATA Drive. Signature: 0x{sig:02X}");
            continue
        }

        // Drive struct missing `block_size` and `block_count` (Need to issue IDENTIFY)
        let mut drive = AHCIDrive {
            mapped_device: mapped.clone(),
            port_id,
            command_list,
            command_tables,
            _fis_buffer: fis_buffer,
            block_size: 0,
            block_count: 0,
            ncs,
            lock: Mutex::new(())
        };

        let id_buf = PhysPage::new(1).ok_or(InitFailed)?;
        let fis = FISRegisterH2D::new(0x80, 0xEC, 0, 0, 0, 0, 0);

        drive.send_command(&fis, id_buf.phys(), 512, false).map_err(|_| InitFailed)?;
        drive.block_count = id_buf.read_data::<u64>(200).ok_or(InitFailed)?;

        let word106 = id_buf.read_data::<u16>(212).ok_or(InitFailed)?;
        drive.block_size = if (word106 >> 12) & 1 == 1 {
            let word117 = id_buf.read_data::<u16>(234).ok_or(InitFailed)?;
            let word118 = id_buf.read_data::<u16>(236).ok_or(InitFailed)?;            

            let half = (u32::from(word118) << 16) | u32::from(word117);
            u64::from(half * 2).max(BLOCK_SIZE)
        } else { BLOCK_SIZE };

        drives.push(drive);
        log_success!("Initialized drive on port {port_id}");
    }
    Ok(drives)
}

impl StorageDrive for AHCIDrive {
    fn block_count(&self) -> u64 {
        (self.block_count * self.block_size) / BLOCK_SIZE
    }

    fn read_blocks(&self, start_block: u64, count: u64, dest: &mut PhysPage) -> Result<(), StorageError> {
        let _guard = self.lock.lock();
        let start_native = (start_block * BLOCK_SIZE) / self.block_size;
        let count_native = (count * BLOCK_SIZE).div_ceil(self.block_size);
        let max_native = (PRDS_PER_TABLE * PRD_MAX_BYTES) as u64 / self.block_size;

        if start_native + count_native > self.block_count { return Err(StorageError::BlockOutOfBounds) }

        if dest.count * PAGE_SIZE < (count_native * self.block_size) as usize { return Err(StorageError::CommandFailed) }

        let mut offset = 0;
        while offset < count_native {
            let chunks = (count_native - offset).min(MAX_SECTORS_PER_CMD).min(max_native);
            let fis = FISRegisterH2D::new(
                0x80,
                0x25,
                0,
                start_native + offset,
                chunks as u16,
                0,
                0
            );

            let bytes = (chunks * self.block_size) as usize;
            let addr = dest.phys() + offset * self.block_size;

            self.send_command(&fis, addr, bytes, false)?;
            offset += chunks;
        }

        Ok(())
    }

    fn write_blocks(&self, start_block: u64, count: u64, src: &PhysPage) -> Result<(), StorageError> {
        let _guard = self.lock.lock();
        let start_native = (start_block * BLOCK_SIZE) / self.block_size;
        let count_native = (count * BLOCK_SIZE).div_ceil(self.block_size);
        let max_native = (PRDS_PER_TABLE * PRD_MAX_BYTES) as u64 / self.block_size;

        if start_native + count_native > self.block_count { return Err(StorageError::BlockOutOfBounds) }

        if src.count * PAGE_SIZE < (count_native * self.block_size) as usize { return Err(StorageError::CommandFailed) }

        let mut offset = 0;
        while offset < count_native {
            let chunks = (count_native - offset).min(MAX_SECTORS_PER_CMD).min(max_native);
            let fis = FISRegisterH2D::new(
                0x80,
                0x35,
                0,
                start_native + offset,
                chunks as u16,
                0,
                0
            );
            let bytes = (chunks * self.block_size) as usize;
            let addr = src.phys() + offset * self.block_size;
            self.send_command(&fis, addr, bytes, true)?;
            offset += chunks;
        }

        Ok(())
    }

    fn zero_blocks(&self, start_block: u64, count: u64) -> Result<(), StorageError> {
        let _guard = self.lock.lock();
        let start_native = (start_block * BLOCK_SIZE) / self.block_size;
        let count_native = (count * BLOCK_SIZE).div_ceil(self.block_size);
        let max_native = (PRDS_PER_TABLE * PRD_MAX_BYTES) as u64 / self.block_size;

        if start_native + count_native > self.block_count { return Err(StorageError::BlockOutOfBounds) }
        
        let size = (count_native.min(MAX_SECTORS_PER_CMD).min(max_native) * self.block_size) as usize;
        let zero_buffer = PhysPage::new(size.div_ceil(PAGE_SIZE)).ok_or(StorageError::CommandFailed)?;

        let mut fis = FISRegisterH2D::new(0x80, 0x35, 0, 0, 0, 0, 0);

        let mut offset = 0;
        while offset < count_native {
            let chunks = (count_native - offset).min(MAX_SECTORS_PER_CMD).min(max_native);
            fis.set_lba(start_native + offset);
            fis.count = chunks as u16;
            let bytes = (chunks * self.block_size) as usize;
            self.send_command(&fis, zero_buffer.phys(), bytes, true)?;

            offset += chunks;
        }

        Ok(())
    }

    fn sync(&self) -> Result<(), StorageError> {
        let _guard = self.lock.lock();
        let fis = FISRegisterH2D::new(0x80, 0xEA, 0, 0, 0, 0, 0);
        self.send_command(&fis, PhysAddr::zero(), 0, false)
    }
}