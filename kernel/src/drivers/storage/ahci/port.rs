//! AHCI port operations.
//!
//! Stop/start the command engine, rebase command list and FIS buffers,
//! probe for connected devices, and find free command slots.

use x86_64::PhysAddr;

use super::{
    super::{TIMEOUT, IOError},
    registers::PortRegisters
};

const MAX_SLOTS: u8 = 0x20;

pub fn stop_cmd(port: &mut PortRegisters) -> Result<(), IOError> {
    port.set_cmd(port.cmd() & !0x01);

    let deadline = crate::acpi::passed_nanos() + TIMEOUT;
    while port.cmd() & 0x8000 != 0 {
        if crate::acpi::passed_nanos() >= deadline { return Err(IOError::Timeout) }
        core::hint::spin_loop();
    }

    port.set_cmd(port.cmd() & !0x10);

    let deadline = crate::acpi::passed_nanos() + TIMEOUT;
    while port.cmd() & 0x4000 != 0 {
        if crate::acpi::passed_nanos() >= deadline { return Err(IOError::Timeout) }
        core::hint::spin_loop();
    }
    Ok(())
}

pub fn start_cmd(port: &mut PortRegisters) -> Result<(), IOError> {
    let deadline = crate::acpi::passed_nanos() + TIMEOUT;
    #[allow(clippy::while_immutable_condition)]
    while port.cmd() & 0x8000 != 0 {
        if crate::acpi::passed_nanos() >= deadline { return Err(IOError::Timeout) }
        core::hint::spin_loop();
    }

    port.set_cmd(port.cmd() | 0x11);

    Ok(())
}

pub fn rebase(port: &mut PortRegisters, cmd_list_base: PhysAddr, fis_base: PhysAddr) -> Result<(), IOError> {
    stop_cmd(port)?;

    port.set_interrupt_status(u32::MAX);
    port.set_sata_err(u32::MAX);

    port.set_cmd_list_base_low(cmd_list_base.as_u64() as u32);
    port.set_cmd_list_base_high((cmd_list_base.as_u64() >> 32) as u32);
    port.set_fis_base_low(fis_base.as_u64() as u32);
    port.set_fis_base_high((fis_base.as_u64() >> 32) as u32);
    start_cmd(port)?;
    Ok(())
}

pub fn probe(port: &PortRegisters) -> Option<u32> {
    let sata_status = port.sata_status();
    let device_ready = sata_status & 0xF == 3;
    let power_ready = (sata_status >> 8) & 0xF == 1;

    if device_ready && power_ready { return Some(port.signature()) }

    None
}

pub fn find_slot(port: &PortRegisters) -> Option<u8> {
    let slots = port.sata_active() | port.cmd_issue();
    for i in 0..MAX_SLOTS {
        if slots & (1 << i as u32) == 0 { return Some(i) }
    }
    None
}
