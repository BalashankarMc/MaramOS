# GPT Parser & Partition Device Multiplexer — Architectural Blueprint
### Target: x86_64 `#![no_std]` Monolithic Kernel, Limine/UEFI, SMP-enabled

---

## 1. Exact Structure Layouts & Memory Blueprints

### 1.1 Protective MBR (LBA 0, 512 bytes)

The Protective MBR exists purely to prevent legacy MBR-only tools from misinterpreting the disk as unpartitioned/corrupt. Your parser must treat it as a **sanity gate**, not a data source.

| Offset (dec) | Size (bytes) | Rust Type | Field Name | Purpose |
|---|---|---|---|---|
| 0x000 | 440 | `[u8; 440]` | Boot Code Area | Unused under GPT. Typically zeroed by mkgpt tools, but must NOT be assumed zero — never validate against it. |
| 0x1B8 | 4 | `u32` | Unique Disk Signature | Legacy BIOS field. Ignore. |
| 0x1BC | 2 | `u16` | Reserved (0x0000) | Ignore. |
| 0x1BE | 16 | `[u8; 16]` | Partition Entry #1 (Protective) | The only entry that matters. Must describe the "protective" partition. |
| 0x1CE | 16 | `[u8; 16]` | Partition Entry #2 | Must be all-zero in a strict protective MBR. |
| 0x1DE | 16 | `[u8; 16]` | Partition Entry #3 | Must be all-zero. |
| 0x1EE | 16 | `[u8; 16]` | Partition Entry #4 | Must be all-zero. |
| 0x1FE | 2 | `u16` | Boot Signature | Must equal `0x55AA` (bytes: `0x55, 0xAA` at offsets 0x1FE/0x1FF). |

**Protective Partition Entry #1 sub-layout (16 bytes, at offset 0x1BE):**

| Sub-offset | Size | Rust Type | Field | Expected Value |
|---|---|---|---|---|
| +0x0 | 1 | `u8` | Boot Indicator | `0x00` (non-bootable) |
| +0x1 | 3 | `[u8; 3]` | Starting CHS | `0x000200` (historical filler, do not rely on it) |
| +0x4 | 1 | `u8` | OS Type / Partition Type | `0xEE` (GPT Protective) |
| +0x5 | 3 | `[u8; 3]` | Ending CHS | Historical filler, ignore |
| +0x8 | 4 | `u32` | Starting LBA (LE u32) | `0x00000001` |
| +0xC | 4 | `u32` | Size in LBA (LE u32) | Disk size − 1, **or** `0xFFFFFFFF` if disk exceeds 32-bit LBA range |

**Validation rule:** Check byte 0x1FE:0x1FF == `55 AA` and that Partition Entry #1's OS Type == `0xEE`. Do **not** hard-fail if entries 2–4 are non-zero — some vendor tools populate them incorrectly; log a warning and proceed to GPT header validation, which is the authoritative source.

---

### 1.2 Primary GPT Header (LBA 1, typically 512 bytes sector, header itself is 92 bytes minimum)

| Offset (dec) | Size | Rust Type | Field Name | Purpose |
|---|---|---|---|---|
| 0x00 | 8 | `[u8; 8]` | Signature | Must equal ASCII `"EFI PART"` (`45 46 49 20 50 41 52 54`) |
| 0x08 | 4 | `u32` (LE) | Revision | Typically `0x00010000` (v1.0) |
| 0x0C | 4 | `u32` (LE) | Header Size | Size of this header structure in bytes (usually 92 = 0x5C). Used for CRC32 bounds. |
| 0x10 | 4 | `u32` (LE) | Header CRC32 | CRC32 of header bytes `[0..HeaderSize)` with **this field zeroed during computation** |
| 0x14 | 4 | `u32` | Reserved | Must be `0x00000000` |
| 0x18 | 8 | `u64` (LE) | My LBA | LBA of this header (1 for primary, last LBA for backup) |
| 0x20 | 8 | `u64` (LE) | Alternate LBA | LBA of the other header (backup ↔ primary cross-reference) |
| 0x28 | 8 | `u64` (LE) | First Usable LBA | First LBA usable for partitions (after primary array) |
| 0x30 | 8 | `u64` (LE) | Last Usable LBA | Last LBA usable for partitions (before backup array) |
| 0x38 | 16 | `[u8; 16]` | Disk GUID | Mixed-endian GUID; unique disk identifier |
| 0x48 | 8 | `u64` (LE) | Partition Entry LBA | Starting LBA of the GUID Partition Entry array (typically 2) |
| 0x50 | 4 | `u32` (LE) | Number of Partition Entries | Entry count in the array (commonly 128) |
| 0x54 | 4 | `u32` (LE) | Size of Partition Entry | Bytes per entry (commonly 128; **must never be assumed**, always read) |
| 0x58 | 4 | `u32` (LE) | Partition Entry Array CRC32 | CRC32 over the **entire raw entry array** bytes |
| 0x5C | * | `[u8; N]` (N = sector_size − 92) | Reserved (padding to sector size) | Zero-filled to end of logical sector; **must not** be parsed as data |

