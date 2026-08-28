//! Kernel memory allocators.

mod buddy;
mod slab;
mod range;

pub use buddy::BuddyAllocator;
pub use range::RangeAllocator;
