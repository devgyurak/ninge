# Disassembly syntax

Sources: GBATEK "ARM Instruction Summary", "ARM Opcodes: Data Processing (ALU)", "ARM Opcodes:
PSR Transfer (MRS, MSR)", "ARM Opcodes: Memory: Single Data Transfer (LDR, STR, PLD)", "ARM
Opcodes: Memory: Halfword, Doubleword, and Signed Transfer", "ARM Opcodes: Memory: Block Data
Transfer (LDM, STM)", "ARM Opcodes: Branch and Branch with Link (B, BL, BX, BLX)", "ARM Opcodes:
Coprocessor Instructions", "ARM Opcodes: Exceptions (SWI, BKPT)", "THUMB Instruction Summary" and
the per-format Thumb pages; ARM ARM (ARMv5TE) assembler syntax for the same instructions.
Used by: `crates/core/src/cpu/disasm.rs`, `crates/core/src/cpu/disasm_thumb.rs`   Phase: 1 (#16)

A disassembler has no hardware behavior of its own. Encodings are the hardware's and live in
the notes linked below; **everything about how the text looks is a notation convention** chosen
here (GBATEK's pre-UAL mnemonics, which predate unified assembler language). Nothing in this
note is observable on hardware; a different convention would not change any emulated result.

Encodings are defined in `arm-data-processing.md`, `arm-barrel-shifter.md`, `arm-multiply.md`,
`arm-load-store.md`, `arm-branch-pipeline.md`, `arm-exceptions-modes.md`, `thumb.md`,
`armv5te.md` and `cp15.md` (coprocessor field layout). This note says only how a decoded word
is written.

## The decoder decides

The text is produced after the instruction decoder has classified the word (`Kind` for ARM,
`ThumbKind` for Thumb) on the processor being disassembled. A word the decoder does not give a
handler to — `Unhandled` or `Undefined` — is written `undefined 0x…` with the whole word in
lowercase hex, zero-padded to the word width (8 digits for ARM, 4 for Thumb), without a
condition. Because ARMv5TE additions decode as `Unhandled` on the ARM7, the same word reads as
`clz r2, r1` for the ARM9 and as `undefined 0xe16f2f11` for the ARM7. The output therefore never
claims an instruction the processor would not run as that instruction.

## Common notation (conventions)

- Everything is lowercase. R13, R14, R15 are `sp`, `lr`, `pc`; the others `r0`–`r12`.
- Data immediates, memory offsets, `swi`/`bkpt` numbers: lowercase hex with `#0x` and no
  padding (`#0x1`, `#0xff000000`); a negative memory offset is `#-0x4`. A rotated immediate is
  shown after the rotation. `swi` and `bkpt` take a bare `0x…` (no `#`).
- Shift amounts and coprocessor numbers/opcodes/registers are decimal (`lsl #2`, `p15`, `c1`).
- Branch targets and the PC-relative load comment are absolute addresses, 8 hex digits
  (`0x02000100`), computed with wrapping arithmetic from the instruction address: PC = address
  + 8 in ARM state, + 4 in Thumb state.
- A PC-relative load or preload (`ldr`, `ldrb`, `ldrh`, … or `pld` with an immediate offset,
  base `pc`, pre-indexed) is followed by ` ; 0x…`, the address it reads, so
  `ldr r0, [pc, #0x10] ; 0x02000118`. Thumb `ldr r0, [pc, #imm]` uses the word-aligned PC
  (`thumb.md`, "Edge cases").
- Register lists are written `{…}` in register-number order. A run of three or more
  consecutive registers is a range (`r0-r3`, `r4-lr`, `r11-pc`); one or two stay single. An
  empty list is `{}`.

## ARM

Mnemonics follow GBATEK's pre-UAL order: **mnemonic, condition, then the letters** (`s`, `b`,
`h`, `sb`, `sh`, `d`, `t`, `l`, or the block mode). `eq ne cs cc mi pl vs vc hi ls ge lt gt le`;
`al` (1110) is not written. Condition 1111 on the ARM7 — which decodes by the ordinary table and
never executes (`armv5te.md`, "Unconditional space") — is written `nv`. On the ARM9 the 1111 space
is only BLX immediate and PLD (both without a condition); every other word there is
`undefined`.

| Kind | Text |
|---|---|
| Data processing, `and eor sub rsb add adc sbc rsc orr bic` | `add{cond}{s} rd, rn, op2` |
| `mov mvn` | `mov{cond}{s} rd, op2` |
| `tst teq cmp cmn` | `cmp{cond} rn, op2` (the S bit is implied; no `s`, no Rd) |
| op2, immediate | `#0x…` (rotated value) |
| op2, register shifted by immediate | `rm`, `rm, lsl #n`, `rm, lsr #n`, `rm, asr #n`, `rm, ror #n`, `rm, rrx` |
| op2, register shifted by register | `rm, lsl rs` (also lsr, asr, ror) |
| MRS | `mrs{cond} rd, cpsr` or `spsr` |
| MSR | `msr{cond} cpsr_fc, rm`, `msr{cond} spsr_f, #0xf0000000` |
| MUL, MLA | `mul{cond}{s} rd, rm, rs`; `mla{cond}{s} rd, rm, rs, rn` |
| Long multiply | `umull{cond}{s} rdlo, rdhi, rm, rs`; also `umlal smull smlal` |
| SWP | `swp{cond}{b} rd, rm, [rn]` |
| LDR, STR, byte, T | `ldr{cond}{b}{t} rd, addr` (`str` likewise) |
| LDRH, STRH, LDRSB, LDRSH | `ldr{cond}h`, `str{cond}h`, `ldr{cond}sb`, `ldr{cond}sh` with `addr` |
| LDRD, STRD (ARM9) | `ldr{cond}d rd, addr`; `str{cond}d` |
| LDM, STM | `ldm{cond}ia rn!, {list}^` (modes `ia ib da db`; `!` for W; `^` for S) |
| B, BL | `b{cond} 0x…`, `bl{cond} 0x…` |
| BX | `bx{cond} rm` |
| BLX register (ARM9) | `blx{cond} rm` |
| BLX immediate (ARM9) | `blx 0x…` (target + 2 when H, bit 24, is set; no condition) |
| CLZ (ARM9) | `clz{cond} rd, rm` |
| Saturating (ARM9) | `qadd{cond} rd, rm, rn`; `qsub qdadd qdsub` |
| Signed multiplies (ARM9) | `smla{x}{y}{cond} rd, rm, rs, rn`; `smulxy rd, rm, rs`; `smlawy` / `smulwy`; `smlalxy rdlo, rdhi, rm, rs`. `x`, `y` are `b` or `t` |
| BKPT (ARM9) | `bkpt{cond} 0x…` |
| PLD (ARM9) | `pld [rn, #0x20]`, `pld [rn, -rm, lsl #2]` (no condition) |
| SWI | `swi{cond} 0x…` |
| CDP | `cdp{cond} p15, 0, c1, c2, c3, 4` |
| MCR, MRC | `mcr{cond} p15, 0, r0, c1, c0, 0`; `mrc` likewise |
| MCRR, MRRC | `mcrr{cond} p15, 0, rd, rn, c1`; `mrrc` likewise |
| LDC, STC | `ldc{cond}{l} p15, c1, addr` (`l` for N, bit 22); `stc` likewise |

Notes on the forms:

- **Shifts of #0** keep their special meaning from `arm-barrel-shifter.md`: an immediate `lsr`
  or `asr` of 0 is written `#32`, `ror` of 0 is `rrx`, and `lsl #0` writes no shift at all
  (`movs r0, r1`). The same rules apply to a shifted register offset.
- **Addressing, `addr`.** Pre-indexed: `[rn]` (offset +0, no writeback), `[rn, #±0x…]`, `[rn, ±rm]`,
  `[rn, ±rm, shift]`, followed by `!` when W is set. Post-indexed: `[rn], #±0x…`, `[rn], ±rm`,
  `[rn], ±rm, shift`. A word load/store with P = 0 and W = 1 gets the `t` letter. Offset −0 is
  `#-0x0`. The halfword/doubleword forms take no shift. LDC/STC offsets are the word offset ×4;
  the unindexed form (P = 0, U = 1, W = 0) is `[rn], {option}` with the option in decimal.
- **The T form of the halfword transfers does not exist**; a halfword word with P = 0 and W = 1
  is written as an ordinary post-indexed access, the W bit not shown (it should be zero).
- **Coprocessor numbers** are the word's bits 11-8; no coprocessor is looked up. On the ARM9,
  MCRR/MRRC (bits 27-20 = 0xC4 / 0xC5) take precedence over the LDC/STC words that share them;
  they are ARMv5TE additions, so the ARM7 reads those words as STC/LDC.
- **LDC/STC with P = 0 and W = 0** have no defined addressing mode except the unindexed one
  (U = 1); the form is written as unindexed, `[rn], {option}`, whatever U is. A post-indexed
  access needs W.
- **TEQP-style Rd = 15 forms** are not given a special letter; `teq` with S implied is written
  as above.
- **MSR** field mask letters are in the order `f s x c` (bits 19, 18, 17, 16); the mask 0 is
  written `cpsr_` with no letters.
- **Condition on BKPT** is printed when it is not AL, as the decoder runs it conditionally
  (see "Unverified").

### Branch targets

`branch_target()` is `Some` for B, BL and BLX immediate: address + 8 + sign-extended offset24 × 4,
plus 2 for BLX when H = 1, wrapping at 32 bits. Every other instruction, including BX and BLX
register (the target is a register), is `None`.

## Thumb

Pre-UAL Thumb: no `s` suffix, shifts and moves use the same mnemonics as ARM.

| Format | Text |
|---|---|
| 1 | `lsl rd, rs, #n`; `lsr`, `asr` (n written as the 5-bit value, with 0 written `#32` for `lsr` and `asr`) |
| 2 | `add rd, rs, rn`, `sub rd, rs, rn`, `add rd, rs, #0x3`, `sub rd, rs, #0x3` |
| 3 | `mov rd, #0x1`, `cmp`, `add`, `sub` |
| 4 | `and eor lsl lsr asr adc sbc ror tst neg cmp cmn orr mul bic mvn rd, rs` |
| 5 | `add rd, rs` / `cmp` / `mov` over r0–r15; `bx rs`; on the ARM9 with H1 = 1 `blx rs` |
| 6 | `ldr rd, [pc, #0x10] ; 0x02000118` |
| 7 | `str`, `strb`, `ldr`, `ldrb` with `[rb, ro]` |
| 8 | `strh`, `ldsb`, `ldrh`, `ldsh` with `[rb, ro]` |
| 9, 10, 11 | `str`/`ldr`/`strb`/`ldrb`/`strh`/`ldrh` with `[rb, #0x…]` (offset in bytes), or `[sp, #0x…]` |
| 12 | `add rd, pc, #0x…`, `add rd, sp, #0x…` |
| 13 | `add sp, #0x…`, `sub sp, #0x…` |
| 14 | `push {list}`, `pop {list}` (R adds `lr` or `pc`) |
| 15 | `stmia rb!, {list}`, `ldmia rb!, {list}` |
| 16 | `beq 0x…` (conditions as ARM, including `bal` for 1110) |
| 17 | `swi 0x…` |
| 18 | `b 0x…` |
| 19 | below |
| BKPT (ARM9) | `bkpt 0x…` |

- A zero offset is written `[rb]`, as in ARM.
- Branch targets: address + 4 + sign-extended offset × 2, wrapping.

### The BL pair

A Thumb BL is two halfwords executed separately (`thumb.md`, "Long branch with link"). The
disassembler is given the halfword, the **next** halfword if there is one, and the address:

- A **prefix** followed by a **suffix** (`11111 off11`) prints the whole instruction on the
  prefix line: `bl 0x…`, the target being address + 4 + (sign-extended off11 << 12) + (suffix
  off11 << 1). On the ARM9 a prefix followed by the BLX suffix (`11101 off11`) prints
  `blx 0x…`, and the target has bits 1–0 cleared (`armv5te.md`, "BLX"). `branch_target()` is
  `Some` for those two lines.
- A suffix prints `(bl suffix)`; the BLX suffix prints `(blx suffix)`. The disassembler is not
  told what came before, so a suffix reached on its own and one that completes a prefix read
  the same. `branch_target()` is `None`.
- A prefix with no usable next halfword (`None`, or one that is not a suffix, or a BLX suffix
  on the ARM7 where that encoding is `Unhandled`) prints the partial form
  `(bl prefix lr=0x…)`, the address the prefix leaves in LR (address + 4 + sign-extended
  off11 << 12). `branch_target()` is `None`.

## Edge cases

- PC-relative reads use the pipeline offset (+8 ARM, +4 Thumb) at the *instruction's* address.
- A destination of `pc` (`mov pc, lr`, `ldr pc, [sp], #0x4`) is written like any other.
- Wrapping: a branch near the top of the address space wraps to the bottom.
- **A branch that never runs still has a target.** On the ARM7, condition 1111 is written `nv`
  and `branch_target()` is still `Some` for B and BL, because the target is a fact of the
  encoding. A consumer that draws branch arrows (the debugger's Disassembly panel, #56) should
  skip the `nv` ones.
- `ldm`/`stm` with `^` and `!` together prints both: `ldmia r0!, {r1-r3, pc}^`.

## Unverified

The notation is convention, not hardware, so there is nothing to verify against hardware. These
items are the places where this note guesses what a disassembler *should* say about a word whose
meaning hardware leaves open:

- **BKPT with a condition other than AL.** The decoder runs it conditionally and the text shows the
  condition; `armv5te.md` leaves open whether hardware does. — `disasm.rs`, `Kind::Breakpoint` —
  verify by a test ROM on real hardware.
- **Thumb format 16 with condition 1110.** Written `bal`; GBATEK calls it undefined and the
  suite shows an unconditional branch (`thumb.md`, "Unverified"). — `disasm_thumb.rs`,
  `ThumbKind::ConditionalBranch` — same.
- **Words the decoder leaves `Unhandled` that hardware may treat as defined** (the ARM9's
  1111 space, the ARMv6 UMAAL space, `thumb.md`'s 1011 space). They are `undefined 0x…` because
  the decoder is the authority here. — the `TODO(hw-unverified)` markers sit in `decode.rs` and
  `thumb_decode.rs`, not in the disassemblers, which only follow them; this resolves when the
  decoder's own Unverified items do.
- **Coprocessor words on the ARM7.** The decoder reports `Coprocessor`, but the ARM7 traps them as
  undefined (`arm-exceptions-modes.md`); the text is still the general coprocessor form. —
  `disasm.rs`, `Kind::Coprocessor`.
