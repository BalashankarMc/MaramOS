use core::ops::Add;

use alloc::{sync::Arc, vec::Vec};
use x86_64::PhysAddr;

use crate::{drivers::pci::PCIFunction, helpers::wait_timeout, memory::{MMIORegion, PAGE_SIZE, PhysPage}, println};
use super::{StorageDrive, StorageError, TIMEOUT, BLOCK_SIZE};

mod fis;
mod irq;
mod port;
mod commands;
mod registers;

use registers::{GlobalRegisters, PortRegisters};
use commands::{CommandHeader, CommandTableHeader, PhysRegionDescTableEntry};
use fis::FISRegisterH2D;

const PRD_MAX_BYTES: usize = 1 << 22;
const PRDS_PER_TABLE: usize = 8;
const MAX_SECTORS_PER_CMD: usize = u16::MAX as usize;

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
    block_count: u64
}

impl AHCIDrive {
    fn send_command(&self, fis: &FISRegisterH2D, data: PhysAddr, data_bytes: usize, is_write: bool) -> Result<(), StorageError> {
        use StorageError::CommandFailed;
        
        // Get port specific registers
        let port = PortRegisters::new(&self.mapped_device.1, self.port_id)
            .ok_or(CommandFailed)?;

        // Find free slot
        let slot = port::find_slot(&port).ok_or(CommandFailed)?;

        // Initialize command header
        let mut cmd_header = CommandHeader::zeroed();
        cmd_header.fis_info.set(5, is_write);
        
        let ct_phys = self.command_tables.phys().as_u64() + u64::from(slot) * 0x100;
        cmd_header.cmd_table_base_addr_lower = ct_phys as u32;
        cmd_header.cmd_table_base_addr_upper = (ct_phys >> 32) as u32;

        // Zero command table header
        let offset = usize::from(slot) * 0x100;
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
                self.command_tables.address().add(0x80 + i as u64 * 16).as_mut_ptr::<PhysRegionDescTableEntry>()
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

        // Write CI (Command Issued) flag
        port.ci.write(1 << slot);

        if !wait_timeout(|| (port.ci.read() >> slot) & 1 == 1, &TIMEOUT) { return Err(StorageError::Timeout) }

        if port.is.read() & (1 << 30) != 0 {
            port.is.write(1 << 30);
            return Err(CommandFailed)
        }

        Ok(())
    }
}

pub fn init(device: PCIFunction) -> Result<Vec<AHCIDrive>, StorageError> {
    use StorageError::{InitFailed, PCIeError};
    
    // Initliaze the PCI device
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
    let num_ports = (cap & 0x1F) + 1;

    if (cap >> 31) & 1 != 1 { return Err(InitFailed) } // No 64-bit addressing :(

    if global_registers.cap2.read() & 2 == 2 { // Handoff supported
        let bohc = global_registers.bohc.read();
        global_registers.bohc.write(bohc | 2); // Request Ownership

        // Wait for ownership / timeout
        if !wait_timeout(|| (global_registers.bohc.read() >> 4).trailing_zeros() < 2, &TIMEOUT) { return Err(StorageError::Timeout) }
    }

    // Controller reset
    let ghc = global_registers.ghc.read();
    global_registers.ghc.write(ghc | 1);
    if !wait_timeout(|| global_registers.ghc.read() & 1 == 1, &TIMEOUT) { return Err(StorageError::Timeout) }

    // Enable AHCI Mode
    let ghc = global_registers.ghc.read();
    global_registers.ghc.write(ghc | (1 << 31));

    // Get the PI(Ports Implemented). We do this here to prevent borrow checking problems with `mapped`
    let pi = global_registers.pi.read();    

    // Store the mapped PCI Device and get a new reference to its mapped ABAR
    let mapped = Arc::new(MappedPCI(device, bar));
    let bar = &mapped.1;

    let mut drives = Vec::new(); // Return value containing the initialized drives

    for port_id in 0..num_ports {
        if (pi >> port_id) & 1 == 0 { continue } // No port here
        
        let port_registers = PortRegisters::new(bar, port_id).ok_or(InitFailed)?;

        // Write SUD (Boot brive if it uses staggered spin-up) and POD
        let cmd = port_registers.cmd.read();
        port_registers.cmd.write(cmd | 6);
        if !wait_timeout(|| port_registers.ssts.read() & 0xF != 3, &TIMEOUT) { continue } // Wait for drive boot

        let ssts = port_registers.ssts.read();
        let ipm = (ssts >> 8) & 0xF;
        let det = ssts & 0xF;

        if det + ipm == 0 { continue } // No device (DET and IPM are 0)

        if det == 3 && ipm != 1 && !port::com_reset(&port_registers) { continue } // Power up the device (COMRESET)

        port_registers.serr.write(u32::MAX); // Clear SERR

        if port_registers.sig.read() != 0x101 { continue } // Not a SATA Drive

        // Allocate memory for DMA Buffers
        let command_list = PhysPage::new(1).ok_or(InitFailed)?;
        let fis_buffer = PhysPage::new(1).ok_or(InitFailed)?;
        let command_tables = PhysPage::new(2).ok_or(InitFailed)?;

        // Update port with correct buffers
        port::rebase(&port_registers, command_list.phys(), fis_buffer.phys())?;

        // Drive struct missing `block_size` and `block_count` (Need to issue INDENTIFY)
        let mut drive = AHCIDrive {
            mapped_device: mapped.clone(),
            port_id,
            command_list,
            command_tables,
            _fis_buffer: fis_buffer,
            block_size: 0,
            block_count: 0
        };

        let id_buf = PhysPage::new(1).ok_or(InitFailed)?;
        let fis = FISRegisterH2D::new(0x80, 0xEC, 0, 0, 0, 0, 0);

        drive.send_command(&fis, id_buf.phys(), 512, false).map_err(|_| InitFailed)?;
        drive.block_count = id_buf.read_data::<u64>(200).ok_or(InitFailed)?;
        drive.block_size = u64::from(id_buf.read_data::<u32>(212).ok_or(InitFailed)?);

        println!("Drive Block Size: {}", drive.block_size);

        drives.push(drive);
    }
    Ok(drives)
}

impl StorageDrive for AHCIDrive {
    fn block_count(&self) -> u64 {
        (self.block_count * self.block_size) / BLOCK_SIZE as u64
    }

    fn read_blocks(&self, start_block: u64, count: u64, _dest: &mut PhysPage) -> Result<(), StorageError> {
        if start_block + count > self.block_count() { return Err(StorageError::BlockOutOfBounds) }

        todo!()
    }

    fn write_blocks(&self, _start_block: u64, _count: u64, _src: &PhysPage) -> Result<(), StorageError> {
        todo!()
    }

    fn zero_blocks(&self, _start_block: u64, _count: u64) -> Result<(), StorageError> {
        todo!()
    }
}