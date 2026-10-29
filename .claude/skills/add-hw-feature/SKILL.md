---
name: add-hw-feature
description: Implement or extend an emulated hardware feature in ninge (a CPU instruction group, a register block, IPC, DMA, timers, a GPU mode, SPU, cart, SPI, an HLE SWI) by the plan §7.1 procedure. Use whenever the user asks to implement, add, support, or emulate a piece of hardware. Enforces phase check → docs/hw note in our own words → failing test → implementation in ninge-core → debugger exposure → verify → decision issue.
---

# Implement a hardware feature (plan §7.1)

Do the steps in order. Skipping straight to code is the failure this procedure exists to
prevent: behavior with no note cannot be proven clean-room, and code with no failing test
cannot be proven right.

## 0. Is it in this phase?

Read `docs/STATUS.md`. If the feature belongs to a later phase, implement only what the
current phase needs and leave a stub with `TODO(phaseN)` (plan §8 rule 9). If you cannot tell
which phase owns it, ask.

## 1. Write the spec note — in your own words

`docs/hw/<feature>.md`, from the template in `docs/hw/README.md`.

- Sources: **GBATEK** first, then the ARM ARM (ARMv5TE) and the ARM7TDMI / ARM946E-S TRMs. A
  blog post or talk may supply an idea; write it as a hardware fact.
- **Never** open GPL emulator source (AGENTS.md §3.3) — the guard hook blocks it — and never
  read a BIOS or firmware dump (§3.2).
- Record: register addresses, widths, bit fields, reset values, behavior step by step, timing
  where known, edge cases.
- **Unverified** gets its own section: anything GBATEK leaves open or that you inferred, with
  the code site that will carry `TODO(hw-unverified)` and how it could be verified.

## 2. The failing test first

Delegate to `test-author` or follow `AGENTS.md` §5 yourself:

- **CPU**: unit tests and the JSON-suite harness. Every v5TE extension gets its own
  self-written test (plan §5, Phase 1 DoD).
- **Anything observable on the machine**: a bare-metal test ROM in `testroms/` (no libnds
  init before Phase 3 — it waits on ARM7 and IPC), with its expected result — a golden frame
  hash of both screens after N frames, or a memory-dump condition.
- Expected values come from the note, never from running our implementation.

Run it and **see it fail** for the reason you expect.

## 3. Implement in ninge-core

In the module the plan assigns (`cpu/`, `bus/`, `scheduler/`, `gpu2d/`, `gpu3d/`, `spu/`,
`cart/`, `spi/`, `hle/`, `debug/`). Check it against AGENTS.md §2:

- state lives in `Machine`, is plain serde data, and uses no hash container;
- no IO, clock, randomness, threads, `dyn`, or allocation on the hot path;
- guest-controlled values never panic — mask, mirror, or open-bus as the note says;
- register bits are named, and the doc comment cites `docs/hw/<feature>.md`;
- every Unverified item from the note has `TODO(hw-unverified)` at its site;
- a timed effect is a scheduler event with a deterministic tie-break;
- bank or ownership changes invalidate the page table.

## 4. Expose it to the debugger

New state (a register block, a FIFO, a mapping, a pending event) is readable through the
`debug/` hooks so a debugger panel can show it (plan §7.1 step 4). If the panel does not
exist yet, the PR says which panel will show it and in which phase.

## 5. Verify

`./scripts/verify.sh` (the `verify` skill). Then, in one message and on fresh sessions:
`architecture-guard` and `cleanroom-auditor`.

## 6. Record the decision

If you made a design decision that is expensive to reverse (a state layout, a timing model,
a fast-path trick, an `unsafe`), it goes into a GitHub issue labeled `decision` from
`.github/ISSUE_TEMPLATE/decision.md`, and the PR links it. Draft the issue only when the
user asks; deciding it is theirs.

## PR

One PR = this one feature (plan §7.3). Title `ADD: <feature>`; the body names the
`docs/hw/` note, the tests that pin it, any Unverified items, and every golden hash added or
changed with the reason.
