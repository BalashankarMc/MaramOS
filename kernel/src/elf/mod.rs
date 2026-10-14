use alloc::vec::Vec;
use thiserror::Error;
use x86_64::VirtAddr;

use crate::memory::{PhysPage, VMemAllocator};

mod loader;

pub use loader::load;

pub struct LoadedBinary {
    pub entry: VirtAddr,
    pub address_space: (VMemAllocator, PhysPage),
    pub segments: Vec<PhysPage>
}

#[derive(Debug, Error)]
pub enum LoaderError {
    #[error("File is not ELF64!")]
    BadClass,
    #[error("Target for this ELF is not x86_64!")]
    BadTarget,
    #[error("Unsupported ELF Type!")]
    BadType,
    #[error("ELF File has no segments!")]
    NoSegments,
    #[error("ELF has overlapping segments!")]
    OverlappingSegments,
    #[error("ELF has invalid segemts")]
    BadSegments,

    #[error(transparent)]
    ParseError(#[from] elf::ParseError),

    #[error(transparent)]
    MemoryError(#[from] crate::memory::MemoryError),

    #[error(transparent)]
    IO(#[from] crate::fs::FSError)
}