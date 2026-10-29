# Hardware notes

One note per hardware feature, written **in our own words** from GBATEK (plan §7.1 step 1).
Code in `ninge-core` implements what a note says; a behavior with no note is not
implemented yet. These notes are also the clean-room boundary: they carry *facts about the
hardware*, never code or structure from another emulator (AGENTS.md §3).

File name: `docs/hw/<feature>.md`, kebab-case (`ipc-fifo.md`, `vram-banks.md`,
`arm-barrel-shifter.md`).

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