**Critical mapping structure to define in Rust:**
```
#[repr(C, packed)]
struct GptHeaderRaw { /* fields above, exact byte order */ }
```
This struct must be `packed` because on-disk layout has no natural alignment guarantees relative to Rust's default struct alignment rules (particularly the `u64` fields at non-8-byte-aligned relative offsets are fine here since header is naturally aligned, but treat as packed defensively since you will read it out of a raw disk-cache buffer that may not itself be 8-byte aligned in memory).

---

### 1.3 GPT Partition Entry (LBA 2 onward, entry size from header, commonly 128 bytes)

| Offset (dec) | Size | Rust Type | Field Name | Purpose |
|---|---|---|---|---|
| 0x00 | 16 | `[u8; 16]` | Partition Type GUID | Mixed-endian GUID; identifies filesystem/partition role (ESP, Linux, your LemonFS, etc.) |
| 0x10 | 16 | `[u8; 16]` | Unique Partition GUID | Mixed-endian GUID; per-partition unique identifier, distinct from type |
| 0x20 | 8 | `u64` (LE) | Starting LBA | First LBA of partition (inclusive) |
| 0x28 | 8 | `u64` (LE) | Ending LBA | Last LBA of partition (inclusive) |
| 0x30 | 8 | `u64` (LE, bitfield) | Attribute Flags | Bit 0: Platform required; Bit 1: EFI firmware ignore; Bit 2: Legacy BIOS bootable; Bits 48–63: type-specific (e.g., Windows read-only/hidden/no-drive-letter) |
| 0x38 | 72 | `[u16; 36]` (UTF-16LE) | Partition Name | Null-padded human-readable label |
| (0x80 if entry=128) | remaining | `[u8; N]` (N = entry_size − 128) | Vendor-specific padding | Only present if `Size of Partition Entry` > 128; must skip using header-declared stride, never hardcode 128 |

**GUID mixed-endian encoding warning (applies to Disk GUID, Type GUID, Unique GUID):**

A GUID/UUID on-disk in GPT is **not** pure big-endian or pure little-endian — it follows the Microsoft mixed-endian convention:

| Field | Bytes | Rust Type | Endianness on disk |
|---|---|---|---|
| `time_low` | 4 | `u32` | Little-endian |
| `time_mid` | 2 | `u16` | Little-endian |
| `time_hi_and_version` | 2 | `u16` | Little-endian |
| `clock_seq_hi_and_reserved` + `clock_seq_low` | 2 | `[u8; 2]` | Byte-order as-is (treated as raw bytes, not reversed) |
| `node` | 6 | `[u8; 6]` | Byte-order as-is (raw bytes, not reversed) |

This means: when you display or compare a GUID against a canonical string form (`XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`), the first three groups must be **byte-swapped** from disk order, while the last two groups are copied verbatim. Any GUID comparison logic (e.g., "is this the ESP GUID?") must either: (a) compare the raw 16-byte disk representation against a pre-swapped constant table, or (b) canonicalize both sides before comparing. Choose (a) for performance — store your reference GUID table in raw disk-byte order so comparisons are a straight `memcmp`-equivalent, avoiding per-comparison swapping.

---

### 1.4 Alignment & UB Avoidance Rules for Packed Structures in `#![no_std]`

