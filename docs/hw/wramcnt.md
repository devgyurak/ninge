# Shared WRAM and WRAMCNT

Sources: GBATEK "DS Memory Control - WRAM"; GBATEK "DS Memory Maps" (NDS9 and NDS7 Memory Map);
GBATEK "DS Memory Control - Shared-RAM" for the `037F_8000h` mirror.
Used by: `crates/core/src/bus/`, `crates/core/src/hle/direct_boot.rs`   Phase: 3 (#22)

The DS has 32 KiB of shared WRAM that a register hands to the two CPUs, in whole 16 KiB halves.
The ARM7 additionally has 64 KiB of its own WRAM. This note covers who sees what; the rest of
each CPU's map is in `arm9-memory-map.md` and `scheduler.md`.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0247h` | WRAMCNT | 8 | ARM9 R/W (see "Unverified") | not stated, 0 here |
| `0400_0241h` | WRAMSTAT | 8 | ARM7 read-only | mirrors WRAMCNT |

### WRAMCNT bits

| Bits | Meaning |
|---|---|
| 0-1 | Allocation mode (below) |
| 2-7 | Not used |

| Mode | ARM9 gets | ARM7 gets |
|---|---|---|
| 0 | all 32 KiB | nothing |
| 1 | the second 16 KiB | the first 16 KiB |
| 2 | the first 16 KiB | the second 16 KiB |
| 3 | nothing | all 32 KiB |

GBATEK warns that the register should not be changed under the official API; it is a
boot-time switch for software that knows what it is doing.

## Behavior

**ARM9 window.** `0300_0000h`-`03FF_FFFFh` (16 MiB). Whatever the ARM9 owns repeats through the
whole window: with 32 KiB the offset is `(addr - 0300_0000h) mod 32 KiB`; with one 16 KiB half it
is that half, repeating every 16 KiB. With nothing allocated the window is empty ("undefined
data").

**ARM7 window.** `0300_0000h`-`037F_FFFFh` (8 MiB) is shared WRAM, repeated the same way. When
the ARM7 owns nothing, the same range shows the ARM7's own 64 KiB WRAM instead, which repeats
through it. `0380_0000h`-`03FF_FFFFh` is always the ARM7's own WRAM, repeating every 64 KiB.

**The `037F_8000h` mirror.** Programs reach shared WRAM at `037F_8000h` on both CPUs. On the
ARM9 that is just a spot inside the repeating window. On the ARM7 the 32 KiB ending at
`037F_FFFFh` is the last repetition of the shared allocation, and it runs straight into
`0380_0000h`, so with all 32 KiB allocated to the ARM7 the block `037F_8000h`-`0380_FFFFh` is
one contiguous 96 KiB. With nothing allocated to the ARM7 the same addresses are the ARM7's own
WRAM seen through its repetition.

**Changing the mode.** The new split takes effect for the next access by either CPU. The bytes
in the shared WRAM do not move or clear; only which half each CPU reaches changes, so a byte
written under one mode shows up elsewhere (or not at all) under another. The 16 KiB halves keep
their own contents (the "first" half is bytes 0-16383 of the 32 KiB, the "second" is
16384-32767).

## Edge cases

- A write to `0400_0246h` and `0400_0247h` together (halfword) or the four registers at
  `0400_0244h` (word) is split into bytes; the byte for WRAMCNT sets the mode. Byte `0400_0246h`
  (VRAMCNT_G) belongs to Phase 4 (#35); a write to it is discarded here.
- Bits 2-7 of a WRAMCNT write are dropped, so the register reads back as 0-3.
- The ARM7 writing `0400_0247h` changes nothing: WRAMCNT is an ARM9 register and is not on the
  ARM7 I/O map.
- A mode change in the middle of a run is seen by the ARM7 on its next instruction, including
  an instruction fetch.
- The first and last byte of each half (`+3FFFh` / `+4000h`) and the first byte after the
  window (`0400_0000h`) are the boundaries that decide ownership.
- ARM7 I/O other than WRAMSTAT is still unmodeled and reads 0 (`TODO(phase3)`).

## Unverified

- **WRAMCNT after reset.** GBATEK's WRAM section gives no value. Power-on state is 0 (all to the
  ARM9), a plain reset choice; direct boot then sets 3 (below). — `Memory::new`
  in `crates/core/src/bus/memory.rs` — verify by a test ROM on hardware reading it first.
- **WRAMCNT after direct boot** is set to 3 (all 32 KiB to the ARM7); GBATEK does not state the
  value. It is inferred: GBATEK's "NDS BIOS RAM Usage" lists, at cartridge boot, the ARM7 boot
  area `037F_8000h`-`0380_7DFFh`, the ARM7 stacks `0380_FD80h`/`0380_FF80h`/`0380_FFC0h` and the
  IRQ vector `0380_FFF8h`/`0380_FFFCh` as separate things. With mode 0 the ARM7's 64 KiB WRAM
  repeats through the `037F_8000h` window, so `037F_FD80h` and `0380_FD80h` would be the same
  byte and a first push would overwrite the tail of a binary larger than `7D80h` bytes; mode 3
  keeps them apart. The cost: the ARM9 stacks (`0300_2F7Ch` and up) are unbacked after boot, which
  fits the "(!)" GBATEK puts next to the ARM9 stack but is not stated. — `boot` in
  `crates/core/src/hle/direct_boot.rs` — verify by a test ROM on hardware reading WRAMCNT first.
- **WRAMCNT read.** GBATEK's WRAM section says R/W, its I/O map says write-only. The model returns
  the stored value to the ARM9. — `Memory::read_slow` — verify likewise.
- **An empty ARM9 window** reads 0 and ignores writes; GBATEK says "undefined data". —
  `Memory::read_slow` — verify likewise.
- **WRAMSTAT** value is the stored 2-bit mode, upper bits 0. — `Memory::read_slow7` — verify likewise.
- **Mirroring through the 16 MiB / 8 MiB windows** is as written in GBATEK; the ARM7's repetition of
  its own WRAM through `0380_0000h`-`03FF_FFFFh` is GBATEK's general rule. — `Memory::build_wram_pages`
  — verify likewise.
- **The ARM7's contiguous 96 KiB at `037F_8000h`.** GBATEK's Shared-RAM text says only that
  programs commonly use mirrors at `037F_8000h` on both CPUs; the contiguity with `0380_0000h`
  is what the repetition rule gives, not a stated fact. — `Memory::build_wram_pages` — verify
  likewise.
- **Other ARM7 addresses** (I/O, VRAM windows, BIOS) read 0. — `Memory::read_slow7`.
