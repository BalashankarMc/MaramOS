use core::fmt::Debug;

pub enum KernelError {
    BadLimineResp,
    MemoryError(MemoryError),
}

impl Debug for KernelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            Self::BadLimineResp => "Bootloader (Limine) provided invalid response",
            Self::MemoryError(x) => x.to_str()
        };

        f.write_str(s)
    }
}

impl From<MemoryError> for KernelError {
    fn from(value: MemoryError) -> Self {
        Self::MemoryError(value)
    }
}

pub type KernelResult<T> = Result<T, KernelError>;

pub enum MemoryError {
    OutOfMemory,
    InvalidMapping
}

impl MemoryError {
    const fn to_str<'a>(&self) -> &'a str {
        match self {
            Self::OutOfMemory => "Out of Memory!",
            Self::InvalidMapping => "Attempted to perform an invalid mapping operation"
        }
    }
}