These are non-negotiable given you're reading raw disk-cache buffers into typed structures on a target where Rust's alignment guarantees are enforced by the compiler/UB rules, not by the hardware (x86_64 tolerates unaligned loads, but Rust's aliasing/reference rules do not):

1. **Never cast a raw disk-cache byte buffer directly to `&GptHeaderRaw`.** A `&T` reference in Rust requires the pointer to satisfy `T`'s alignment, and creating a reference to underaligned memory is immediate UB, even if `#[repr(packed)]` is applied to the type itself. `repr(packed)` only removes *internal* padding between fields — it does not relax the requirement that the **whole struct's** memory address be validly referenceable; in practice with `packed`, all field accesses degrade to byte-wise reads, but you must still avoid taking `&` references to individual multi-byte fields inside a packed struct (that itself is documented UB-adjacent territory the Rust compiler warns about — "reference to packed field").

2. **Preferred pattern:** treat the on-disk structures as **pure byte-slice parsers**, not as directly transmuted Rust structs. Read each field via explicit `u32::from_le_bytes(buf[a..b].try_into().unwrap())` / `u64::from_le_bytes(...)` calls. This sidesteps alignment UB entirely because you are copying bytes into a properly-aligned local variable rather than referencing unaligned memory in place. This is the only fully sound approach in a `no_std` environment without relying on `read_unaligned`.

3. **If you must use `repr(packed)` structs for ergonomic field access,** access fields only by value (copy out), never by reference — e.g. avoid `&header.signature`; instead bind `let sig = header.signature;` immediately, or better, use `core::ptr::addr_of!(header.field)` to get a raw (unaligned-safe) pointer, then `core::ptr::read_unaligned` to extract the value. This is the sound low-level escape hatch when byte-slicing is too slow for hot paths.

4. **Endianness discipline:** every multi-byte numeric field on GPT disk structures is little-endian by spec. On x86_64 this happens to match native endianness, but **do not rely on native-endian transmutation** — always route through explicit `from_le_bytes`/`to_le_bytes`. This keeps the code correct if ever ported and documents intent for auditors.

5. **CRC32 computation buffers must be heap-allocated copies**, not in-place mutation of the disk-cache buffer, because you need to zero the Header CRC32 field before computing — mutating a shared disk-cache page in place would corrupt the cache's view of on-disk truth for other concurrent readers under SMP.

6. **Sector size assumption hazard:** never hardcode 512-byte sectors. Query your block-device abstraction layer for the physical/logical sector size (4Kn drives exist) and use it to compute the byte offset of LBA 1, LBA 2, etc. `partition_entry_lba * sector_size`, always via `checked_mul`.

---

## 2. Step-by-Step Parsing & Validation Plan

### Phase 1: Integrity Attestation

Sequence, in strict order, short-circuiting to a fallback (backup header) or hard failure on any step:

1. **Read LBA 0** via the block-device abstraction (single sector read through the disk cache layer).
2. **Validate MBR boot signature**: bytes `[510..512]` == `[0x55, 0xAA]`. Failure ⇒ not a valid disk image at all; abort GPT parsing entirely (this is a lower-level failure than "no GPT present").
3. **Inspect protective entry OS Type** (byte offset `0x1BE + 4` == `0xEE`). Absence doesn't necessarily mean "no GPT" (some disks are hybrid or malformed) — treat this as advisory, log, and proceed to step 4 regardless, since the GPT header is the authoritative source.
4. **Read LBA 1** (Primary GPT Header sector) via block-device abstraction.
5. **Validate Signature field**: bytes `[0..8]` == ASCII `"EFI PART"`. Hard failure if mismatched — attempt fallback to backup header (see step 10).
6. **Validate Header Size field** (`0x0C`): must be ≥ 92 (minimum spec size) and ≤ sector size (sanity upper bound — a header claiming to be larger than the sector it lives in is corrupt). Reject with hard failure otherwise.
7. **Validate Header CRC32**: copy header bytes `[0..HeaderSize)` into a heap-allocated scratch buffer, zero out the 4-byte CRC32 field at relative offset `0x10` in the *copy*, compute CRC32 (standard IEEE 802.3 polynomial, matching UEFI spec algorithm) over the copy, compare to the on-disk value. Mismatch ⇒ header corrupt, fall back to backup header at "Alternate LBA."
8. **Cross-validate `My LBA` field** equals 1 (for primary). Mismatch indicates the header was read from the wrong location or disk layout is inconsistent — treat as corruption.
9. **Sanity-bound `First Usable LBA` / `Last Usable LBA`** against the disk's total sector count (obtained from your block-device layer's capacity query) using `checked` comparisons: `first_usable <= last_usable < disk_total_sectors`. Reject on violation.
10. **Backup header fallback protocol**: if primary fails CRC or signature validation, read LBA = disk_total_sectors − 1 (the backup header's canonical location), re-run steps 5–9 against it. If backup also fails, surface a hard "GPT unreadable" error to the caller — do not guess or synthesize a partition table.
11. **Validate `Size of Partition Entry`** (`0x54`): must be ≥ 128 (spec minimum) and a multiple of 8 (spec requirement for entry alignment). Reject non-conforming values.

### Phase 2: Dynamic Ingestion Algorithm

The partition entry array's total size is **not fixed** — it must be derived from header metadata, never assumed to be 128 entries × 128 bytes (16 KiB), though that is the common case.

**Computation sequence (all using `checked_*` arithmetic, propagating `None`/error on overflow rather than panicking):**

1. `entry_count: u32` = header's Number of Partition Entries (offset `0x50`).
2. `entry_size: u32` = header's Size of Partition Entry (offset `0x54`), already validated ≥128 and multiple-of-8 in Phase 1.
3. `array_total_bytes: u64` = `entry_count.checked_mul(entry_size)` widened to u64 — reject on overflow (a corrupt header could claim absurd values here; this is your primary defense against a maliciously or accidentally corrupted header causing an unbounded allocation).
4. **Impose a hard upper-bound cap** on `array_total_bytes` (e.g., reject anything beyond a sane ceiling like 16 MiB) *before* allocating — this defends against denial-of-service via a corrupted/hostile header requesting a multi-gigabyte heap allocation.
5. `sector_size: u64` = queried from block-device abstraction for this device.
6. `array_sector_count: u64` = `(array_total_bytes + sector_size - 1) / sector_size` (ceiling division, using checked_add/checked_div) — this is the number of sectors you must read starting at `Partition Entry LBA`.
7. **Bounds-check the array's sector span** against `First Usable LBA`: `partition_entry_lba.checked_add(array_sector_count)? <= first_usable_lba` must hold (the array must not overlap the usable partition region). Reject otherwise.

**Ingestion algorithm (allocation-conscious, single-pass):**

1. Allocate one heap buffer (`Vec<u8>` or similar) sized exactly `array_total_bytes` (not rounded to sector size) — read into a sector-sized scratch stage buffer per I/O call, then copy only the meaningful bytes into the final buffer, avoiding retaining padding.
2. Issue block reads in a loop across `array_sector_count` sectors starting at `Partition Entry LBA`, going through your disk cache layer (so repeated boot-time reads of small disks benefit from cache warm-up, and multi-sector reads can be coalesced by the cache/driver layer if it supports scatter-gather).
3. Once fully buffered, compute CRC32 over the **entire buffer** (all `array_total_bytes`, across all entries including any that are "unused/zeroed") and compare against the header's Partition Entry Array CRC32 (offset `0x58`). Mismatch ⇒ treat entire array as untrustworthy; fall back to backup array location (`Alternate LBA` header's own Partition Entry LBA) using the same procedure, or hard-fail.
4. Iterate the buffer in `entry_size`-strided windows (not hardcoded 128), for `0..entry_count`:
   - Slice out the 16-byte Type GUID at relative offset 0.
   - **Skip entirely** (do not allocate a `PartitionDevice` for) any entry whose Type GUID is all-zero (16 zero bytes) — this is the spec-defined "unused entry" marker.
   - Parse Starting LBA / Ending LBA via `u64::from_le_bytes`.
   - Validate `starting_lba <= ending_lba` and both fall within `[first_usable_lba, last_usable_lba]` using checked comparisons — reject/skip malformed entries individually rather than aborting the whole table (a single corrupt entry shouldn't invalidate the whole disk).
   - Emit a normalized in-memory descriptor (Type GUID, Unique GUID, LBA range, attribute flags, decoded name) into a growable `Vec<PartitionDescriptor>` — this becomes the seed list for Section 3's multiplexer.

### Phase 3: UTF-16 to ASCII Normalization for Framebuffer Display

Since your output path is a linear framebuffer text renderer (implying an ASCII/Latin-1 glyph set, not full Unicode), the 36-`u16` UTF-16LE partition name needs safe, lossy normalization:

1. **Slice extraction**: take the 72-byte name field, reinterpret as 36 little-endian `u16` code units via `u16::from_le_bytes` pairs (never a direct pointer cast, per Section 1.4 rules) — iterate in a fixed-size stack array `[u16; 36]`, no heap allocation needed for this staging step.
2. **Null-termination scan**: GPT names are null-padded, not necessarily null-terminated at a guaranteed position — scan for the first `0x0000` code unit; treat everything before it as the logical name length. If no null is found, the full 36 units are the name (spec allows this for max-length names).
3. **Decode UTF-16 → `char` iterator**: use a UTF-16 decoding algorithm that correctly handles surrogate pairs (high surrogate `0xD800..=0xDBFF` followed by low surrogate `0xDC00..=0xDFFF` combining into a code point above `0xFFFF`) even though in practice partition names are almost always in the Basic Multilingual Plane — correctness here avoids UB/panics on malformed or adversarial disk data, it does not need to render the character correctly.
4. **ASCII lossy transform for framebuffer**: for each decoded `char`, if it's in the printable ASCII range (`0x20..=0x7E`), pass through; otherwise substitute a placeholder glyph (e.g., `?` or `.`) — your framebuffer renderer presumably only has ASCII glyph bitmaps available.
5. **Allocate the final `String`** (heap-backed, using your existing `alloc` support) only once, sized to the final ASCII byte count — avoid repeated reallocation by first counting output length in a dry-run pass, or use `String::with_capacity(36)` as a safe upper bound (UTF-16 code units ≥ output ASCII chars, since surrogate pairs *reduce* count and non-ASCII substitutions are 1:1).
6. **Truncate/pad** for fixed-width framebuffer columns if your text renderer requires fixed-width cells (common for a monospace kernel console) — this is a rendering-layer concern, not a parser concern; keep it decoupled (the parser returns a `String`, the renderer decides truncation).

---

## 3. Partition Device Multiplexer Architecture

### 3.1 Conceptual Model

The multiplexer is a **virtualization shim** sitting between LemonFS (and any other future filesystem drivers) and your existing unified block-device abstraction. Each discovered GPT entry becomes one `PartitionDevice` instance implementing the *same trait/interface* your AHCI/NVMe layer already exposes to its callers — this is the key architectural principle: **partitions are just block devices with a translated address space and a restricted extent**, so nothing above this layer (disk cache, filesystem) needs to know partitions exist at all.

**Structural components:**

| Component | Responsibility |
|---|---|
| `PartitionTable` (per physical disk) | Owns the `Vec<PartitionDescriptor>` produced by Phase 2 parsing; exposes enumeration/lookup by index or GUID |
| `PartitionDevice` (per partition) | Implements the block-device trait; holds a reference/handle to the parent physical device, plus its own LBA offset + extent bounds |
| `PartitionRegistry` (global, kernel-wide) | Maps a stable partition identifier (e.g., disk index + partition index, or the Unique Partition GUID) to a live `PartitionDevice` handle, for lookup by mount code, syscall path resolution, etc. |

### 3.2 Thread-Safety & SMP Considerations

Given preemptive scheduling and multi-core execution, the multiplexer must guard against concurrent access from multiple cores simultaneously issuing reads/writes to different (or the same) partitions on the same physical disk:

1. **Immutable descriptor data** (Type GUID, Unique GUID, LBA bounds, name) should be treated as read-only after Phase 2 parsing completes — wrap the `PartitionTable`'s `Vec<PartitionDescriptor>` in an `Arc`-equivalent (your kernel's heap-backed reference-counted pointer) so multiple cores can hold cheap read access without locking, since the data never mutates post-discovery (barring a future "rescan/repartition" event, which should invalidate and rebuild the whole table atomically rather than mutating entries in place).
2. **The underlying physical block device** (AHCI/NVMe driver instance) already presumably has its own internal concurrency control (command queue locking, NVMe submission/completion queue pairs per core, etc., given your existing SMP-active architecture) — the `PartitionDevice` should **not** introduce a second independent lock around the same physical resource; it should be a thin, lock-free translation wrapper that forwards the *translated* LBA request into the existing thread-safe physical driver call path. Double-locking here risks priority inversion or unnecessary contention across cores hitting different partitions on the same disk.
3. **Per-partition sequencing concerns** (e.g., if LemonFS requires read-modify-write ordering guarantees for its own metadata) belong at the filesystem layer, not the multiplexer — the multiplexer's contract should be: "each translated I/O request is forwarded atomically and independently to the physical layer; ordering between requests is the caller's responsibility," matching standard block-device semantics.
4. Use `#[repr(align(64))]` on `PartitionDevice` (consistent with your existing `Task`/`Scheduler`/`CacheEntry` convention) if instances are frequently accessed/hot in per-core lookup paths, to avoid false-sharing when multiple cores dereference distinct `PartitionDevice` instances that happen to land on the same cache line.
5. **Registry lookups** (`PartitionRegistry`) will be accessed concurrently from syscall handlers on any core — back it with a reader-writer lock (readers vastly outnumber writers, since writes only happen at partition-discovery/rescan time) or, if your kernel has a lock-free concurrent map primitive, prefer that to avoid any per-I/O-request lock acquisition overhead on the hot path.

