use core::fmt::Debug;

pub enum KernelError {
    BadLimineResp,
    MemoryError(MemoryError),
    ACPIError(ACPIError)
}

impl Debug for KernelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::BadLimineResp => "Bootloader (Limine) provided invalid response",
            Self::MemoryError(x) => x.to_str(),
            Self::ACPIError(x) => x.to_str()
        };

        f.write_str(s)
    }
}

impl From<MemoryError> for KernelError {
    fn from(value: MemoryError) -> Self {
        Self::MemoryError(value)
    }
}

impl From<ACPIError> for KernelError {
    fn from(value: ACPIError) -> Self {
        Self::ACPIError(value)
    }
}

pub type KernelResult<T> = Result<T, KernelError>;

#[derive(Debug)]
pub enum MemoryError {
    OutOfMemory,
    InvalidMapping
}

impl MemoryError {
    const fn to_str(&self) -> &'static str {
        match self {
            Self::OutOfMemory => "Out of Memory!",
            Self::InvalidMapping => "Attempted to perform an invalid mapping operation"
        }
    }
}

#[derive(Debug)]
pub enum ACPIError {
    RSDPIntegrityFailed,
    RSDPUnsupportedRevision,
    XSDTChecksumFailed,
    SDTChecksumFailed,
    FADTRevisionTooOld,
    FADTNoResetRegister,
    FADTUnsupportedResetAddressSpace,
    HPETPeriodZero,
    MADTNoIoApicFound,
    MADTEntryLengthZero,
    IOAPICNotInitialized,
    GSIUnderflow,
    HPETNotInitialized,
    LAPICBaseNotMapped,
    NoX2APIC
}

impl ACPIError {
    const fn to_str(&self) -> &'static str {
        match self {
            Self::RSDPIntegrityFailed => "RSDP checksum is invalid",
            Self::RSDPUnsupportedRevision => "RSDP revision is not 2 (ACPI 2.0+)",
            Self::XSDTChecksumFailed => "XSDT checksum is invalid",
            Self::SDTChecksumFailed => "SDT entry checksum is invalid",
            Self::FADTRevisionTooOld => "FADT revision is too old (requires >= 2)",
            Self::FADTNoResetRegister => "FADT has no reset register",
            Self::FADTUnsupportedResetAddressSpace => "FADT reset register uses unsupported address space",
            Self::HPETPeriodZero => "HPET capability register reports zero period",
            Self::MADTNoIoApicFound => "MADT contains no I/O APIC entry",
            Self::MADTEntryLengthZero => "MADT entry has zero length (infinite loop)",
            Self::IOAPICNotInitialized => "I/O APIC was not initialized (missing MADT entry)",
            Self::GSIUnderflow => "GSI is below I/O APIC GSI base",
            Self::HPETNotInitialized => "HPET has not been initialized yet",
            Self::LAPICBaseNotMapped => "LAPIC base address frame is zero/unmapped",
            Self::NoX2APIC => "System does not support X2APIC mode"
        }
    }
}