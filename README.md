# MaramOS

A 64-bit x86_64 operating system kernel written in Rust.

## Overview

MaramOS is a monolithic x86_64 kernel written in `#![no_std]` Rust (nightly). It boots
on UEFI systems via the [Limine](https://github.com/limine-bootloader/limine) bootloader.
At startup it initializes the platform, then loads and runs `/bin/init` from its LemonFS
filesystem.

## Building and Running

### Prerequisites

- **Rust**: the nightly toolchain pinned in `rust-toolchain.toml`.
- **Assembler/linker**: `nasm` and `ld` (used to build `bins/`).
- **Disk tools**: `truncate`, `sgdisk`, and `mtools` (`mformat`, `mmd`, `mcopy`).
- **LemonFS tools**: `lemonmnt` and `fusermount`.
- **QEMU**: `qemu-system-x86_64`.
- **OVMF firmware**: `/usr/share/edk2/x64/OVMF.4m.fd`.

The build is driven by an `xtask` binary (`xtask/src/main.rs`). A Cargo alias in
`.cargo/config.toml` lets you invoke it as `cargo xtask`.

### Commands

```sh
# Build the kernel and copy it to ./kernel.elf
cargo xtask build

# Assemble bins/, create disk.img (ESP + LemonFS), and populate /bin
cargo xtask disk

# Boot the disk in QEMU (OVMF, q35, AHCI, 4 cores)
cargo xtask run

# Remove build artifacts (kernel.elf, disk.img, mount/, compiled/)
cargo xtask clean
```

A typical workflow is:

```sh
cargo xtask build && cargo xtask disk && cargo xtask run
```

## System Calls (planned)

> **Status: not yet implemented.**

MaramOS does not currently expose a system call interface. User programs are loaded and
scheduled, but there is no `syscall`/`sysret` (or `int`-based) entry point and no syscall
dispatch table, so userspace cannot yet request kernel services.

The intended design is a `syscall`/`sysret` fast path configured through the EFER, `STAR`,
`LSTAR`, and `FMASK` MSRs, dispatching by syscall number. This section will be expanded
once the ABI is implemented.

## License

Licensed under the Apache License, Version 2.0 ([LICENSE](LICENSE) or
https://www.apache.org/licenses/LICENSE-2.0).

Unless required by applicable law or agreed to in writing, software distributed under
the License is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
KIND, either express or implied. See the License for the specific language governing
permissions and limitations under the License.