### 3.3 Address Translation Logic

Every I/O request arriving at a `PartitionDevice` carries a **virtual (partition-relative) LBA**, which must be translated to a **physical (disk-absolute) LBA** before forwarding to the underlying block-device driver.

**Translation algorithm (per request), mirroring your existing `checked_add` LBA validation convention:**

1. Given: `virtual_lba: u64` (request from filesystem layer), `virtual_block_count: u32` (number of contiguous blocks requested).
2. `partition_base_lba: u64` = this partition's Starting LBA (immutable, cached in the `PartitionDevice` struct at discovery time).
3. `partition_extent_blocks: u64` = `(ending_lba - starting_lba).checked_add(1)` (precomputed once at construction, not per-request, since it never changes).
4. **Bounds check (per request, before translation):** `virtual_lba.checked_add(virtual_block_count as u64)` must yield `Some(end)` where `end <= partition_extent_blocks`. Overflow on the checked_add, or `end > partition_extent_blocks`, ⇒ reject the request immediately with an out-of-bounds error — this must happen **before** any translation math, mirroring the same defensive posture as your existing block-device layer's LBA validation.
5. **Translation:** `physical_lba = partition_base_lba.checked_add(virtual_lba)` — this must also be checked (even though step 4 already bounds virtual_lba within the partition extent, defense-in-depth against future refactors is worth the negligible cost of one more checked_add).
6. Forward `(physical_lba, virtual_block_count)` to the underlying physical block-device driver's existing read/write entry point — the `PartitionDevice` layer does not touch the disk cache directly; it delegates to whatever layer your AHCI/NVMe/cache stack already exposes, preserving your existing caching behavior transparently for partition-relative I/O.
7. **Write-path additional invariant:** for write requests, re-validate the same bounds check independently (do not share a single "read-or-write" bounds-check code path with implicit trust) — an accidental cross-wiring bug here is a data-corruption-class bug, not just a read failure, so the extra defensive redundancy is warranted.

