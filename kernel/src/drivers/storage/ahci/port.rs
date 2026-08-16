//! AHCI port operations.
//!
//! Stop/start the command engine, rebase command list and FIS buffers,
//! probe for connected devices, and find free command slots.

use x86_64::PhysAddr;

use crate::helpers::{Time, wait, wait_timeout};

use super::{
    super::{TIMEOUT, StorageError},
    registers::PortRegisters
};

pub fn stop_cmd(port: &PortRegisters) -> Result<(), StorageError> {
    port.cmd.write(port.cmd.read() & !0x01);

    wait_timeout(|| port.cmd.read() & 0x8000 != 0, &TIMEOUT).ok_or(StorageError::Timeout)?;

    port.cmd.write(port.cmd.read() & !0x10);

    wait_timeout(|| port.cmd.read() & 0x4000 != 0, &TIMEOUT).ok_or(StorageError::Timeout)?;

    Ok(())
}

pub fn start_cmd(port: &PortRegisters) -> Result<(), StorageError> {
    wait_timeout(|| port.cmd.read() & 0x8000 != 0, &TIMEOUT).ok_or(StorageError::Timeout)?;

    port.cmd.write(port.cmd.read() | 0x11);

    Ok(())
}

pub fn rebase(port: &PortRegisters, cmd_list_base: PhysAddr, fis_base: PhysAddr) -> Result<(), StorageError> {
    stop_cmd(port)?;

    port.is.write(u32::MAX);
    port.serr.write(u32::MAX);

    port.clb.write(cmd_list_base.as_u64() as u32);
    port.clbu.write((cmd_list_base.as_u64() >> 32) as u32);
    port.fb.write(fis_base.as_u64() as u32);
    port.fbu.write((fis_base.as_u64() >> 32) as u32);
    start_cmd(port)?;
    Ok(())
}

pub fn com_reset(port: &PortRegisters) -> bool {
    // Clear SERR
    port.serr.write(u32::MAX);

    // Set
    let sctl = port.sctl.read();
    port.sctl.write((sctl & !0xF) | 1);

    // Wait 1ms (Hold)
    wait(&Time::Milliseconds(1));

    // Release
    let sctl = port.sctl.read();
    port.sctl.write(sctl & !0xF);

    // Wait for linkup
    let closure = || {
        let ssts = port.ssts.read();
        (ssts & 0xF) != 3 && ((ssts >> 8) & 0xF) != 1
    };
            
    wait_timeout(closure, &TIMEOUT)
}

pub fn find_slot(port: &PortRegisters, ncs: u8) -> Option<u8> {
    let slots = port.sact.read() | port.ci.read();
    for i in 0 ..= ncs {
        if slots & (1 << u32::from(i)) == 0 { return Some(i) }
    }
    None
}
