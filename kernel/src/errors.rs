use thiserror::Error;

use crate::{drivers::{pci::PCIError, ps2::PS2Error, storage::{GPTError, StorageError}}, fs::FSError};

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
    IDTRegisterError(u8)
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
pub enum MemoryError {
    #[error("Out of Memory!")]
    OutOfMemory,
    #[error("Attempted to perform an invalid mapping operation")]
    InvalidMapping,
    #[error("Attempted to access memory out of bounds")]
    OutOfBounds
}

#[derive(Error, Debug)]
pub enum ACPIError {
    #[error("RSDP checksum is invalid")]
    RSDPIntegrityFailed,
    #[error("RSDP revision is not 2 (ACPI 2.0+)")]
    RSDPUnsupportedRevision,
    #[error("XSDT checksum is invalid")]
    XSDTChecksumFailed,
    #[error("SDT entry checksum is invalid")]
    SDTChecksumFailed,
    #[error("FADT revision is too old (requires >= 2)")]
    FADTRevisionTooOld,
    #[error("FADT has no reset register")]
    FADTNoResetRegister,
    #[error("FADT reset register uses unsupported address space")]
    FADTUnsupportedResetAddressSpace,
    #[error("HPET capability register reports zero period")]
    HPETPeriodZero,
    #[error("MADT contains no I/O APIC entry")]
    MADTNoIoApicFound,
    #[error("MADT entry has zero length (infinite loop)")]
    MADTEntryLengthZero,
    #[error("I/O APIC was not initialized (missing MADT entry)")]
    IOAPICNotInitialized,
    #[error("GSI is below I/O APIC GSI base")]
    GSIUnderflow,
    #[error("HPET has not been initialized yet")]
    HPETNotInitialized,
    #[error("LAPIC base address frame is zero/unmapped")]
    LAPICBaseNotMapped,
    #[error("System does not support X2APIC mode")]
    NoX2APIC,
}

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