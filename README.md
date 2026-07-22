# Maram OS

An x86_64 monolithic kernel written in Rust (`#![no_std]`) that boots via the [Limine](https://limine-bootloader.org/) bootloader on UEFI firmware.

## Features

- **Frame-buffer text console** -- PSF2 font rendering directly to a linear framebuffer with colored logging macros
- **Preemptive multitasking** -- per-CPU schedulers with priority-based task selection, burst prediction, and context switching
- **User-space processes** -- ELF loading, demand paging (page faults), and a `syscall`/`sysret` interface
- **LemonFS** -- custom filesystem with bitmap allocator, hash-table directory, and link chains for large files
- **GPT partition parsing** -- backup-header fallback, CRC32 integrity checks
- **Storage drivers** -- AHCI/SATA and NVMe with an LRU block cache
- **PCIe enumeration** -- ECAM (MCFG) with hierarchical bridge scanning
- **ACPI** -- RSDP/XSDT parsing for HPET, APIC (Local + I/O), FADT, MCFG
- **FPU/SSE lazy save/restore** -- CR0.TS and `#NM` trapping for efficient context switching
- **SMP** -- multi-core support with Inter-Processor Interrupts (TLB shootdown, function calls, rescheduling, panic halt)
- **Load balancing** -- cross-core task migration with rate-limited rebalancing

## Architecture

```
Limine Bootloader (UEFI)
    |
    v
Kernel Init (boot/mod.rs)
  Memory, Framebuffer, FPU, ACPI, GDT/IDT, LAPIC, SMP,
  PCI, Syscalls, Storage, GPT, LemonFS
    |
    v
Scheduler (per-CPU, priority-based with burst prediction)
    |
    v
User-space ELF loader + demand paging + syscall dispatch
```

### Memory Layout

| Region | Virtual Address Range | Description |
|---|---|---|
| Higher-half kernel | `0xFFFFFFFF80000000`+ | Kernel code and data |
| HHDM | Physical + offset | Direct physical-to-virtual mapping |
| Kernel heap | `0xFFFF_C000_0000_0000` | Buddy allocator managed heap |
| MMIO | `0xFFFF_FE00_0000_0000` -- `0xFFFF_FFFF_8000_0000` | Memory-mapped I/O regions |
| User space | `0x0000_0000_0000_0000`+ | Demand-paged user address space |

## Project Structure

```
OS/
  Makefile              # Top-level build system
  kernel/
    Cargo.toml          # Kernel crate configuration
    linker.ld           # Kernel linker script
    src/
      main.rs           # Kernel entry point (kmain)
      stdout.rs         # Framebuffer console + logging macros
      fpu.rs            # FPU/SSE initialization and lazy context switching
      tests.rs          # Integration test suite (feature-gated)
      boot/             # Initialization sequence + Limine requests
      memory/           # Page allocator, heap, page tables, wrappers
      allocator/        # Buddy allocator + slab sub-allocator
      descriptors/      # GDT + IDT setup
      cpu/              # Per-CPU state, IPIs, SMP startup
      acpi/             # ACPI table parsing, APIC, HPET, FADT, MCFG
      drivers/
        pci/            # PCIe enumeration + MSI/MSI-X
        storage/        # Storage abstraction, block cache
          nvme/         # NVMe driver
          ahci/         # AHCI/SATA driver
      fs/
        lemonfs/        # LemonFS implementation
      gpt/              # GPT partition table parsing
      scheduling/       # Scheduler, tasks, load balancing
      syscalls/         # Syscall entry + dispatch
      library/          # LateInit, Time, CRC32, InterruptMutex
      loader/           # ELF loader
  asm/                  # Test user-space assembly programs
  tools/
    lemoncc/            # Host-side tool: compile .asm + inject into LemonFS
  limine-files/         # Bootloader binaries and config
```

## Building

### Prerequisites

- Rust nightly toolchain with `x86_64-unknown-none` target
- `nasm` (for assembly files)
- `parted`, `sgdisk` (for disk image creation)
- `mtools` (for FAT32 ESP formatting)
- `xorriso` (for ISO creation, test mode)
- QEMU with OVMF (for running/testing)

### Quick Start

```bash
# Build the kernel and create a bootable disk image
make

# Run in QEMU (requires OVMF at /usr/share/OVMF/x64/OVMF.4m.fd)
make run

# Run with NVMe emulation instead of AHCI
make run DISK=nvme
```

### Test Suite

```bash
# Run integration tests in QEMU (captures serial output)
make test

# Run Rust unit tests
make test-unit

# Debug build with GDB attach
make test-gdb
```

The integration test suite (`kernel/src/tests.rs`) is compiled only with the `integration-test` Cargo feature and covers:

- **Library** -- LateInit, Time conversions
- **Buddy allocator** -- block math, alloc/free, split/merge
- **Memory** -- page allocation, MMIO mapping
- **Bitmap** -- set/check/clear bits, bounds checking
- **Hash functions** -- FNV-1a determinism, name encoding roundtrips
- **FileType** -- enum discriminants
- **Directory cache** -- insert/lookup/remove, LRU eviction
- **Scheduler** -- priority calculation, burst prediction
- **CRC32** -- known test vectors, empty input, known string
- **HashPointer** -- LBA/offset calculation
- **PhysPage** -- write/read data, leak
- **SuperBlock** -- layout calculations

### Deploy to Physical Hardware

```bash
# WARNING: This will destroy all data on the target device
make deploy DEVICE=/dev/sdX
```

## LemonFS

LemonFS is a custom filesystem with the following on-disk layout:

1. **SuperBlock** (1 LBA) -- magic, version, block counts, dirty flag
2. **Hash Table** (variable) -- FNV-1a hash table with open addressing and tombstones
3. **Bitmap** (variable) -- block allocation tracking
4. **Data** (remaining) -- file contents with link chains for files exceeding one block

Key design decisions:
- Fixed 40-byte filenames (null-padded)
- Open-addressing hash table with tombstone deletion
- Link chains for large files (direct + indirect pointers)
- Auto-format on first mount

## Syscalls

Currently implemented:

| Number | Name | Description |
|---|---|---|
| 0 | `PRINT` | Print a string from the user's syscall buffer |

The syscall interface uses the `syscall`/`sysret` instructions with STAR/LSTAR/SFMask MSRs.

## License

This project does not currently include a license file. Contact the author for usage terms.
