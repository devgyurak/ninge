# DMA

Sources: GBATEK "DS DMA Transfers" (the DMA registers, the control bits, the start-timing
tables, the fill registers); GBATEK "DS Interrupts" for the IF bits (see `interrupts.md`).
Used by: `crates/core/src/dma.rs`, `crates/core/src/io.rs`, `crates/core/src/lib.rs`,
`crates/core/src/scheduler/mod.rs`   Phase: 3 (#26)

Each CPU has four DMA channels. They sit at the same addresses on the ARM9 and the ARM7 but are
separate hardware, like the timers (`timers.md`): a write by one CPU never shows on the other, and
a channel's end-of-transfer IRQ goes to the CPU that owns it. A channel copies a block of 16-bit
or 32-bit units from a source address to a destination address through the owning CPU's bus.

## Registers

Channel *n* (0-3) owns twelve bytes starting at `0400_00B0h + 12n`:

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_00B0h` + 12n | DMAnSAD | 32 | R/W | not stated, 0 here |
| `0400_00B4h` + 12n | DMAnDAD | 32 | R/W | not stated, 0 here |
| `0400_00B8h` + 12n | DMAnCNT | 32 | R/W | not stated, 0 here |
| `0400_00E0h` + 4n | DMAnFILL | 32 | R/W, ARM9 only | not stated, 0 here |

DMAnCNT is one 32-bit register: its low half (GBATEK's `_L`) is the word count and its high half
(`_H`) is the control. The ARM7 has no fill registers: that block is not on its bus.

### Address masks

The source and destination are 28-bit addresses on the ARM9 (bits 28-31 read 0). On the ARM7 the
width depends on the channel; see "Unverified".

| CPU | Source | Destination |
|---|---|---|
| ARM9, all channels | 28 bits | 28 bits |
| ARM7, channel 0 | 27 bits | 27 bits |
| ARM7, channels 1-2 | 28 bits | 27 bits |
| ARM7, channel 3 | 28 bits | 28 bits |

### Word count (DMAnCNT bits 0-20)

The count is in units of the transfer width. It is 21 bits on the ARM9. On the ARM7 it is 14 bits
for channels 0-2 and 16 bits for channel 3; the bits above read 0. A count of 0 means the largest
value, one past the field's top: 200000h (ARM9), 4000h (ARM7 channels 0-2) or 10000h (ARM7
channel 3).

### DMAnCNT control bits (the high half)

| Bits | Meaning |
|---|---|
| 21-22 | Destination control: 0 increment, 1 decrement, 2 fixed, 3 increment and reload |
| 23-24 | Source control: 0 increment, 1 decrement, 2 fixed, 3 prohibited |
| 25 | Repeat |
| 26 | Unit: 0 = 16-bit, 1 = 32-bit |
| 27-29 | Start timing, ARM9 (three bits) |
| 28-29 | Start timing, ARM7 (two bits; bit 27 is not used) |
| 30 | IRQ at the end of the transfer |
| 31 | Enable (busy) |

### Start timing

ARM9 (bits 27-29):

| Value | Start | Phase |
|---|---|---|
| 0 | Immediately | 3 (#26) |
| 1 | VBlank start | 3 (#26) |
| 2 | HBlank start | 3 (#26) |
| 3 | Start of display (once per frame, line 0) | `TODO(phase4)` |
| 4 | Main-memory display | `TODO(phase4)` |
| 5 | DS cartridge slot | `TODO(phase5)` (#37) |
| 6 | GBA cartridge slot | no phase plans it |
| 7 | Geometry command FIFO | `TODO(phase7)` (#48) |

ARM7 (bits 28-29):

| Value | Start | Phase |
|---|---|---|
| 0 | Immediately | 3 (#26) |
| 1 | VBlank start | 3 (#26) |
| 2 | DS cartridge slot | `TODO(phase5)` (#37) |
| 3 | Channels 0 and 2: wireless interrupt; channels 1 and 3: GBA cartridge slot | no phase plans it |

The ARM7 has no HBlank start. A channel set to a start timing that is not implemented is enabled
and busy but never starts.

### IF bits

The end of a channel's transfer requests IF bit 8 + *n* on the CPU that owns it, when control bit
30 is set (`interrupts.md`, "IE and IF bits"): bits 8-11.

## Behavior

### Enabling

Enable is control bit 31. When it goes from 0 to 1, the channel latches its source address,
destination address and word count into internal copies; the transfer works on those, and the
registers keep what was written. A write that leaves enable at 1 changes the registers only: the
internal copies are not reloaded. A write that clears enable stops the channel; a transfer that
was waiting for its start trigger never runs.

Enable reads 1 for as long as the channel is armed or transferring (it is the busy flag): code
that polls it waits for the transfer to finish.

### Starting

- **Immediately:** the transfer starts when the enable write retires.
- **VBlank start:** the transfer starts at the start of VBlank, line 192 (`display-timing.md`,
  "Hooks for later features"). Enabling during VBlank waits for the next one.
- **HBlank start:** the transfer starts at the start of HBlank in each visible line, 0-191. See
  "Unverified" for the blank lines.

When several channels of one CPU start at the same time, channel 0 goes first, then 1, 2, 3. When
both CPUs have channels waiting for the same trigger, the ARM9's go before the ARM7's (the tie
order of `scheduler.md`, "Interleaving").

### Transferring

A transfer moves *count* units. For each unit the channel reads the source, then writes the
destination, both through the owning CPU's bus (so it reaches main RAM, VRAM, I/O registers and
the other devices alike), then moves the addresses by the unit's size (2 or 4 bytes):

- **Increment:** add the size. **Decrement:** subtract it. **Fixed:** no change.
- **Increment and reload** (destination only) behaves as increment during the transfer; it matters
  at a repeat.
- Source control 3 is prohibited. The model treats it as increment (see "Unverified").
- Addresses are aligned down to the unit's size (halfword or word) before use.

DMA cannot reach the ARM9's tightly-coupled memories. The model gives its accesses the bus as it
is with both TCM regions switched off, so an address inside a mapped ITCM or DTCM window reads and
writes the address behind it (`arm9-memory-map.md`, `cp15.md`); see "Unverified".

### Fill (ARM9)

The fill registers hold data that a DMA can use as a source: a channel whose source address is
that channel's DMAnFILL address, with source control fixed, writes the fill word over and over.
They are ordinary readable registers; no special mode is involved.

### The end of a transfer

When the last unit is written:

1. If control bit 30 is set, IF bit 8 + *n* is requested.
2. If repeat (bit 25) is set and the start timing is not Immediate, the channel stays enabled and
   waits for its next trigger: the internal count is reloaded from the register, and the internal
   destination is reloaded from the register when destination control is 3. The internal source
   is not reloaded; it continues from where the transfer ended.
3. Otherwise enable is cleared.

## Edge cases

- **Count 0** moves the largest count above. **Count 1** moves one unit.
- **A transfer whose source or destination is a DMA register** (its own channel's, another
  channel's, or the fill registers) goes through the same bus as any access and takes effect as a
  register write would; the transfer works on its latched copies.
- **A transfer whose destination is an IPC FIFO** sends one word per unit through the same path as
  a CPU write, so a full FIFO sets its error bit as `ipc.md` ("Behavior") says it does for a
  write.
- **Two channels due at once** run one after the other in channel order; each finishes before the
  next starts.

## Model (not hardware)

These are choices so DMA fits the event scheduler (`scheduler.md`); none is claimed to be
hardware behavior.

- **One event per channel.** A start, whatever its timing, is a `DmaStart` event on the channel's
  slot, queued when the start is known: at the end of the enable write for Immediate, at the
  trigger's time for VBlank and HBlank. Events for channels triggered together are queued in
  channel order, so `(time, seq)` fires them 0 to 3.
- **The transfer is atomic.** When the event fires, all units move at once, at that master time,
  and the CPUs are not stopped while it happens. The enable bit stays set until it is done.
- **An ARM7 start can be late.** The ARM7's clock lags the master clock inside a round, so a
  transfer an ARM7 write starts fires at the end of the round, up to one slice later
  (`scheduler.md`, "Interleaving"; `timers.md`, "Model" has the same lag for a timer).
- **The transfer uses the owning CPU's bus view with no TCM**, and the ARM7 side uses the ARM7's
  clock for the register reads it makes.

## Unverified

- **Cost and CPU stall.** A real transfer takes cycles and holds the bus; the model takes none
  and stops no CPU — `crates/core/src/dma.rs`, `crates/core/src/lib.rs` — revisit with the
  instruction-cost work (#115), verify with a cycle-count test ROM on hardware.
- **Preemption by priority.** On hardware a lower-numbered channel can pre-empt a running one; the
  atomic transfer cannot be pre-empted — `crates/core/src/lib.rs` — same fix as above.
- **ARM7 start lag** (see "Model") — `crates/core/src/lib.rs` — the same bounded approximation as
  the ARM7 timer lag (#118).
- **ARM7 address widths** (the table in "Registers") are as GBATEK gives them for the GBA's
  channels and are carried over for the ARM7 here — `crates/core/src/dma.rs` — verify with a
  test ROM that writes `FFFF_FFFFh` to each address register and reads it back.
- **Address registers read back what was written**, masked. GBATEK lists them as write-only for
  some CPUs — `crates/core/src/dma.rs` — verify the same way.
- **HBlank start in the blank lines (192-262).** The model starts nothing there —
  `crates/core/src/lib.rs` — verify with a test ROM that counts HBlank transfers over a frame.
- **Repeat with Immediate.** The model ignores repeat for Immediate: the channel ends and enable
  clears — `crates/core/src/dma.rs` — verify with a test ROM.
- **Source control 3** acts as increment — `crates/core/src/dma.rs`.
- **Unused control bits** (ARM7 bit 27, and the ARM7's third start-timing bit) read 0 and are
  dropped on write — `crates/core/src/dma.rs`.
- **Same-time channel order** (0 to 3, the ARM9's before the ARM7's) is the model's reading of
  the priority rule — `crates/core/src/dma.rs`, `crates/core/src/io.rs`.
- **What is behind a TCM window.** GBATEK says DMA cannot reach the TCMs; that its access falls
  through to the address behind the window is the model's choice —
  `crates/core/src/lib.rs` (`run_dma`) — verify with a test ROM that DMAs from a mapped DTCM
  address.
- **Writes to the channel's own registers during its transfer.** The transfer works on its
  latched copies; at its end enable is cleared or kept by the repeat rule above, whatever the
  register held — `crates/core/src/dma.rs`.
- **A channel re-armed by its own transfer.** If the transfer clears and sets its own enable, the
  new start is kept and the end-of-transfer rules do not touch the channel —
  `crates/core/src/dma.rs`.
- **Half and byte accesses** to a register split into bytes in ascending address order, and the
  enable edge is seen when the byte holding bit 31 is written (`arm9-memory-map.md`,
  "Unverified") — `crates/core/src/dma.rs`.
- **IRQ with enable cleared mid-transfer.** The IRQ bit is read at the end — `dma.rs`.
- **The interrupt-during-DMA note** in GBATEK ("raw TCM-only IRQs may be processed during
  DMA(?)", `interrupts.md`) is not modeled: no CPU runs during an atomic transfer.
