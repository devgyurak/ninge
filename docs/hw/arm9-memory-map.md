# ARM9 memory map: main RAM, TCM, unmapped space

Sources: GBATEK "DS Memory Maps" (NDS9 Memory Map, Undefined I/O Ports, Undefined Memory
Regions); GBATEK "DS Memory Control - Cache and TCM"; GBATEK "ARM CP15 Tightly Coupled Memory
(TCM)"; GBATEK "ARM Opcodes: Memory: Single Data Transfer" and "ARM CPU Memory Alignments" (via
`arm-load-store.md`, "Alignment"); GBATEK "DS Memory Control - VRAM" (via `vram-banks.md`).
Used by: `crates/core/src/bus/`   Phase: 2 (#18), ARM9 view only

This note is the address decoding the ARM9 sees. The TCM registers are in `cp15.md`; VRAM bank A
is in `vram-banks.md`. The ARM7's view, shared WRAM and WRAMCNT are in `wramcnt.md` (#22); the other
VRAM banks are Phase 4 (#35); the BIOS at `FFFF0000h` is `hle-swi.md` (#27). Until those land the
regions below are simply unmapped.

## Regions (ARM9, Phase 2)

| Base | Size | What | Phase 2 |
|---|---|---|---|
| `0000_0000h` | 32 KiB physical | Instruction TCM, mirrored over its virtual size | mapped |
| `0xxx_x000h` | 16 KiB physical | Data TCM, base from CP15 | mapped |
| `0200_0000h` | 4 MiB | Main RAM, repeated through the 16 MiB block | mapped |
| `0300_0000h` | 0, 16 or 32 KiB | Shared WRAM | mapped by WRAMCNT (`wramcnt.md`); empty when the ARM9 owns none |
| `0400_0000h` | | I/O ports | only VRAMCNT_A; the rest reads 0 |
| `0500_0000h`, `0700_0000h` | | palettes, OAM | unmapped, `TODO(phase4)` |
| `0600_0000h`..`067F_FFFFh` | | BG/OBJ VRAM windows | unmapped, `TODO(phase4)` (#35) |
| `0680_0000h` | 128 KiB used | VRAM bank A as LCDC | mapped while bank A is enabled with MST = 0 |
| `0800_0000h`.. | | GBA slot | unmapped |
| `FFFF_0000h` | 32 KiB | ARM9 BIOS | mapped, synthetic: reads zero, writes ignored (`hle-swi.md`, "The BIOS area") |

## Evidence

The `memmap` test ROM (`testroms/memmap/`) exercises the map on the emulator and records what
it read; `crates/runner/tests/memmap.rs` holds the expected values. It covers main RAM in all
three widths and the unaligned load (slots 0-9), the last word of the 4 MiB (14) and the mirrors of it and of other words (10-13, 15-17), the DTCM, its
mirroring and its load mode (18-24), the ITCM, its mirror and an instruction fetch from it
(25-27), and an undefined I/O port at `0400_0090h`, an address no ARM9 register in GBATEK's I/O
map occupies (32). The ROM clears the protection unit and cache enable bits of the control register before it
starts (the entry state is not stated, `direct-boot.md`), so on hardware the record does not
depend on them. Running it there and comparing the record with this note settles the
Unverified items marked below.

## Behavior

### Order of decoding

For an access the ARM9 makes, the first match wins:

1. **TCM.** The instruction TCM, then the data TCM (below). Neither goes over the bus.
2. **Memory regions** of the table above, found by the top address bits (the BIOS included).
3. **I/O ports** at `04xx_xxxxh`.
4. **Unmapped.** Nothing answers.

### Main RAM

4 MiB at `0200_0000h`. GBATEK's rule for a 16 MiB block that holds a defined region is that the
unused upper part usually holds mirrors of that region (TCM and BIOS excepted). So the 4 MiB
repeat four times through `0200_0000h`-`02FF_FFFFh`: the offset is `(addr - 0200_0000h) mod
4 MiB`. A 16 MiB block that holds nothing (or only mapped TCM) is typically undefined.
GBATEK mentions an 8 MiB main RAM on the debug hardware; this model has the retail 4 MiB.

Power-on contents are not stated; see "Unverified".

### Alignment

The memory system ignores the address bits below the access width: a word access uses
`addr & !3`, a halfword access `addr & !1`. The CPU rotates or sign-extends what it reads from
those low bits (`arm-load-store.md`, "Alignment"); the bus never rotates. Bytes use the address
as given. Data is little-endian (the control register's big-endian bit is only held,
`cp15.md`).

### Tightly coupled memory

A TCM is described by the resolved view in `cp15.md` ("C9,C1,0 / C9,C1,1"): base, virtual size
(`512 << N`), enable bit and load-mode bit. The physical sizes are 32 KiB (ITCM) and 16 KiB
(DTCM) (`cp15.md`, "C0,C0,0 / 1 / 2").

- **Region.** An address belongs to a TCM when it is inside `[base, base + virtual size)` and the
  TCM is enabled. The ITCM base is always 0.
- **Placement inside the region.** The TCM is mirrored through its region: the byte used is
  `(addr - base) mod physical size`. A virtual size larger than the physical one repeats the TCM;
  a smaller one uses only the first part of it (the offset never reaches the physical size, so
  the same formula holds). On the DS the firmware sets the ITCM size to 32 MiB, which mirrors
  the ITCM through `0000_0000h`-`01FF_FFFFh`.
- **Priority.** A TCM takes the access before main RAM or any other region, so it hides the
  memory underneath: GBATEK says TCM does not use the bus, and its load-mode text has a TCM
  answer at the same addresses as the main memory it overlays. If the two TCMs overlap, GBATEK
  says the ITCM "appears to have priority"; the model treats that as the rule (see
  "Unverified").
- **Disabled.** A disabled TCM is not there: the access reaches the memory underneath.
- **Load mode** (enabled, load-mode bit set). The TCM is write-only: a load reads the memory
  underneath and a store writes the TCM. GBATEK says it works for every load and store
  opcode except SWP and SWPB, which are not modeled (see "Not modeled").
- **Instruction fetches.** GBATEK says the DTCM serves data accesses only; the ITCM serves both
  code and data. So a fetch can hit the ITCM and never hits the DTCM: a fetch from an address
  inside the DTCM region reads the memory underneath. A load-mode ITCM is write-only, so a fetch
  from it also reads underneath (see "Unverified").
- **A CP15 change** takes effect for the next instruction. The model copies the TCM registers
  into the ARM9's bus view before each step; an opcode the pipeline already holds was fetched under
  the old mapping (see "Unverified").

### Unmapped addresses and I/O

At the ARM9, an I/O port that is not defined reads as zero (GBATEK, "Undefined I/O Ports"); a
write to it is ignored here. Other unmapped addresses are "undefined" and GBATEK gives no value:
a read returns 0 and a write is ignored. Guest accesses never fault.

## Edge cases

- The last byte of a mirror and the first byte after the 16 MiB block: `02FF_FFFFh` is main RAM,
  `0300_0000h` is not.
- A word access at `0200_0003h` uses the word at `0200_0000h`.
- A TCM virtual size above the physical size repeats the TCM; a size field below 3 or a base
  that is not aligned to its size is held as written (`cp15.md`) and the region follows it.
- A TCM region whose end is exactly `1_0000_0000h` (for example a 16 KiB DTCM at `FFFF_C000h`)
  serves its last word and byte. With an aligned base a region cannot reach past the top.
- A byte write to a TCM works (only VRAM, palette and OAM ignore byte writes, `vram-banks.md`).

## Not modeled

- **Load mode does not apply to SWP and SWPB** (GBATEK, "TCM Enable, TCM Load Mode"). GBATEK
  says only that; what a swap reads there is not stated (most likely the TCM like a normal
  access, which is an inference). The model cannot tell a swap from a load, because the `Bus`
  trait carries no access kind (#76), so a swap sees the load-mode rule. This is a known gap, not an open question. — `Cpu::swap` in
  `crates/core/src/cpu/arm_transfer.rs` carries `TODO(phase8)`.

## Unverified

- **Main RAM mirroring.** The mirror through the whole 16 MiB block is GBATEK's general rule
  ("usually", "typically"), not a statement about main RAM. — `Memory::build_page_table` in
  `crates/core/src/bus/memory/mod.rs` — verify by a test ROM on real hardware. The `memmap` ROM records it (slots 10-13, 15-17);
  running that ROM on hardware settles it.
- **Power-on RAM contents** are not stated; all RAM (main RAM, TCM, VRAM) starts at zero. —
  `Memory::new` — verify likewise.
- **Unmapped reads** return 0 and writes are ignored; GBATEK says "undefined" for memory and
  gives zero only for I/O ports. — `read_registers` in `bus/view.rs` — verify
  likewise. The `memmap` ROM records the I/O case (slot 32) only.
- **Write-only registers read.** VRAMCNT_A is marked write-only; a read returns 0 here. —
  `read_registers` in `bus/view.rs` — verify likewise.
- **Wider writes to a byte register.** VRAMCNT_A sits next to VRAMCNT_B; a halfword or word
  write there is split into bytes, each byte goes to the register at its own address, and a
  byte that lands on no register (VRAMCNT_B before it exists) is dropped. —
  `Access::write_registers` in `bus/view.rs` — verify likewise.
- **A fetch from the DTCM region** reads the memory underneath; a fetch from a load-mode ITCM
  reads underneath. GBATEK does not describe the fetch path in either case. —
  `Memory::fetch` in `bus/memory/mod.rs` — verify likewise.
- **ITCM priority over the DTCM.** GBATEK words it as "ITCM appears to have priority"; the model
  takes it as certain. — `Memory::load` and `Memory::store` in `bus/memory/mod.rs` — verify
  likewise.
- **I/O blocks are not mirrored.** The `04xx_xxxxh` block is decoded at its exact register
  addresses only, although GBATEK's general 16 MiB rule would suggest mirrors, and it says
  nothing specific for I/O. — `Access::write_registers` in `bus/view.rs` and `read_registers` in `bus/view.rs` —
  verify likewise.
- **Load-mode ITCM over the DTCM.** When the ITCM is in load mode and its region overlaps an
  enabled DTCM, a load at an overlapping address reads the memory underneath, not the DTCM:
  the ITCM claims the address. — `Memory::load` — verify likewise.
- **Opcodes already prefetched** when a CP15 write changes a TCM mapping keep the old mapping:
  the pipeline is not refilled. — `Machine::step_arm9` in `crates/core/src/machine/mod.rs` — verify
  likewise.
- **TCM corner cases** (size field outside 3-23, base not aligned to the size) follow
  `cp15.md`: the region is `[base, base + 512 << N)` as written. — `tcm_offset` in `bus/memory/mod.rs` —
  verify likewise.
- **Default DTCM base.** GBATEK names `027C0000h` as the default base, but does not say whether
  that is a hardware reset value or the value the firmware or software sets; CP15 starts at
  zero here (`cp15.md`, "Unverified"). Direct boot leaves CP15 at power-on and records the
  question in `direct-boot.md`, "Unverified". — no code site in this note.
