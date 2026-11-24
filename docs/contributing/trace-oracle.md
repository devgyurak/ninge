# The trace oracle

Reference detail behind `AGENTS.md` §6 (investigating a bug, plan §7.2).

Decided: the oracle is a melonDS release binary driven over the GDB remote protocol — a
standard protocol, so no source is read. NooDS's binary is the fallback if melonDS's stub
cannot reach both CPUs. Each developer installs it; it is never vendored or redistributed.
Running a reference emulator's *binary* is fine; reading its source is not (`AGENTS.md` §3.3).

Traces are line-oriented text, one instruction per line (CPU, step, PC, opcode, r0–r15,
CPSR), written by ninge-runner. ARM9/ARM7 interleaving differs between emulators by design, so
streams are compared **per CPU**, narrowing from frame hashes to event boundaries to
instructions.

## The trace format

A trace is UTF-8 text, written by `ninge-runner trace`, produced live by the oracle adapter in
`ninge-tracediff` (which compares it in memory and writes no file), and parsed by `ninge_core::trace` (pure types plus encode and parse, no IO, so
the runner can write what the diff tool reads without a dependency between them).

```
ninge-trace 1
# free-form comment
<cpu> <step> <pc> <opcode> <r0> <r1> ... <r15> <cpsr>
```

- The first line is the header `ninge-trace 1`. A reader refuses another version.
- A line that starts with `#` is a comment and a blank line is ignored.
- One line per executed instruction, columns separated by one space, all numbers lowercase hex
  with no prefix and a fixed width: `cpu` is `9` or `7`; `step` is 8 digits; `pc`, the registers
  and `cpsr` are 8 digits; `opcode` is 8 digits in ARM state and 4 in Thumb (CPSR bit 5).
- Registers are the state **before** the instruction runs, as the current mode sees them.
  `r15` is the address of the instruction itself, not the pipeline value (`pc + 8` or `pc + 4`),
  so it equals the `pc` column.
- `step` counts that CPU's recorded instructions from 0. The two CPUs are compared separately:
  how ARM9 and ARM7 interleave differs between emulators by design.

## BIOS collapsing

ninge runs an HLE BIOS: at a BIOS address the machine does the BIOS's work in Rust and no
instruction executes there. The oracle runs real or free BIOS code. To make the streams
comparable, **BIOS-region execution leaves no line on either side**: the oracle adapter runs
until the PC leaves the region and then resumes recording. The regions are the ARM9 BIOS at
`FFFF0000h` (32 KiB, `arm9-memory-map.md`) and the ARM7 BIOS at `00000000h` (16 KiB,
`hle-swi.md`, "The BIOS area").

The adapter never reads memory in a BIOS region: an opcode fetched there would be BIOS data,
and no BIOS byte may leave the oracle (`AGENTS.md` §3.2). Use the oracle's built-in free BIOS
and firmware, not a dump.

## Narrowing

The oracle sits behind GDB, so it can report registers and memory but not a frame hash. Narrowing
has three levels:

1. **Frame.** A person picks the frame where the symptom shows, from the runner's frame hashes or
   from looking at the screen. The runner can start tracing at that frame, but the oracle runs from boot, so a trace
   started at frame F lines up with it only if the oracle is brought to the same moment (an
   event-boundary hit does that); `live` and `boundaries` on a trace that starts later than boot
   compare different moments.
2. **Event boundary.** At the k-th hit of an address breakpoint (the PC is given with
   `--breakpoint`; the user IRQ handler's entry is the usual choice) both sides' registers are compared; the first boundary that
   differs is found.
3. **Instruction.** From the last agreeing boundary both sides step one instruction at a time
   until the first divergence.

## Divergence classes

IRQ timing is approximate by design (`AGENTS.md` §2 rule 7), so the report separates:

- **timing**: the two pcs part and the two sides are in different modes: an IRQ is taken at a
  different instruction. Not a bug by itself. (Both sides in the same mode, say a return from
  an IRQ to the wrong address, is a mismatch.)
- **after BIOS call**: only r0-r3 and r12 differ right after a collapsed BIOS call, where the
  two BIOS implementations may legitimately leave scratch registers different.
- **mismatch**: anything else (PC, opcode, a register, CPSR).

## Running the oracle

The maintainer starts the oracle by hand with its GDB stub enabled and gives the ARM9 and ARM7
ports; an agent connects only to `127.0.0.1:<port>`. The clean-room guard blocks any command
that names a GPL emulator's path, and that stays as it is.

## Traces of a user's ROM

A trace of a commercial ROM contains ROM bytes (the opcode column, and any memory the adapter
reads). Run the tool on a user's ROM only yourself, locally. Never commit such a trace, paste it
into an issue or a PR, or give it to an agent. Traces of our own test ROMs are fine to read,
print and use as test data.

## Oracle probe findings

Pending: the melonDS stub's behavior (both CPUs reachable, register layout, step,
breakpoints, reset, throughput) is recorded here after the probe in the next stack step.