**Struct-shape guidance** (conceptual, not code) for `PartitionDevice`:
- `parent_device_handle`: reference/handle into your unified block-device abstraction (not a raw pointer — respect your existing no-raw-pointer discipline even internally).
- `base_lba: u64`, `extent_blocks: u64`: immutable, set once at discovery.
- `type_guid: [u8; 16]`, `unique_guid: [u8; 16]`: for identification/mount-matching (e.g., LemonFS driver scans the registry for its own reserved Type GUID).
- `sector_size: u32`: cached from the parent device at construction to avoid a re-query per I/O.
- No internal mutable state beyond what's needed for statistics/diagnostics (e.g., an atomic request counter, if desired) — keep it structurally as close to a "pure function of (base_lba, extent) + forward-call" as possible, which is what makes the lock-free design in 3.2 viable.

---

## 4. Technical References and Specification Sheets

### 4.1 Standard Partition Type GUIDs (canonical string form, for reference/documentation)

| Partition Role | Canonical GUID (string form) |
|---|---|
| Unused Entry | `00000000-0000-0000-0000-000000000000` |
| EFI System Partition (ESP) | `C12A7328-F81F-11D2-BA4B-00A0C93EC93B` |
| Linux Filesystem Data | `0FC63DAF-8483-4772-8E79-3D69D8477DE4` |
| Linux Swap | `0657FD6D-A4AB-43C4-84E5-0933C84B4F4F` |
| Linux Root (x86_64) | `4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709` |
| Microsoft Basic Data | `EBD0A0A2-B9E5-4433-87C0-68B6B72699C7` |
| Microsoft Reserved (MSR) | `E3C9E316-0B5C-4DB8-817D-F92DF00215AE` |
| BIOS Boot Partition (GRUB) | `21686148-6449-6E6F-744E-656564454649` |

