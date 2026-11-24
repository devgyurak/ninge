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
