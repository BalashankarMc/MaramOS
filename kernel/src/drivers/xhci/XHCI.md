# xHCI (USB 3.0) Driver Reference & Implementation Plan — Maram OS

Target: Rust `#![no_std]`, edition 2024, nightly, x86_64-unknown-none. Written against
xHCI spec rev 1.2 semantics, tailored to your `PCIFunction` / `KMemory` / `DMABuffer` /
`msix` / NVMe-style register-wrapper conventions.

---

## PART 1 — REGISTER REFERENCE

All multi-byte registers are little-endian. All addresses in pointer-fields are
physical addresses (the DMA-visible/HHDM-mapped physical addresses your `DMABuffer`
and `PhysPage` types return via `.phys()` / `.get_phys_address()`), **not** virtual
addresses — xHCI is a bus-mastering device and only understands physical memory.

### 1.1 PCI Discovery

| Field | Value |
|---|---|
| Class | `0x0C` (Serial Bus Controller) |
| Subclass | `0x03` (USB) |
| Prog IF | `0x30` (xHCI) |
| BAR0/BAR1 | Combine into a single 64-bit MMIO base (BAR0 low 32 bits, BAR1 high 32 bits; `BarInfo.is_64bit` should be `true`, `BarInfo.is_mmio` should be `true`) |

Required PCI setup before touching MMIO:
1. `dev.enable_bus_master()` — xHCI is a DMA master (rings, contexts, doorbells).
2. `dev.enable_mmio()` — enables the memory decode so BAR reads/writes work.
3. Optionally `dev.find_capability(0x11)` (MSI-X cap ID `0x11`) to confirm MSI-X support before relying on it; fall back to `find_capability(0x05)` (MSI, cap ID `0x05`) otherwise.

### 1.2 Capability Registers (MMIO base + 0x00)

These are **read-only**. `CAPLENGTH` tells you where the Operational register block starts.

| Offset | Name | Width | Description |
|---|---|---|---|
| 0x00 | CAPLENGTH | u8 | Length of capability register block; Operational base = MMIO base + CAPLENGTH |
| 0x01 | Reserved | u8 | — |
| 0x02 | HCIVERSION | u16 | BCD version, e.g. `0x0100` = 1.0.0, `0x0110` = 1.1.0 |
| 0x04 | HCSPARAMS1 | u32 | Structural Parameters 1 |
| 0x08 | HCSPARAMS2 | u32 | Structural Parameters 2 |
| 0x0C | HCSPARAMS3 | u32 | Structural Parameters 3 |
| 0x10 | HCCPARAMS1 | u32 | Capability Parameters 1 |
| 0x14 | DBOFF | u32 | Doorbell array offset from MMIO base |
| 0x18 | RTSOFF | u32 | Runtime register offset from MMIO base |
| 0x1C | HCCPARAMS2 | u32 | Capability Parameters 2 |

**HCSPARAMS1** (0x04):

| Bits | Field | Meaning |
|---|---|---|
| 7:0 | MaxSlots | Max number of Device Slots (Device Context Array entries) |
| 18:8 | MaxIntrs | Max number of Interrupters |
| 31:24 | MaxPorts | Number of root hub ports |

**HCSPARAMS2** (0x08):

| Bits | Field | Meaning |
|---|---|---|
| 3:0 | IST | Isochronous Scheduling Threshold |
| 7:4 | ERST Max | Max ERST entries = `2^ERST_Max` |
| 25:21 | Max Scratchpad Bufs Hi | High 5 bits of scratchpad buffer count |
| 26 | SPR | Scratchpad Restore — implementation detail, generally ignore |
| 31:27 | Max Scratchpad Bufs Lo | Low 5 bits of scratchpad buffer count |

Compute total scratchpad count as:
```text
max_scratchpad_bufs = (hcsparams2[25:21] << 5) | hcsparams2[31:27]
```

**HCSPARAMS3** (0x0C):

| Bits | Field | Meaning |
|---|---|---|
| 7:0 | U1 Device Exit Latency | Worst-case U1 exit latency (µs) |
| 31:16 | U2 Device Exit Latency | Worst-case U2 exit latency (µs) |

**HCCPARAMS1** (0x10) — **critical, read this before allocating any context memory**:

| Bits | Field | Meaning |
|---|---|---|
| 0 | AC64 | 1 = 64-bit addressing capable (pointer fields support full 64-bit) |
| 1 | BNC | Bandwidth Negotiation Capability |
| 2 | **CSZ** | **Context Size: 0 = 32-byte contexts, 1 = 64-byte contexts.** Every Slot Context, Endpoint Context, and Input Control Context is sized/strided by this bit. |
| 3 | PPC | Port Power Control capable |
| 4 | PIND | Port Indicators capable |
| 5 | LHRC | Light HC Reset Capable |
| 6 | LTC | Latency Tolerance Messaging Capable |
| 7 | NSS | No Secondary SID support |
| 8 | PAE | Parked Authorization not supported (Parse All Event Data) |
| 9 | SPC | Stopped - Short Packet capable |
| 10 | SEC | Stopped EDTLA Capable |
| 11 | CFC | Contiguous Frame ID Capable |
| 15:12 | MaxPSASize | Max Primary Stream Array Size = `2^(val+1)` if nonzero |
| 31:16 | xECP | Extended Capabilities Pointer, in **32-bit DWORDs** from MMIO base (i.e. byte offset = `xECP * 4`); `0` = none |

**DBOFF** (0x14): bits 1:0 reserved (always 0), bits 31:2 are the doorbell array
byte-offset from MMIO base divided by 4 — i.e. mask off the low 2 bits and use the
result directly as a byte offset (`dboff = read_u32(0x14) & !0x3`).

**RTSOFF** (0x18): bits 4:0 reserved, bits 31:5 are the runtime register byte-offset
from MMIO base — mask off the low 5 bits (`rtsoff = read_u32(0x18) & !0x1F`).

**HCCPARAMS2** (0x1C): U3C, CMC, FSC, CTC, LEC, CIC capability bits — informational,
not required for a baseline driver; safe to ignore initially.

### 1.3 Operational Registers (MMIO base + CAPLENGTH)

| Offset | Name | Width | R/W |
|---|---|---|---|
| 0x00 | USBCMD | u32 | R/W |
| 0x04 | USBSTS | u32 | R/WC (write-1-to-clear on change bits) |
| 0x08 | PAGESIZE | u32 | RO |
| 0x0C | Reserved | — | — |
| 0x14 | DNCTRL | u32 | R/W |
| 0x18 | CRCR | u64 | R/W (**reads back as 0**, see pitfall below) |
| 0x20 | Reserved | — | — |
| 0x30 | DCBAAP | u64 | R/W |
| 0x38 | CONFIG | u32 | R/W |

**USBCMD** (0x00):

| Bit | Field | Meaning |
|---|---|---|
| 0 | RS | Run/Stop. 1 = tell HC to run, 0 = stop after finishing current transactions |
| 1 | HCRST | Host Controller Reset. Write 1 to reset; self-clears when done |
| 2 | INTE | Interrupter Enable |
| 3 | HSEE | Host System Error Enable |
| 7 | LHCRST | Light Host Controller Reset (optional, check LHRC in HCCPARAMS1) |
| 8 | CSS | Controller Save State |
| 9 | CRS | Controller Restore State |
| 10 | EWE | Enable Wrap Event |
| 11 | EU3S | Enable U3 MFINDEX Stop |
| 13 | CME | CEM Enable |
| 14 | ETE | Extended TBC Enable |
| 15 | TSC_EN | TSC Enable |
| 16 | VTIOE | VTIO Enable |

**USBSTS** (0x04):

| Bit | Field | Meaning |
|---|---|---|
| 0 | HCH | HCHalted. 1 = HC is halted (not running). Set after RS=0 and all transactions complete, and after reset |
| 2 | HSE | Host System Error (fatal, e.g. bad memory access) |
| 3 | EINT | Event Interrupt — set when an event ring has a pending event and IE is set on that interrupter |
| 4 | PCD | Port Change Detect — some PORTSC change bit is set on some port |
| 8 | SSS | Save State Status |
| 9 | RSS | Restore State Status |
| 10 | SRE | Save/Restore Error |
| 11 | **CNR** | **Controller Not Ready.** Must be 0 before writing any operational register other than USBSTS itself, after reset or power-on |
| 12 | HCE | Host Controller Error (fatal, needs HCRST) |

**PAGESIZE** (0x08): bit `n` set means the controller supports a page size of
`2^(n+12)` bytes. In practice bit 0 (4 KiB pages) is essentially universal; verify
it's set and use 4 KiB pages throughout (matches `KMemory` page granularity).

**DNCTRL** (0x14): 16-bit mask (bits 15:0), bit N enables Device Notification of
type N. Leave 0 unless you need device notification events.

**CRCR** (0x18) — Command Ring Control Register, 64-bit:

