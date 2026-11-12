# Hardware notes

One note per hardware feature, written **in our own words** from GBATEK (plan §7.1 step 1).
Code in `ninge-core` implements what a note says; a behavior with no note is not
implemented yet. These notes are also the clean-room boundary: they carry *facts about the
hardware*, never code or structure from another emulator (AGENTS.md §3).

File name: `docs/hw/<feature>.md`, kebab-case (`ipc-fifo.md`, `vram-banks.md`,
`arm-barrel-shifter.md`). A new note also gets a line in the index below.

## Notes

**CPU**

- [CPU registers and modes](cpu-registers.md)
- [ARM data processing and PSR transfer](arm-data-processing.md)
- [ARM barrel shifter](arm-barrel-shifter.md)
- [ARM multiply and long multiply](arm-multiply.md)
- [ARM loads and stores](arm-load-store.md)
- [ARM branches, BX, and the prefetch pipeline](arm-branch-pipeline.md)
- [ARM exceptions, mode entry, and the vector base](arm-exceptions-modes.md)
- [Thumb instruction set (ARMv4T)](thumb.md)
- [ARMv5TE extensions (ARM9)](armv5te.md)
- [CP15, the ARM9 system control coprocessor (minimal)](cp15.md)
- [Disassembly syntax](disassembly.md)

**Memory**

- [Memory map](memory-map.md)
- [ARM9 memory map: main RAM, TCM, unmapped space](arm9-memory-map.md)
- [Shared WRAM and WRAMCNT](wramcnt.md)
- [VRAM banks (bank A: LCDC mode and engine A BG)](vram-banks.md)
- [Palette RAM](palette-ram.md)

**System**

- [Scheduler: clocks, frame timing, and CPU interleaving](scheduler.md)
- [Interrupt controller (IME, IE, IF)](interrupts.md)
- [Inter-process communication (IPCSYNC, IPC FIFO)](ipc.md)
- [Timers](timers.md)
- [DMA](dma.md)
- [Hardware divider and square root](divsqrt.md)
- [Keypad](keypad.md)

**Video**

- [Display: the two screens](display.md)
- [Display timing: lines, VBlank, HBlank, DISPSTAT and VCOUNT](display-timing.md)
- [Engine A display mode 2 (VRAM display) and the screen swap](display-mode-vram.md)
- [Engine A text backgrounds](bg-text.md)
- [Background modes, affine layers and mosaic (both engines)](bg-modes.md)
- [Sprites (OBJ): OAM, tile, affine and bitmap objects, priority, mosaic, line limit](obj.md)
- [Extended palettes (BG and OBJ, both engines)](extended-palettes.md)
- [Windows: WIN0, WIN1, the OBJ window and the outside region (both engines)](windows.md)

**Boot and BIOS**

- [Cartridge header](cart-header.md)
- [Direct boot (ARM9)](direct-boot.md)
- [HLE BIOS: the BIOS area, the vectors, SWI entry and the IRQ dispatcher](hle-swi.md)

## Template

```markdown
# <Feature>

Sources: GBATEK "<section title>"; <ARM ARM / TRM section, if any>.
Used by: `crates/core/src/<module>/…`   Phase: <N>

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_0180 | IPCSYNC | 16 | R/W | 0 |

### <Register> bits

| Bits | Meaning |
|---|---|
| 0-3 | … |

## Behavior

What the hardware does, step by step, in plain sentences. Timing in ARM7/ARM9 cycles where
known.

## Edge cases

- What happens on the unusual input (full FIFO, zero divisor, unaligned access, a write while
  disabled, …).

## Unverified

Anything GBATEK leaves open, contradicts, or we inferred. Each item names the code site that
carries `TODO(hw-unverified)` and how it could be verified (a test ROM on real hardware, a
trace against the reference emulator used as an oracle).

- <claim> — `crates/core/src/…` — verify by …
```

## Rules

- **Own words.** Summarize; do not paste GBATEK tables wholesale, and never paste another
  emulator's code, comments, or structure.
- **Unverified is a section, not a shrug.** If you are not sure, write it here and leave
  `TODO(hw-unverified)` at the code site (plan §8 rule 8). Guessing silently is the failure.
- **Cite the section.** A reader must be able to find the source paragraph.
- **One feature per note.** A note grows with the feature across phases; mark what each
  phase implements.
