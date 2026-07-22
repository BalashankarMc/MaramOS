//! Kernel memory allocators.
//!
//! Provides a buddy allocator for power-of-two page allocations and a slab
//! sub-allocator for non-power-of-two counts. The buddy allocator splits and
//! merges blocks in coalescing free lists; the slab allocator carves out
//! exact-size ranges from buddy-provided blocks to eliminate rounding waste.

mod buddy;
mod slab;

pub use buddy::BuddyAllocator;