| Bit(s) | Field | Meaning |
|---|---|---|
| 0 | RCS | Ring Cycle State — write the initial Producer Cycle State of the command ring (normally 1) |
| 1 | CS | Command Stop (write 1 to stop ring processing, self-clearing) |
| 2 | CA | Command Abort (write 1 to abort, self-clearing) |
| 3 | CRR | Command Ring Running (**read-only status bit**) |
| 5:4 | Reserved | — |
| 63:6 | Command Ring Pointer | Physical address of first TRB, **4 KiB aligned** (low 6 bits are 0 by construction since it's page-aligned in practice, but the field itself only requires 64-byte alignment) |

> **Pitfall:** reading CRCR back (the whole register or any of its bits) returns 0
> on real hardware/most emulation. Never rely on read-back to check the ring base or
> RCS — keep a shadow struct (`{ base: PhysAddr, cycle_state: bool }`) in your driver
> state and mutate it as you enqueue/dequeue, mirroring writes to CRCR only at setup
> time and treating CRR as the only legitimate thing to poll.

**DCBAAP** (0x30) — Device Context Base Address Array Pointer, 64-bit:

| Bit(s) | Field |
|---|---|
| 5:0 | Reserved (must be 0 — 64-byte aligned) |
| 63:6 | Physical address of the Device Context Base Address Array |

The array itself is `(MaxSlots + 1)` **64-bit** pointers (entry 0 is the Scratchpad
Buffer Array pointer if scratchpad buffers are used, or 0 if not; entries 1..=MaxSlots
point to each slot's Device Context once allocated).

**CONFIG** (0x38):

| Bits | Field | Meaning |
|---|---|---|
| 7:0 | MaxSlotsEn | Number of Device Slots enabled — write ≤ MaxSlots from HCSPARAMS1 |
| 8 | U3E | U3 Entry Enable |
| 9 | CIE | Configuration Information Enable |

### 1.4 Port Registers (Operational base + 0x400, one 16-byte block per port)

Ports are 1-indexed (Port 1..=MaxPorts). Port `n`'s block starts at
`op_base + 0x400 + (n - 1) * 0x10`.

| Offset | Name | Description |
|---|---|---|
| 0x00 | PORTSC | Port Status and Control |
| 0x04 | PORTPMSC | Port Power Management Status/Control |
| 0x08 | PORTLI | Port Link Info |
| 0x0C | Reserved / Port Hardware LPM Control (version-dependent) | — |

**PORTSC** (0x00) — 32-bit, mixed RO/RW/RW1C:

| Bit(s) | Field | Type | Meaning |
|---|---|---|---|
| 0 | CCS | RO | Current Connect Status — 1 = device present |
| 1 | PED | RW1C | Port Enabled/Disabled |
| 2 | RsvdZ | — | — |
| 3 | OCA | RO | Over-current Active |
| 4 | PR | RW | Port Reset — write 1 to reset; self-clears, watch PRC |
| 7:4 is actually a single field per-spec table below | | | |
| 8:5 | PLS | RW/RO | Port Link State (see table) |
| 9 | PP | RW | Port Power |
| 13:10 | Port Speed | RO | 1=Full,2=Low,3=High,4=SuperSpeed,5=SuperSpeedPlus (0=undefined until CCS) |
| 15:14 | PIC | RW | Port Indicator Control |
| 16 | LWS | RW | Port Link State Write Strobe |
| 17 | CSC | RW1C | Connect Status Change |
| 18 | PEC | RW1C | Port Enabled/Disabled Change |
| 19 | WRC | RW1C | Warm Port Reset Change (USB3 only) |
| 20 | OCC | RW1C | Over-current Change |
| 21 | PRC | RW1C | Port Reset Change |
| 22 | PLC | RW1C | Port Link State Change |
| 23 | CEC | RW1C | Port Config Error Change |
| 24 | CAS | RO | Cold Attach Status |
| 25 | WCE | RW | Wake on Connect Enable |
| 26 | WDE | RW | Wake on Disconnect Enable |
| 27 | WOE | RW | Wake on Over-current Enable |
| 28:29 | RsvdZ | — | — |
| 30 | DR | RO | Device Removable |
| 31 | WPR | RW | Warm Port Reset (USB3 only) |

> Note: bit widths above reflect the canonical layout; some references show PR as a
> single bit (4) and PLS as bits 8:5 — treat PR(4) and PLS(8:5) as adjacent
> independent fields, not overlapping.

Common PLS values: `0` = U0, `2` = U2, `3` = U3 (suspended), `5` = RxDetect,
`15` = Resume. For USB2 ports interpret differently (Disabled/Enabled/Suspended/Reset
etc.) — for a first driver you mainly need to detect **CCS=1** (something is plugged
in) and, after reset, **PED=1 + PLS=U0** (device is operational).

Writing to PORTSC: **read-modify-write carefully.** The RW1C bits (CSC, PEC, WRC, OCC,
PRC, PLC, CEC) clear themselves when you write 1; writing 0 to them is a no-op. If you
naively write back a value you read with those bits set, you'll inadvertently clear
change flags you haven't processed yet — always mask to only the bits you intend to
change (e.g. to trigger a reset: read PORTSC, clear all RW1C bits in your local copy,
set PR, write back).

**PORTPMSC** (0x04): layout differs for USB2 vs USB3 ports (L1 timeout / BESL fields
vs U1/U2 timeout fields). Not required for baseline enumeration; leave at
power-on defaults initially.

**PORTLI** (0x08), USB3 only:

| Bits | Field |
|---|---|
| 15:0 | Link Error Count |
| 19:16 | Rx Lane Count |
| 23:20 | Tx Lane Count |

### 1.5 Runtime Registers (MMIO base + RTSOFF)

| Offset | Name |
|---|---|
| 0x00 | MFINDEX |
| 0x20 + 32*n | Interrupter Register Set `n` (n = 0..MaxIntrs-1) |

**MFINDEX** (0x00): bits 13:0 = current microframe index (125 µs each), free-running.
Not required for baseline driver.

**Interrupter Register Set** (base `0x20 + 32*n`):

| Offset | Name | Width |
|---|---|---|
| 0x00 | IMAN | u32 |
| 0x04 | IMOD | u32 |
| 0x08 | ERSTSZ | u32 |
| 0x0C | Reserved | — |
| 0x10 | ERSTBA | u64 |
| 0x18 | ERDP | u64 |

**IMAN** (0x00):

| Bit | Field | Meaning |
|---|---|---|
| 0 | IP | Interrupt Pending — RW1C, set by HC when an event is queued, write 1 to clear |
| 1 | IE | Interrupt Enable |

**IMOD** (0x04):

| Bits | Field | Meaning |
|---|---|---|
| 15:0 | IMODI | Interval, in 250 ns units, min time between interrupts (0 = no throttling) |
| 31:16 | IMODC | Counter, current countdown value |

**ERSTSZ** (0x08): bits 15:0 = number of entries in the Event Ring Segment Table
(must be ≤ `2^ERST_Max` from HCSPARAMS2). Set **before** ERSTBA for a clean init.

**ERSTBA** (0x10), 64-bit: bits 5:0 reserved (64-byte aligned), bits 63:6 = physical
address of the Event Ring Segment Table.

**ERDP** (0x18), 64-bit — Event Ring Dequeue Pointer:

| Bit(s) | Field | Meaning |
|---|---|---|
| 2:0 | DESI | Dequeue ERST Segment Index — which segment the dequeue pointer is in |
| 3 | EHB | Event Handler Busy — RW1C, HC sets when it queues an event while EHB was already set (i.e. "you're behind"); software clears by writing 1 after updating the pointer |
| 63:4 | Event Ring Dequeue Pointer | Physical address of next TRB software will read, **16-byte aligned** |

### 1.6 Doorbell Registers (MMIO base + DBOFF)

Array of `MaxSlots + 1` doorbells, 32-bit each, 4-byte stride, **write-only**
(reads are undefined — don't rely on them):

| Doorbell index | Meaning |
|---|---|
| 0 | Host Controller / Command Ring doorbell |
| 1..=MaxSlots | Device Slot doorbells |

Each doorbell write:

| Bits | Field | Meaning |
|---|---|---|
| 7:0 | DB Target | For doorbell 0: must be 0 (command ring). For slot doorbells: endpoint DCI (Device Context Index) 1-31 to ring that endpoint's transfer ring |
| 15:8 | Reserved | 0 |
| 31:16 | DB Stream ID | Stream ID if the endpoint uses streams, else 0 |

Ringing doorbell 0 with target 0 tells the HC "new Command TRB(s) are on the command
ring." Ringing slot doorbell `s` with target = DCI tells the HC "new Transfer TRB(s)
are on endpoint DCI's transfer ring for slot s."

Endpoint DCI numbering: `DCI = (endpoint_number * 2) + direction`, where
`direction = 0` for OUT/control and `1` for IN — except EP0 (control) which is always
DCI 1. So: EP0=1, EP1 OUT=2, EP1 IN=3, EP2 OUT=4, EP2 IN=5, etc.

---

## PART 2 — DATA STRUCTURES

### 2.1 TRBs (Transfer Request Blocks) — 16 bytes, always

Generic layout (all TRB types share this shape):

```text
Offset 0x0: Parameter   (u64) — meaning depends on TRB type
Offset 0x8: Status      (u32) — meaning depends on TRB type
Offset 0xC: Control     (u32) — Cycle bit + TRB Type + type-specific flags
```

Control dword (0xC), common to every TRB type:

| Bits | Field | Meaning |
|---|---|---|
| 0 | C | Cycle bit — must match the ring's current Producer/Consumer Cycle State for the TRB to be valid |
| 15:10 | TRB Type | Selects interpretation of Parameter/Status/rest of Control |
| others | type-specific | ENT, CH, IOC, etc. depending on type (below) |

**TRB Type codes** (bits 15:10 of Control):

| Value | Name | Category |
|---|---|---|
| 1 | Normal | Transfer |
| 2 | Setup Stage | Transfer |
| 3 | Data Stage | Transfer |
| 4 | Status Stage | Transfer |
| 5 | Isoch | Transfer |
| 6 | Link | Transfer/Command (ring-management) |
| 7 | Event Data | Transfer |
| 8 | No Op | Transfer |
| 9 | Enable Slot Command | Command |
| 10 | Disable Slot Command | Command |
| 11 | Address Device Command | Command |
| 12 | Configure Endpoint Command | Command |
| 13 | Evaluate Context Command | Command |
| 14 | Reset Endpoint Command | Command |
| 15 | Stop Endpoint Command | Command |
| 16 | Set TR Dequeue Pointer Command | Command |
| 17 | Reset Device Command | Command |
| 18 | Force Event Command | Command |
| 19 | Negotiate Bandwidth Command | Command |
| 20 | Set Latency Tolerance Value Command | Command |
| 21 | Get Port Bandwidth Command | Command |
| 22 | Force Header Command | Command |
| 23 | No Op Command | Command |
| 24 | Get Extended Property Command | Command |
| 25 | Set Extended Property Command | Command |
| 32 | Transfer Event | Event |
| 33 | Command Completion Event | Event |
| 34 | Port Status Change Event | Event |
| 35 | Bandwidth Request Event | Event |
| 36 | Doorbell Event | Event |
| 37 | Host Controller Event | Event |
| 38 | Device Notification Event | Event |
| 39 | MFINDEX Wrap Event | Event |

#### Normal TRB (Type 1) — bulk/interrupt data stages

| Field | Location | Meaning |
|---|---|---|
| Data Buffer Pointer | Parameter[63:0] | Physical address of data buffer |
| TRB Transfer Length | Status[16:0] | Bytes to transfer (max 0x1FFFF) |
| TD Size | Status[21:17] | Number of packets remaining in this TD, capped at 31 |
| Interrupter Target | Status[31:22] | Which interrupter gets the completion event |
| C | Control[0] | Cycle bit |
| ENT | Control[1] | Evaluate Next TRB |
| ISP | Control[2] | Interrupt on Short Packet |
| NS | Control[3] | No Snoop |
| CH | Control[4] | Chain bit — links to next TRB as one TD |
| IOC | Control[5] | Interrupt On Completion |
| IDT | Control[6] | Immediate Data — Parameter holds data directly (≤8 bytes) instead of a pointer |
| BEI | Control[9] | Block Event Interrupt |
| TRB Type | Control[15:10] | `1` |

#### Setup Stage TRB (Type 2) — control transfers, stage 1

| Field | Location | Meaning |
|---|---|---|
| bmRequestType | Parameter[7:0] | USB setup packet byte 0 |
| bRequest | Parameter[15:8] | USB setup packet byte 1 |
| wValue | Parameter[31:16] | USB setup packet bytes 2-3 |
| wIndex | Parameter[47:32] | USB setup packet bytes 4-5 |
| wLength | Parameter[63:48] | USB setup packet bytes 6-7 |
| TRB Transfer Length | Status[16:0] | Always `8` (setup packet is 8 bytes) |
| Interrupter Target | Status[31:22] | Interrupter index |
| C | Control[0] | Cycle bit |
| IDT | Control[6] | Must be **1** — Parameter is immediate data, not a pointer |
| TRT | Control[17:16] | Transfer Type: 0=No Data, 2=OUT Data, 3=IN Data |
| TRB Type | Control[15:10] | `2` |

The full 8-byte little-endian USB setup packet is packed directly into Parameter —
construct it exactly as the USB spec's `SETUP` packet layout and write it as a raw
`u64`.

#### Data Stage TRB (Type 3) — control transfers, optional stage 2

| Field | Location | Meaning |
|---|---|---|
| Data Buffer Pointer | Parameter[63:0] | Physical address of data buffer |
| TRB Transfer Length | Status[16:0] | Bytes to transfer |
| TD Size | Status[21:17] | Remaining packet count |
| Interrupter Target | Status[31:22] | Interrupter index |
| C | Control[0] | Cycle bit |
| ENT/ISP/NS/CH/IOC/IDT | Control | Same semantics as Normal TRB |
| DIR | Control[16] | 0 = OUT, 1 = IN |
| TRB Type | Control[15:10] | `3` |

#### Status Stage TRB (Type 4) — control transfers, final stage

| Field | Location | Meaning |
|---|---|---|
| Parameter | — | Reserved, 0 |
| Interrupter Target | Status[31:22] | Interrupter index |
| C | Control[0] | Cycle bit |
| ENT | Control[1] | Evaluate Next TRB |
| CH | Control[4] | Chain bit |
| IOC | Control[5] | Interrupt On Completion (typically set here to know when the control transfer finished) |
| DIR | Control[16] | Direction opposite of the Data stage (0=OUT if data was IN, etc.); for No-Data transfers, 1 (IN) by convention |
| TRB Type | Control[15:10] | `4` |

#### Link TRB (Type 6) — ring-wraparound, used on **every** ring (command, transfer, event-adjacent)

| Field | Location | Meaning |
|---|---|---|
| Ring Segment Pointer | Parameter[63:4] | Physical address of the segment to jump to (typically back to segment start) |
| Interrupter Target | Status[31:22] | Usually 0 |
| C | Control[0] | Cycle bit |
| TC | Control[1] | Toggle Cycle — if set, software's Producer Cycle State flips after processing this TRB |
| CH | Control[4] | Chain bit, rarely used on Link |
| IOC | Control[5] | Interrupt on Completion |
| TRB Type | Control[15:10] | `6` |

Every ring you build (command ring, each transfer ring) needs its **last slot**
occupied by a Link TRB with TC=1 pointing back to slot 0, so the ring is circular. The
Event Ring does **not** use Link TRBs directly on the hardware-facing side in the
simple 1-segment case — wraparound is handled by the ERST + cycle-bit convention
described in §2.5.

#### Event TRBs (produced by hardware, Type 32/33/34, read-only from software's side)

**Transfer Event** (Type 32):

| Field | Location | Meaning |
|---|---|---|
| TRB Pointer | Parameter[63:4] | Physical address of the Transfer TRB that generated this event |
| TRB Transfer Length | Status[23:0] | Bytes **not** transferred (residual) on error, or actual length transferred |
| Completion Code | Status[31:24] | 1=Success, 13=Short Packet, 3=Data Buffer Error, 5=Babble, 6=USB Transaction Error, 7=TRB Error, 21=Stall Error, others — see §2.1.1 |
| C | Control[0] | Cycle bit |
| ED | Control[2] | Event Data — TRB Pointer refers to an Event Data TRB, not a real Transfer TRB |
| Endpoint ID | Control[20:16] | DCI of the endpoint |
| Slot ID | Control[31:24] | Device slot that generated this |
| TRB Type | Control[15:10] | `32` |

**Command Completion Event** (Type 33):

| Field | Location | Meaning |
|---|---|---|
| Command TRB Pointer | Parameter[63:4] | Physical address of the Command TRB completed |
| Command Completion Parameter | Status[23:0] | Type-specific (e.g. new Slot ID isn't here — see below) |
| Completion Code | Status[31:24] | Same code space as Transfer Event |
| C | Control[0] | Cycle bit |
| VF ID | Control[23:16] | Virtualization, usually 0 |
| Slot ID | Control[31:24] | For Enable Slot Command completion, this **is** the newly assigned Slot ID |
| TRB Type | Control[15:10] | `33` |

**Port Status Change Event** (Type 34):

| Field | Location | Meaning |
|---|---|---|
| Port ID | Parameter[31:24] | 1-indexed port number that changed |
| Completion Code | Status[31:24] | Usually 1 (Success) |
| C | Control[0] | Cycle bit |
| TRB Type | Control[15:10] | `34` |

On receiving this event, go re-read that port's **PORTSC** to see what actually
changed (CSC/PEC/PRC/PLC/etc.) — the event itself doesn't tell you which bit changed.

##### 2.1.1 Completion codes worth handling explicitly

| Code | Name | Typical handling |
|---|---|---|
| 1 | Success | Proceed |
| 3 | Data Buffer Error | Log, treat as failed transfer |
| 5 | Babble Detected | Device misbehaved, may need Reset Endpoint |
| 6 | USB Transaction Error | Retry policy or fail up to caller |
| 7 | TRB Error | Driver bug — malformed TRB, fix construction |
| 11 | Resource Error | HC out of internal resources — back off |
| 13 | Short Packet | Not necessarily fatal — check TRB Transfer Length for actual bytes moved |
| 21 | Stall Error | Endpoint stalled — needs Reset Endpoint Command then clear STALL via control transfer |
| 24 | Command Ring Stopped | Response to a Stop/Abort on the command ring |
| 192 | Ring Underrun / Overrun | Isochronous only |

### 2.2 Device Context — Slot Context + Endpoint Contexts

The Device Context is allocated **per slot**, once as part of Address Device (as the
target of that slot's DCBAA entry). Its size is `(1 + 31) * context_stride` where
`context_stride` is 32 or 64 bytes depending on **HCCPARAMS1.CSZ**. Only as many
Endpoint Contexts as you've configured actually matter, but the whole block should be
allocated and zeroed up front (`32 * context_stride` for slot+ep0 minimum, full
`32 * context_stride` covers slot + all 31 possible endpoints).

> **Pitfall:** if CSZ=1, *every* context-sized structure (Slot Context, each Endpoint
> Context, and the Input Control Context) is 64 bytes, not 32, and array strides
> throughout the Device Context / Input Context must use 64-byte spacing. Getting
> this wrong silently corrupts the layout the HC reads. Always compute
> `let ctx_size: usize = if hccparams1 & (1 << 2) != 0 { 64 } else { 32 };` once at
> init and thread it through every context calculation.

**Slot Context** (first `ctx_size` bytes of the Device Context, and of the Input
Context's context area):

| Offset | Field | Bits | Meaning |
|---|---|---|---|
| 0x00 dword0 | Route String | 19:0 | Hub routing string (0 for a device directly on a root port) |
| | Speed | 23:20 | Deprecated alias of Port Speed from PORTSC — 1=Full,2=Low,3=High,4=Super |
| | Reserved | 24 | — |
| | MTT | 25 | Multi-TT |
| | Hub | 26 | 1 if this device is a USB hub |
| | Context Entries | 31:27 | Index of last valid Endpoint Context + 1 (e.g. 1 if only EP0 configured) |
| 0x04 dword1 | Max Exit Latency | 15:0 | µs |
| | Root Hub Port Number | 23:16 | 1-indexed port this device is downstream of (top-level) |
| | Number of Ports | 31:24 | Nonzero only if this is a hub |
| 0x08 dword2 | Parent Hub Slot ID | 7:0 | 0 if directly on a root port |
| | Parent Port Number | 15:8 | Port on parent hub, if applicable |
| | TTT | 17:16 | TT Think Time |
| | Reserved | 21:18 | — |
| | Interrupter Target | 31:22 | Which interrupter gets Port Status Change events for this device |
| 0x0C dword3 | USB Device Address | 7:0 | HC-assigned address (matches SET_ADDRESS); read-only from software after Address Device |
| | Reserved | 26:8 | — |
| | Slot State | 31:27 | 0=Disabled/Enabled, 1=Default, 2=Addressed, 3=Configured |
| 0x10-0x1F | Reserved | — | Zero |

**Endpoint Context** (each `ctx_size` bytes, at offset `ctx_size * DCI` within the
Device Context, where DCI is the endpoint's Device Context Index — so Endpoint
Context for DCI=1 (EP0) starts at byte offset `ctx_size` from the start of the Device
Context, i.e. right after the Slot Context):

| Offset | Field | Bits | Meaning |
|---|---|---|---|
| 0x00 dword0 | EP State | 2:0 | 0=Disabled,1=Running,2=Halted,3=Stopped,4=Error |
| | Reserved | 7:3 | — |
| | Mult | 9:8 | Isoch/interrupt max burst multiplier - 1 |
| | MaxPStreams | 14:10 | log2 of max primary stream array size, 0 if no streams |
| | LSA | 15 | Linear Stream Array |
| | Interval | 23:16 | Polling interval, `2^(interval)` * 125 µs |
| | Max ESIT Payload Hi | 31:24 | High 8 bits of ESIT payload |
| 0x04 dword1 | Reserved | 1:0 | — |
| | CErr | 2:1 (actually 2 bits at 2:1... see note) | Error Count — retries before EP halts |
| | EP Type | 5:3 | 0=Not Valid,1=Isoch Out,2=Bulk Out,3=Interrupt Out,4=Control Bidir,5=Isoch In,6=Bulk In,7=Interrupt In |
| | HID | 7 | Host Initiate Disable |
| | Max Burst Size | 15:8 | USB3 burst size - 1 |
| | Max Packet Size | 31:16 | Max packet size in bytes for this EP |
| 0x08 dword2 | DCS | 0 | Dequeue Cycle State — initial cycle bit for this endpoint's transfer ring |
| | Reserved | 3:1 | — |
| | TR Dequeue Pointer Lo | 31:4 | Low bits of transfer ring physical address (16-byte aligned) |
| 0x0C dword3 | TR Dequeue Pointer Hi | 31:0 | High 32 bits of transfer ring physical address |
| 0x10 dword4 | Average TRB Length | 15:0 | Software hint to HC for bandwidth scheduling |
| | Max ESIT Payload Lo | 31:16 | Low 16 bits of ESIT payload |
| 0x14-0x1F | Reserved | — | Zero |

> Note on CErr: it's actually bits 2:1 giving values 0-3 (typical value: 3). Bit 0 of
> dword1 is reserved.

For **EP0 (control)** specifically: EP Type = 4 (Control Bidir), Max Packet Size
depends on device speed (8 for Low Speed, 8/16/32/64 for Full Speed per descriptor,
64 for High Speed, 512 for SuperSpeed+), CErr = 3, Interval = 0.

### 2.3 Input Context — used to configure/modify Device Context via commands

Layout: Input Control Context (`ctx_size` bytes) followed by a Slot Context and up to
31 Endpoint Contexts, in the *same* layout as the Device Context described above. Total
size = `32 * ctx_size` (1 control + 1 slot + 30... conventionally allocate for all 31
endpoint slots to keep math simple: `(1 + 1 + 31) * ctx_size`, or just reuse the same
`32 * ctx_size` allocation size as the Device Context plus one leading control block).

**Input Control Context** (first `ctx_size` bytes of the Input Context):

| Offset | Field | Bits | Meaning |
|---|---|---|---|
| 0x00 dword0 | Drop Context Flags D0-D31 | 31:0 | D0, D1 reserved (must be 0); Dn (n≥2) = 1 means "remove Endpoint Context n from device" on Configure Endpoint |
| 0x04 dword1 | Add Context Flags A0-A31 | 31:0 | A0 = 1 means apply Slot Context; A1 = 1 means apply EP0 context; An (n≥2) = 1 means apply Endpoint Context n |
| 0x08-0x1C | Reserved | — | Zero, **except**: |
| 0x1C dword7 | Configuration Value | 7:0 | From the device's chosen USB configuration descriptor |
| | Interface Number | 15:8 | From the interface descriptor |
| | Alternate Setting | 23:16 | From the interface descriptor |
| | Reserved | 31:24 | — |

Usage pattern:
- **Address Device Command**: set A0=1 (Slot Context) and A1=1 (EP0 Context) in the
  Input Control Context, fill in Slot Context (route string, root port, speed) and
  EP0's Endpoint Context (max packet size guess based on speed, TR Dequeue Pointer =
  EP0's freshly-allocated transfer ring, DCS=1), leave all other flags 0.
- **Configure Endpoint Command**: set A0=1 plus An=1 for each endpoint you're adding,
  Dn=1 for each you're removing; fill in the corresponding Endpoint Contexts (transfer
  ring pointers, packet sizes, intervals from the endpoint descriptors) and update
  Slot Context's Context Entries field to the highest DCI in use.
- **Evaluate Context Command**: similar, used for narrower updates (e.g. Max Packet
  Size correction after reading the real device descriptor).

### 2.4 Command Ring

A single circular array of TRBs (Command TRBs) that software produces and the HC
consumes.

- **Allocation**: `DMABuffer::new(n * 16)` for `n` TRBs — 16–64 is a reasonable size
  for a hobby OS (more than enough outstanding commands at once). Must be **64-byte
  aligned** minimum (page-aligned via `DMABuffer` easily satisfies this) and must not
  cross a 64 KB boundary if you ever grow it, though a single 4 KiB `DMABuffer`
  segment is safe by construction.
- **Last TRB** in the segment must be a **Link TRB** (Type 6) with Ring Segment
  Pointer = physical address of TRB[0], and **TC=1** (Toggle Cycle) so that after the
  HC processes it, software's Producer Cycle State flips for the next lap.
- **Cycle state**: start with Producer Cycle State (PCS) = 1. Every TRB you write has
  its Cycle bit (Control[0]) set to the *current* PCS value **before** you consider
  it "committed" (the HC must not observe a half-written TRB with the cycle bit
  already flipped — write Parameter+Status first, then Control with the cycle bit
  last, using a proper memory barrier / volatile write ordering).
- **Enqueue pointer**: software-maintained index of the next free slot. After writing
  a TRB, advance the enqueue pointer; if it lands on the Link TRB, write the Link
  TRB's cycle bit to match PCS as well, advance to slot 0, and flip PCS.
- **Producing a command**: write the Command TRB (e.g. Enable Slot Command, Address
  Device Command, Configure Endpoint Command) at the enqueue pointer, then ring
  **Doorbell 0** with target 0.
- **Dequeue pointer**: the HC maintains its own internal consumer pointer; software
  never manages this directly for the command ring — completions arrive as **Command
  Completion Events** on the Event Ring, each one referencing the Command TRB's
  physical address via the event's TRB Pointer field, which is how you match a
  completion back to the command you issued (compare pointers, or keep an in-order
  queue since you're only issuing one command at a time in a simple driver).
- **CRCR write** (setup only): physical address of TRB[0] with the low 6 bits masked
  and OR'd with RCS=1 (the initial PCS).

### 2.5 Transfer Ring

Same ring mechanics as the Command Ring (circular buffer of 16-byte TRBs, Link TRB at
the end with TC=1, cycle-bit-based validity, doorbell-based notification), but:

- **One per endpoint** (referenced by that endpoint's Endpoint Context TR Dequeue
  Pointer + DCS).
- TRB types used are Normal/Setup Stage/Data Stage/Status Stage/Isoch instead of
  Command TRBs.
- Notification is via the **slot's doorbell** (index = Slot ID) with DB Target = the
  endpoint's DCI, instead of doorbell 0.
- A **Transfer Descriptor (TD)** is one or more TRBs linked via the **Chain (CH)**
  bit — e.g. a multi-stage control transfer's Setup+Data+Status TRBs are technically
  three separate TDs (Setup is always its own TD), while a large bulk transfer split
  across multiple Normal TRBs due to buffer discontiguity is one TD with CH=1 on all
  but the last TRB.
- Set **IOC** (Interrupt On Completion) on the last TRB of a TD you care about
  completion for, so a Transfer Event is generated.

### 2.6 Event Ring

Unlike the Command/Transfer rings, the Event Ring is **HC-produced, software-consumed**,
and uses a segment table (ERST) rather than a single flat buffer with a Link TRB.

Structures needed:
1. **Event Ring Segment(s)**: one or more `DMABuffer`s of `m * 16` bytes each (`m`
   TRBs per segment; 16–64 is fine for a first implementation using a single segment).
2. **Event Ring Segment Table (ERST)**: an array of ERST entries, one per segment.
   Each entry is 16 bytes:

   | Offset | Field | Meaning |
   |---|---|---|
   | 0x00 | Ring Segment Base Address | Physical address of the segment, bits 63:6 (6:0 reserved, must be 0 — segment must be 64-byte aligned) |
   | 0x08 | Ring Segment Size | bits 15:0 = number of TRB slots in this segment (16-4096); rest reserved |

   For a single-segment setup: allocate 1 `DMABuffer` for the ERST (16 bytes is
   enough for 1 entry, but page-align it since `DMABuffer` rounds up anyway),
   write one entry pointing at your event-ring segment.
3. **Consumer Cycle State (CCS)**: software starts expecting CCS = 1 for the first
   lap of the event ring. An event TRB is valid (has actually been written by
   hardware, vs. being stale/zeroed) exactly when its Cycle bit equals the current
   CCS. After consuming the last TRB of a segment, **flip CCS** and wrap the
   dequeue pointer back to the first TRB of the (single) segment — the toggle
   happens implicitly at the segment boundary; there's no Link TRB in the event ring.
4. **ERDP**: after processing events, write ERDP = physical address of the new
   dequeue position (with DESI updated if multi-segment) and set bit 3 (EHB) to 1 in
   the same write to acknowledge you've caught up — this clears EHB if the HC had
   set it.

Consumption loop:
```text
loop {
    let trb = read TRB at dequeue_ptr;
    if trb.cycle_bit != current_ccs { break; }  // no more events ready
    process(trb);
    dequeue_ptr += 16;
    if dequeue_ptr reaches end of current segment {
        dequeue_ptr = start of next segment (or same segment if only 1);
        if wrapped past last segment { current_ccs = !current_ccs; }
    }
}
write ERDP = dequeue_ptr | EHB(1);
```

> **Pitfall:** forgetting to toggle CCS on wraparound is the single most common xHCI
> bug — after the first lap through the event ring, every subsequent event will
> appear to have the "wrong" cycle bit and your driver will silently stop seeing
> events, looking like a hung controller.

### 2.7 Scratchpad Buffers

Some controllers require scratchpad memory for internal use, sized by
`max_scratchpad_bufs` computed from HCSPARAMS2 (§1.2). If `max_scratchpad_bufs == 0`,
skip this entirely and set DCBAA entry 0 to 0.

If nonzero:
1. Allocate `max_scratchpad_bufs` individual pages (`KMemory::alloc_pages(1)` each, or
   one contiguous `DMABuffer::new(max_scratchpad_bufs * 4096)` sliced into pages — either
   works since the HC only needs each buffer to itself be **page-aligned and
   page-sized** (4 KiB, matching PAGESIZE register)).
2. Allocate a **Scratchpad Buffer Array**: `DMABuffer::new(max_scratchpad_bufs * 8)` —
   an array of 64-bit physical pointers, one per scratchpad page, **64-byte aligned**
   overall (page alignment from `DMABuffer` easily covers this).
3. Write each scratchpad page's physical address into the Scratchpad Buffer Array.
4. Write the Scratchpad Buffer Array's physical address into **DCBAA entry 0** (the
   slot-0 slot which is otherwise unused, repurposed specifically for this pointer).

---

## PART 3 — ORDERED IMPLEMENTATION PLAN

General conventions used throughout: `regs` is your `XhciRegisters` wrapper over
Operational-space-relative offsets (mirroring `NVMeRegisters`); capability-space and
runtime-space reads go through small helper offsets computed once at init and stored
in the driver struct (`cap_base`, `op_base`, `rt_base`, `db_base` as `VirtAddr`, plus
`dboff`/`rtsoff` as byte offsets). Every busy-wait uses:

```rust
let start = time();
loop {
    if condition { break; }
    if time() - start > TIMEOUT {
        return Err(IOError::Timeout);
    }
    core::hint::spin_loop();
}
```

### Step 0 — Kernel plumbing

**Do:**
- `drivers/pci/mod.rs`: add `Xhci` to `DeviceType`.
- `PCIFunction::get_type()`: add `0x0C => match self.subclass { 0x03 => DeviceType::Xhci, _ => DeviceType::Unknown }`.
- `drivers/storage/mod.rs`'s `init_drive()`: since xHCI is not block storage, either
  skip the match arm there entirely and dispatch xHCI from wherever PCI enumeration
  calls out to non-storage drivers, or add an explicit early-return arm with a comment
  noting it doesn't implement `StorageDrive` and is handled by `drivers::usb::xhci::init()`
  instead.
- Create `drivers/usb/xhci/` with `mod.rs`, `registers.rs`, `context.rs`, `rings.rs`,
  `command.rs`, `enumeration.rs`, `transfer.rs` — mirroring the NVMe module split
  (`mod.rs`/`registers.rs`/`queues.rs`/`admin.rs`/`io.rs`).
- `descriptors/interrupts.rs`: add `XhciIO = 34` to `HardwareInterrupts`.

**Verify:** crate compiles with the new module stubs; `DeviceType::Xhci` is reachable
from PCI enumeration logs (print it when a matching function is found, confirm you
see it during boot on real/emulated hardware with an xHCI controller present).

**Pitfalls:** none yet — this step is pure scaffolding, but get the module layout
right now since every later step assumes it.

---

### Step 1 — PCI discovery + MMIO BAR mapping

**Do:**
```rust
let bar0 = dev.bar(0).ok_or(IOError::InitFailed)?;
let bar1 = dev.bar(1).ok_or(IOError::InitFailed)?;
assert!(bar0.is_mmio && bar0.is_64bit);
let phys_base = bar0.address; // BAR1 is folded into this by your BarInfo parser already,
                               // if not: phys_base = bar0.address | (bar1.address << 32)
let pages = (bar0.size as usize).div_ceil(4096);
dev.enable_bus_master();
dev.enable_mmio();
let virt = KMemory::map_mmio(PhysAddr::new(phys_base), pages);
let regs = XhciRegisters::new(virt);
```
Create `XhciRegisters(VirtAddr)` following the NVMe pattern with `read<T>`/`write<T>`
generic helpers plus typed accessors (`caplength()`, `hciversion()`, `usbcmd()`,
`set_usbcmd()`, etc.) added incrementally as later steps need them.

**Verify:** read CAPLENGTH and HCIVERSION immediately after mapping; CAPLENGTH should
be a small nonzero value (typically `0x20` or `0x40`), HCIVERSION should read a
plausible BCD version like `0x0100` or `0x0110`. A `0xFFFFFFFF`/all-1s read anywhere
means the mapping or bus mastering/MMIO enable is wrong — double check `enable_mmio()`
was called and the BAR was correctly sized.

**Pitfalls:** forgetting `enable_bus_master()` — MMIO reads may still work but any DMA
the controller tries to do (later steps) will silently fail or fault. Also: BAR0+BAR1
combination — don't treat BAR1 as a second, separate BAR; it's purely the high 32 bits
of BAR0's 64-bit address per PCI 64-bit BAR conventions, so `dev.bar(1)` may already be
folded into `dev.bar(0).address` by your PCI layer — check how `BarInfo` for 64-bit
BARs is constructed elsewhere in your codebase (e.g. AHCI/NVMe) before assuming you
need to combine it yourself.

---

### Step 2 — Capability register parsing

**Do:**
```rust
let cap_length = regs.read::<u8>(0x00) as u64;
let hci_version = regs.read::<u16>(0x02);
let hcsparams1 = regs.read::<u32>(0x04);
let max_slots = (hcsparams1 & 0xFF) as u8;
let max_intrs = ((hcsparams1 >> 8) & 0x7FF) as u16;
let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;

let hcsparams2 = regs.read::<u32>(0x08);
let max_scratchpad_bufs =
    (((hcsparams2 >> 21) & 0x1F) << 5) | ((hcsparams2 >> 27) & 0x1F);

let hccparams1 = regs.read::<u32>(0x10);
let ac64 = hccparams1 & 1 != 0;
let ctx_size: usize = if (hccparams1 >> 2) & 1 != 0 { 64 } else { 32 };

let dboff = regs.read::<u32>(0x14) & !0x3;
let rtsoff = regs.read::<u32>(0x18) & !0x1F;

let op_base = mmio_base + cap_length;
let rt_base = mmio_base + rtsoff as u64;
let db_base = mmio_base + dboff as u64;
```
Store all of these (`max_slots`, `max_ports`, `max_scratchpad_bufs`, `ctx_size`,
`ac64`, `op_base`, `rt_base`, `db_base`) on your driver struct — every later step
needs them.

**Verify:** `max_slots` and `max_ports` should be small plausible numbers (typically
≤ 64 for slots, ≤ 32 for ports on real hardware — QEMU's xHCI model commonly reports
around 4-32 depending on config). If either reads as 0 or absurdly large (e.g.
`0xFF` combined with everything else also looking like all-1s), suspect a bad MMIO
mapping from Step 1.

**Pitfalls:** the CSZ bit (`ctx_size`) is the single most consequential value you
extract here — every context offset computed in Steps 4/9/10 depends on it. Compute
it once, store it, and thread it everywhere rather than re-reading HCCPARAMS1 later.

---

### Step 3 — Controller reset

**Do:**
```rust
// If already running, stop first.
let mut cmd = op.read::<u32>(0x00);
if cmd & 1 != 0 {
    op.write::<u32>(0x00, cmd & !1); // clear RS
    wait_until(|| op.read::<u32>(0x04) & 1 != 0, TIMEOUT)?; // USBSTS.HCH == 1
}

// Reset.
cmd = op.read::<u32>(0x00);
op.write::<u32>(0x00, cmd | (1 << 1)); // HCRST
wait_until(|| op.read::<u32>(0x00) & (1 << 1) == 0, TIMEOUT)?; // HCRST self-clears
wait_until(|| op.read::<u32>(0x04) & (1 << 11) == 0, TIMEOUT)?; // USBSTS.CNR == 0
```

**Verify:** after the loop, USBCMD.HCRST reads 0 and USBSTS.CNR reads 0, USBSTS.HCH
should read 1 (halted, not yet running — expected at this point). All operational
registers should now read their reset defaults (CONFIG=0, DCBAAP=0, CRCR=0, etc.) —
spot check DCBAAP reads 0 as a sanity check that reset actually happened.

**Pitfalls:** you must wait for **CNR (Controller Not Ready) to clear**, not just for
HCRST to self-clear — some controllers clear HCRST quickly but stay CNR=1 for longer
while internal state finishes resetting; writing to DCBAAP/CRCR/CONFIG while CNR=1 is
undefined behavior on real hardware (may be silently dropped or corrupt state).

---

### Step 4 — System memory structures: DCBAAP + scratchpad

**Do:**
```rust
// DCBAA: (max_slots + 1) 64-bit pointers.
let dcbaa = DMABuffer::new((max_slots as usize + 1) * 8);
// zeroed by construction

if max_scratchpad_bufs > 0 {
    let scratchpad_array = DMABuffer::new(max_scratchpad_bufs as usize * 8);
    let mut scratchpad_pages: Vec<PhysPage> = Vec::new(); // or fixed-size array if no alloc
    for i in 0..max_scratchpad_bufs as usize {
        let page = KMemory::alloc_pages(1);
        let phys = page.get_phys_address().as_u64();
        unsafe {
            (scratchpad_array.virt().as_u64() + (i * 8) as u64 as u64)
                .into()... // write phys as u64 at offset i*8
        }
        scratchpad_pages.push(page);
    }
    // DCBAA[0] = scratchpad_array.phys()
    write_u64_at(dcbaa.virt(), 0, scratchpad_array.phys().as_u64());
}

op.write::<u64>(0x30, dcbaa.phys().as_u64()); // DCBAAP, bits 5:0 already 0 (page aligned)
```
(Adjust the raw-pointer-write idiom to whatever helper your `DMABuffer`/`PhysPage`
already expose, e.g. `page.write_data(offset, data)` — for `DMABuffer` add an
equivalent small helper if one doesn't exist yet, following the same
`write_volatile` convention as `XhciRegisters`.)

Keep `dcbaa`, `scratchpad_array`, and `scratchpad_pages` alive for the lifetime of the
driver (store on the driver struct) — they're RAII-freed on drop, and dropping them
while the controller is running will cause the HC to fault on next access.

**Verify:** read DCBAAP back — it should reflect what you wrote (this register, unlike
CRCR, does read back correctly). If scratchpad buffers are in use, there isn't a
direct hardware-visible verification until Step 7 (controller runs without HSE/HCE
errors, which would indicate it couldn't find valid scratchpad memory).

**Pitfalls:** DCBAAP must be 64-byte aligned — a fresh `DMABuffer` is page-aligned so
this is automatic, but if you ever suballocate DCBAA from a larger arena, check
alignment explicitly. Also: entry 0 of DCBAA is scratchpad-array-or-zero, **not** slot
0 — slot IDs start at 1, so DCBAA entries 1..=max_slots correspond to Device Contexts.

---

### Step 5 — Command ring setup

**Do:**
```rust
const CMD_RING_TRBS: usize = 32;
let cmd_ring = DMABuffer::new(CMD_RING_TRBS * 16);

// Write Link TRB at the last slot, pointing back to slot 0, TC=1.
let link_offset = (CMD_RING_TRBS - 1) * 16;
write_trb(cmd_ring.virt(), link_offset, TrbRaw {
    parameter: cmd_ring.phys().as_u64(),   // bits 63:4 used; low bits 0 since page aligned
    status: 0,
    control: (1 << 1) /* TC */ | (6 << 10) /* Link */ | 1 /* Cycle = initial PCS */,
});

let mut pcs = true; // Producer Cycle State
let mut enqueue_index: usize = 0;

// CRCR write.
let crcr_val = (cmd_ring.phys().as_u64() & !0x3F) | 1; // RCS = 1
op.write::<u64>(0x18, crcr_val);

// Shadow state — CRCR reads back as 0, so this struct is the only source of truth.
struct CommandRingState {
    base: PhysAddr,
    pcs: bool,
    enqueue_index: usize,
}
```

**Verify:** CRCR itself will read back as 0 or garbage — **do not** use it to verify.
Instead, defer verification to Step 9 when you issue the first real command (Enable
Slot) and confirm a Command Completion Event arrives referencing the TRB you wrote.
As an earlier sanity check, you can poll CRCR bit 3 (CRR) after Step 7 enables the
controller — it should read 1 once the ring is actively running, even though other
CRCR bits stay 0.

**Pitfalls:** this is the #1 documented gotcha for this register — **reading CRCR (or
any bit of it) always returns 0** on essentially all implementations. Never write
code that reads CRCR to check "did my ring base get set" — it will always look wrong
even when it's correct. Maintain the shadow `CommandRingState` and trust it
exclusively.

---

### Step 6 — Event ring + interrupter setup

**Do:**
```rust
const EVT_RING_TRBS: usize = 64;
let evt_ring = DMABuffer::new(EVT_RING_TRBS * 16); // zeroed -> cycle bits all 0

let erst = DMABuffer::new(16); // 1 ERST entry, rounds up to a page anyway
write_erst_entry(erst.virt(), 0, evt_ring.phys().as_u64(), EVT_RING_TRBS as u16);

let ir0 = rt_base + 0x20; // Interrupter Register Set 0

rt.write_at::<u32>(ir0, 0x08, 1);                      // ERSTSZ = 1 entry
rt.write_at::<u64>(ir0, 0x18, evt_ring.phys().as_u64()); // ERDP, EHB=0 implied
rt.write_at::<u64>(ir0, 0x10, erst.phys().as_u64());   // ERSTBA (write LAST: arms the ring)
rt.write_at::<u32>(ir0, 0x04, 0);                      // IMOD = 0 (no throttling initially)
rt.write_at::<u32>(ir0, 0x00, 1 << 1);                 // IMAN.IE = 1 (leave IP as-is)

let mut ccs = true; // Consumer Cycle State
let mut deq_index: usize = 0;

// MSI-X.
let vector = HardwareInterrupts::XhciIO as u8; // 34
let ok = msix::program(dev, 0, vector);
assert!(ok, "MSI-X programming failed - fall back to MSI");
op.write::<u32>(0x00, op.read::<u32>(0x00) | (1 << 2)); // USBCMD.INTE = 1
```

**Verify:** after enabling and once the controller is running (Step 7), trigger any
event (even a bogus/no-op command) and confirm your interrupt handler fires, sets the
`AtomicBool` completion flag, and that reading the event ring at `deq_index` shows a
TRB with Cycle bit == current CCS. If MSI-X `program()` returns false, fall back to
`crate::drivers::pci::msi::program(dev, vector)` and set USBCMD.INTE the same way.

**Pitfalls:** write **ERSTBA last** — writing it is effectively what tells the HC the
event ring is valid and armed; writing ERSTSZ/ERDP first and ERSTBA last avoids a
window where the HC could start using a partially-configured event ring. Also: ERDP's
low bits are DESI (segment index, 0 here) and EHB — don't accidentally leave stray
bits from a previous read when writing back a "processed" ERDP later in Step 12; mask
to the fields you intend to set. And remember IMAN.IP is RW1C — don't write 1 to it
speculatively during setup, only when actually acknowledging a real pending interrupt.

---

### Step 7 — Enable controller

**Do:**
```rust
op.write::<u32>(0x38, max_slots as u32); // CONFIG.MaxSlotsEn — enable all slots up front

let cmd = op.read::<u32>(0x00);
op.write::<u32>(0x00, cmd | 1 | (1 << 2)); // USBCMD.RS = 1, INTE = 1 (if not already set in Step 6)

wait_until(|| op.read::<u32>(0x04) & 1 == 0, TIMEOUT)?; // USBSTS.HCH == 0
```

**Verify:** USBSTS.HCH reads 0 (controller running). USBSTS.HSE and HCE should remain
0 — if either sets, something in the earlier steps is malformed (bad DCBAAP/CRCR/ERST
pointers most commonly) and you should re-check alignment and physical-vs-virtual
address usage across Steps 4-6.

**Pitfalls:** if you see HSE (Host System Error) immediately after setting RS, it
almost always means one of your DMA pointers (DCBAAP, CRCR, ERSTBA, or the event ring
segment pointer itself) is either not actually a physical address (virtual address
leaked in by mistake) or isn't correctly aligned — double check every `.phys()` call
vs `.virt()` call across the setup steps.

---

### Step 8 — Port detection

**Do:**
```rust
for port in 1..=max_ports {
    let offset = 0x400 + (port as u64 - 1) * 0x10;
    let portsc = op.read::<u32>(offset);
    let ccs = portsc & 1 != 0;
    let speed = (portsc >> 10) & 0xF;
    if ccs {
        log::info!("xHCI: port {port} connected, speed={speed}");
    }
}
```

**Verify:** on real hardware/QEMU with a USB device attached (e.g. `-device
usb-kbd` / `-device usb-tablet` in QEMU with an xHCI controller), at least one port
should show CCS=1 with a nonzero speed field. With nothing attached, all ports
legitimately read CCS=0 — that's a correctly-working driver, not a bug.

**Pitfalls:** don't write back the raw value you read from PORTSC while just polling
— it contains RW1C change bits (CSC, PEC, etc.) that would get spuriously cleared if
echoed back. This step should be **read-only**.

---

### Step 9 — Device enumeration (on connection)

**Do**, once you've detected `CCS=1` on a port (from Step 8's poll or Step 13's
hotplug event):

1. **Reset the port** (required before the device is addressable):
   ```rust
   let offset = 0x400 + (port as u64 - 1) * 0x10;
   let mut v = op.read::<u32>(offset);
   v &= !RW1C_MASK; // clear all RW1C bits in our local copy so we don't clear real changes
   v |= 1 << 4;      // PR = 1
   op.write::<u32>(offset, v);
   wait_until(|| op.read::<u32>(offset) & (1 << 21) != 0, TIMEOUT)?; // PRC set
   // clear PRC by writing 1 to it (and only it)
   op.write::<u32>(offset, (1 << 21));
   ```
2. **Enable Slot Command**: build a Command TRB (Type 9) with Parameter=0,
   Status=0, Control = cycle bit | (9 << 10). Enqueue on the command ring, ring
   doorbell 0. Wait (via the `AtomicBool` completion pattern, or a synchronous
   poll of the event ring if interrupts aren't wired yet) for a Command Completion
   Event referencing this TRB; read **Slot ID from Control[31:24]** of that event.
3. **Allocate the Device Context** for this slot: `DMABuffer::new(32 * ctx_size)`,
   zeroed. Write its physical address into `DCBAA[slot_id]` (i.e. at byte offset
   `slot_id * 8` in the DCBAA buffer).
4. **Allocate EP0's transfer ring**: same ring-construction pattern as the command
   ring (§2.4) — N TRBs + trailing Link TRB with TC=1, own PCS shadow state.
5. **Build an Input Context**: `DMABuffer::new(33 * ctx_size)` (1 control block + 32
   context slots), zeroed, then:
   - Input Control Context: A0=1, A1=1 (bits 0 and 1 of dword1 at offset `ctx_size`... — dword1 is at byte offset 4 within the control block, so `ctx_size + 4`).
   - Slot Context (`ctx_size` bytes after the control block): Route String=0 (root
     port device), Speed = value read from PORTSC bits 13:10 at detection time,
     Root Hub Port Number = the physical port number, Context Entries=1,
     Interrupter Target=0.
   - EP0 Context (`2 * ctx_size` bytes after the control block start): EP Type=4
     (Control Bidir), Max Packet Size = 8 for Low Speed / 64 for High Speed / 512
     for SuperSpeed (refine after reading the real device descriptor in Step 10),
     CErr=3, TR Dequeue Pointer = EP0 transfer ring's physical address (bits 63:4),
     DCS=1, Average TRB Length=8.
6. **Address Device Command**: Command TRB (Type 11), Parameter = Input Context
   physical address (bits 63:4), Control = cycle bit | (11 << 10) | BSR-bit-clear
   (bit 9 = 0, meaning actually issue SET_ADDRESS, not just set default state) |
   Slot ID in Control[31:24]. Enqueue, ring doorbell 0, wait for Command Completion
   Event with Completion Code = Success (1).

**Verify:** after step 6 completes successfully, read back the Device Context's Slot
Context dword3 — **Slot State** (bits 31:27) should now read `2` (Addressed), and
**USB Device Address** (bits 7:0) should be a nonzero HC-assigned address. This is the
authoritative confirmation enumeration reached the addressed state.

**Pitfalls:**
- Forgetting to clear the port's RW1C bits before writing PR — accidentally clearing
  CSC before you've processed it means you can miss the fact the device is present at
  all in edge cases.
- Using the wrong Speed value in the Slot Context — it must match PORTSC's Port Speed
  field encoding exactly (same 1-5 values), not a re-derived guess.
- EP0's Max Packet Size is a **guess** at this stage (based on speed) — for Full Speed
  devices in particular the real value (8/16/32/64) is only known after reading the
  first 8 bytes of the device descriptor; plan to follow up with an Evaluate Context
  Command once you know the real value (Step 10).
- Using `ctx_size` inconsistently between the Device Context and Input Context layout
  math — both must use the same stride from HCCPARAMS1.CSZ.

---

### Step 10 — Endpoint configuration

**Do:**
1. **Control endpoint 0 traffic** (needed immediately to read descriptors): issue a
   GET_DESCRIPTOR (Device) control transfer using Setup Stage + Data Stage(IN) +
   Status Stage(OUT) TRBs on EP0's transfer ring (see Step 11 for TRB sequencing
   details). Parse `bMaxPacketSize0` from the returned device descriptor.
2. If the real `bMaxPacketSize0` differs from your Step 9 guess, issue an
   **Evaluate Context Command** (Type 13): Input Context with A1=1 only, EP0 Context's
   Max Packet Size field updated, everything else zeroed/untouched; wait for
   completion.
3. Read the rest of descriptors you need (configuration, interface, endpoint
   descriptors) via further control transfers to determine the device's real
   endpoints (numbers, directions, types, max packet sizes, intervals).
4. For each additional endpoint to activate: build a fresh Input Context, set A0=1
   (Slot Context, updated Context Entries) plus An=1 for each new endpoint's DCI,
   fill in each Endpoint Context (EP Type per §2.2's encoding, Max Packet Size,
   Max Burst Size from the SuperSpeed Endpoint Companion descriptor if applicable,
   Interval, freshly-allocated Transfer Ring pointer + DCS=1).
5. **Configure Endpoint Command** (Type 12): Parameter = Input Context physical
   address, Control = cycle | (12 << 10) | Slot ID. Enqueue, ring doorbell 0, wait
   for Command Completion Event Success.

**Verify:** re-read the Device Context after the Configure Endpoint completion —
Slot State should now read `3` (Configured), and each newly-added Endpoint Context's
EP State (bits 2:0) should read `1` (Running).

**Pitfalls:** Context Entries in the Slot Context must be updated to the **highest**
DCI you've configured, not just incremented by however many endpoints you added this
round — recompute it as `max(existing, highest new DCI)` each time you configure
endpoints incrementally.

---

### Step 11 — USB transfers

**Do**, for a control transfer (e.g. the GET_DESCRIPTOR from Step 10):
```text
1. Setup Stage TRB: bmRequestType/bRequest/wValue/wIndex/wLength packed into
   Parameter, IDT=1, TRT = 3 (IN Data) or 2 (OUT Data) or 0 (No Data),
   TRB Transfer Length = 8, cycle bit set, write to EP0 ring, DO NOT ring doorbell yet.
2. Data Stage TRB (if TRT != 0): Data Buffer Pointer = physical address of a
   DMABuffer sized for the expected response, TRB Transfer Length = wLength,
   DIR = 1 for IN, cycle bit set, write to EP0 ring.
3. Status Stage TRB: DIR = opposite of Data stage direction (or 1/IN if no data
   stage), IOC = 1 (so you get notified when the whole control transfer finishes),
   cycle bit set, write to EP0 ring.
4. Ring doorbell for Slot ID, DB Target = 1 (EP0's DCI).
5. Wait for Transfer Event on the event ring referencing the Status Stage TRB's
   address, check Completion Code == Success, then read the Data Stage buffer.
```
For bulk/interrupt transfers on other endpoints: one or more **Normal TRBs** (chained
with CH=1 if split across multiple buffers, IOC=1 on the last), ring the relevant
endpoint's doorbell (DB Target = that endpoint's DCI), wait for the Transfer Event.

**Verify:** Transfer Event's Completion Code == 1 (Success), and TRB Transfer Length
in the event matches (or sensibly relates to, for short packets/code 13) what you
requested. For GET_DESCRIPTOR specifically, sanity-check the returned
`bLength`/`bDescriptorType` fields look like a real device descriptor (`bLength=18`,
`bDescriptorType=1`).

**Pitfalls:** writing TRBs in the wrong order relative to ringing the doorbell — all
TRBs of a TD (and here, all three stages, since chaining across TRB types within one
control transfer is how xHCI expects it) must be fully written **before** the
doorbell ring, since the HC may start processing immediately. Also: the cycle bit
must be the *last* field written per TRB (write Parameter and Status first, Control
with the cycle bit last) to avoid the HC observing a torn write.

---

### Step 12 — Interrupt handler and event processing loop

**Do:**
```rust
pub static XHCI_COMPLETION_FLAG: AtomicBool = AtomicBool::new(false);

// Interrupt handler (registered against HardwareInterrupts::XhciIO / vector 34):
extern "x86-interrupt" fn xhci_interrupt_handler(_frame: InterruptStackFrame) {
    XHCI_COMPLETION_FLAG.store(true, Ordering::Release);
    lapic_eoi();
}

// Called from the driver's event-processing routine, invoked after the flag is observed set:
fn process_events(&mut self) {
    loop {
        let trb = self.read_event_trb(self.deq_index);
        if trb.cycle_bit() != self.ccs { break; }
        match trb.trb_type() {
            32 => self.handle_transfer_event(trb),
            33 => self.handle_command_completion_event(trb),
            34 => self.handle_port_status_change_event(trb),
            _ => log::warn!("xHCI: unhandled event type {}", trb.trb_type()),
        }
        self.deq_index += 1;
        if self.deq_index == EVT_RING_TRBS {
            self.deq_index = 0;
            self.ccs = !self.ccs;
        }
    }
    let erdp_val = self.evt_ring.phys().as_u64() + (self.deq_index as u64 * 16);
    self.rt.write_at::<u64>(self.ir0, 0x18, erdp_val | (1 << 3)); // + EHB
    // IMAN.IP is RW1C — acknowledge it explicitly too:
    self.rt.write_at::<u32>(self.ir0, 0x00, self.rt.read_at::<u32>(self.ir0, 0x00) | 1);
}
```
Wait pattern for synchronous callers (matches the NVMe `AtomicBool` convention):
```rust
let start = time();
loop {
    if XHCI_COMPLETION_FLAG.swap(false, Ordering::Acquire) {
        without_interrupts(|| self.process_events());
        if self.found_completion_for(expected_trb_ptr) { break; }
    }
    if time() - start > TIMEOUT { return Err(IOError::Timeout); }
    core::hint::spin_loop();
}
```

**Verify:** issuing any command (e.g. re-issuing a No Op Command, Type 23, which is
safe and side-effect-free) should reliably trigger the interrupt handler, and
`process_events` should correctly locate and decode the resulting Command Completion
Event.

**Pitfalls:** updating ERDP without setting EHB, or masking off DESI incorrectly when
single-segment (DESI should stay 0) — both leave the HC thinking software hasn't
caught up. Also, remember IMAN.IP is a separate RW1C acknowledgment from ERDP.EHB;
both should be cleared as part of a clean event-processing pass, not just one or the
other.

---

### Step 13 — Hotplug (async port status change events)

**Do:**
```rust
fn handle_port_status_change_event(&mut self, trb: EventTrb) {
    let port = ((trb.parameter() >> 24) & 0xFF) as u8;
    let offset = 0x400 + (port as u64 - 1) * 0x10;
    let portsc = self.op.read::<u32>(offset);
    let csc = portsc & (1 << 17) != 0;
    let ccs = portsc & 1 != 0;

    // Acknowledge the change bits we're handling (RW1C, write only the ones set).
    self.op.write::<u32>(offset, portsc & RW1C_MASK);

    if csc {
        if ccs {
            self.enumerate_device(port); // Step 9 flow
        } else {
            self.cleanup_device(port);   // tear down slot, free rings/contexts
        }
    }
}
```
`cleanup_device`: find the Slot ID associated with this port (track a
`port -> slot_id` map on the driver struct populated during Step 9), issue a
**Disable Slot Command** (Type 10, Slot ID in Control[31:24]), wait for completion,
then drop the Device Context / Input Context / transfer ring `DMABuffer`s for that
slot (RAII frees them) and zero the corresponding DCBAA entry.

**Verify:** unplug/replug a device (or in QEMU, use the monitor to
`device_del`/`device_add` a USB device) and confirm both directions correctly trigger
enumeration and cleanup without leaking a slot (re-plugging repeatedly shouldn't
exhaust `max_slots`).

**Pitfalls:** the RW1C write to acknowledge PORTSC must mask to *only* the bits that
were actually set in your read (`portsc & RW1C_MASK`, not a hardcoded "clear
everything" value) — writing 1 to a bit that wasn't set can have unintended side
effects like unintentionally forcing a warm reset (WPR) on some controllers if bit
patterns are constructed carelessly.

---

### Step 14 — Error recovery

**Do:**
- **Doorbell/command timeout** (no Command Completion Event within `TIMEOUT`): issue
  **Command Abort**: write CRCR with CA=1 (bit 2) using your shadow base address —
  `(shadow.base.as_u64() & !0x3F) | (1 << 2)`. Wait for CRCR.CRR (bit 3) to clear,
  which confirms the ring actually stopped; you should also see a Command Completion
  Event with Completion Code = Command Ring Stopped (24) for the aborted command.
- **Endpoint stall** (Transfer Event Completion Code = Stall Error, 21): issue a
  **Reset Endpoint Command** (Type 14, Slot ID + Endpoint ID in Control), then a
  **Set TR Dequeue Pointer Command** (Type 16) to reposition that endpoint's transfer
  ring past the failed TRB, then clear the device-side STALL condition via a
  `CLEAR_FEATURE(ENDPOINT_HALT)` control transfer if appropriate for the class driver.
- **Fatal error** (USBSTS.HSE or HCE set): the controller is no longer trustworthy —
  perform a full **HCRST** (Step 3's reset sequence) and redo Steps 4-7 from scratch,
  re-enumerating all previously-attached devices as if freshly plugged in (their
  slots are gone after HCRST).
- **CRCR shadow re-sync after abort**: since CRCR always reads 0 (§ Step 5 pitfall),
  after a Command Abort you must reconstruct your enqueue pointer from where you
  *know* you left off (your own shadow state), not by reading hardware — the abort
  doesn't tell you where the HC actually stopped beyond the Command Ring Stopped
  event, which references the TRB it stopped at.

**Verify:** deliberately trigger a stall (e.g. send a request to a nonexistent
endpoint on a real device, or an invalid control request) and confirm the driver
detects Completion Code 21, performs the Reset Endpoint + Set TR Dequeue Pointer
sequence, and subsequent transfers to that endpoint succeed again without a full
device re-enumeration.

**Pitfalls:** attempting to keep using a transfer ring after a Stall without
repositioning the dequeue pointer via Set TR Dequeue Pointer Command — the ring's
internal HC-side dequeue pointer is left sitting at the failed TRB and further
doorbell rings will be ignored or re-process the same failed TRB.

---

## Summary of Cross-Cutting Pitfalls (repeated for visibility)

1. **CRCR always reads 0.** Never verify the command ring via read-back; maintain
   shadow state (base address, PCS, enqueue index).
2. **Context size is 32 or 64 bytes**, decided at runtime by HCCPARAMS1 bit 2 (CSZ).
   Every Device Context / Input Context offset calculation must use this value
   consistently — compute it once, store it, thread it everywhere.
3. **Event ring cycle bit must toggle on wraparound.** Forgetting this makes the
   driver appear to stop receiving events after exactly one lap through the ring.
4. PORTSC (and other RW1C-bearing registers) must be **read-modify-masked-write**,
   never blindly echoed back, or you'll clear change bits you haven't processed.
5. All pointer fields in TRBs/contexts/registers are **physical addresses** — use
   `.phys()`/`.get_phys_address()`, never `.virt()`/`.get_virt_addr()`.
6. Write TRB fields in order **Parameter → Status → Control**, with the Cycle bit
   the very last write, so the HC never observes a half-constructed TRB.