### 4.2 Little-Endian Disk Encoding Reference (canonical string → raw disk bytes)

Using the ESP GUID as the worked example — canonical string `C12A7328-F81F-11D2-BA4B-00A0C93EC93B` decomposes as:

| Group | Rust Type | Canonical hex | On-disk byte order | Raw bytes (as they appear sequentially on disk) |
|---|---|---|---|---|
| `time_low` (4 bytes) | `u32` | `C12A7328` | reversed (LE) | `28 73 2A C1` |
| `time_mid` (2 bytes) | `u16` | `F81F` | reversed (LE) | `1F F8` |
| `time_hi_and_version` (2 bytes) | `u16` | `11D2` | reversed (LE) | `D2 11` |
| `clock_seq` (2 bytes) | `[u8; 2]` | `BA4B` | as-is | `BA 4B` |
| `node` (6 bytes) | `[u8; 6]` | `00A0C93EC93B` | as-is | `00 A0 C9 3E C9 3B` |

Full raw 16-byte sequence on disk for the ESP GUID:
`28 73 2A C1 1F F8 D2 11 BA 4B 00 A0 C9 3E C9 3B`

**Practical implication for your reference table implementation:** store your known-GUID constants (ESP, Linux, LemonFS) in this *raw disk-byte order* directly in kernel code, so that partition-type identification during Phase 2 ingestion is a straight byte-array equality check against the freshly-read 16-byte field — with zero runtime endian-swapping cost on the hot path. Reserve the canonical-string swapping logic exclusively for the framebuffer/diagnostic display path (Section 2, Phase 3 territory), where performance is irrelevant and human-readability is the goal.

