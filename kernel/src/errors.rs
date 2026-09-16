use thiserror::Error;

use crate::acpi::ACPIError;
use crate::drivers::pci::PCIError;
use crate::drivers::ps2::PS2Error;
use crate::drivers::storage::{GPTError, StorageError};
use crate::elf::LoaderError;
use crate::fs::FSError;
use crate::memory::MemoryError;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Bootloader (Limine) provided invalid response")]
    BadLimineResp,
    #[error(transparent)]
    MemoryError(#[from] MemoryError),
    #[error(transparent)]
    ACPIError(#[from] ACPIError),
    #[error(transparent)]
    DriverError(#[from] DriverError),
    #[error(transparent)]
    FSError(#[from] FSError),
    #[error("Failed to register IDT entry")]
    IDTRegisterError(u8),
    #[error("Failed to load binary!")]
    LoaderError(#[from] LoaderError)
}

impl From<PS2Error> for KernelError {
    fn from(value: PS2Error) -> Self { Self::DriverError(value.into()) }
}

impl From<PCIError> for KernelError {
    fn from(value: PCIError) -> Self { Self::DriverError(value.into()) }
}

impl From<StorageError> for KernelError {
    fn from(value: StorageError) -> Self { Self::DriverError(value.into()) }
}

impl From<GPTError> for KernelError {
    fn from(value: GPTError) -> Self { Self::DriverError(value.into()) }
}

pub type KResult<T> = Result<T, KernelError>;

#[derive(Error, Debug)]
pub enum DriverError {
    #[error(transparent)]
    PS2(#[from] PS2Error),
    #[error(transparent)]
    PCIe(#[from] PCIError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Gpt(#[from] GPTError)
}