### 4.3 Guidelines for a Custom LemonFS Type GUID

1. **Generate a single, permanent v4 (random) UUID** for LemonFS at design time — do this once, off-target (any standard UUID generator), and hardcode it as a constant; do not generate it at runtime/per-build, since it must remain stable across every LemonFS partition ever created for cross-tool recognition (your kernel, any future fsck-equivalent tool, any future GUI partition editor you build, etc. must all agree on the same constant).
2. **Reserve it formally in your own project documentation** the same way this reference table documents ESP/Linux — treat it as part of your on-disk format specification, versioned alongside LemonFS's own superblock format version field.
3. **Encode it in raw disk-byte order** (per 4.2's methodology) as the comparison constant used by Phase 2's per-entry Type GUID matching — this is what your partition discovery code checks against to decide "this entry belongs to LemonFS, hand it to the LemonFS driver/mounter."
4. **Do not reuse or alias** any existing registered GUID (Linux, Microsoft, etc.) even temporarily during development — GUID collision is exactly the failure mode the entire GPT spec exists to prevent; a random v4 UUID has a collision probability low enough to treat as effectively zero for a project-identifying constant.
5. **Consider a secondary "LemonFS variant/feature" distinction** (if you anticipate multiple incompatible on-disk LemonFS format versions in the future) via the Attribute Flags field's type-specific high bits (bits 48–63, offset 0x30 in the partition entry) rather than minting additional GUIDs — this keeps discovery logic simpler (one GUID to match) while still allowing your mounter to branch on format sub-version after the type match succeeds.

---

## Summary of Cross-Cutting Invariants (apply throughout all four sections)

- **All arithmetic on disk-derived offsets/counts uses `checked_*` operations** — consistent with your existing block-device LBA validation discipline; no raw `+`/`*` on untrusted header/entry fields anywhere in this subsystem.
- **No direct reference-casting of raw disk bytes to Rust structs** — byte-slice parsing or `read_unaligned`-via-raw-pointer only, per Section 1.4.
- **CRC32 and signature validation gate every trust boundary** before that data influences allocation sizes, address translation, or driver dispatch.
- **Corruption in one partition entry must not cascade** — skip and log individual malformed entries rather than aborting whole-disk discovery, but header-level corruption (Phase 1) *does* warrant full fallback-then-abort, since the header is the root of trust for everything downstream.
- **The multiplexer never bypasses your existing disk cache / block-device abstraction** — it is purely an address-translation and identity layer sitting above it, preserving every caching/validation guarantee your AHCI/NVMe stack already provides.
