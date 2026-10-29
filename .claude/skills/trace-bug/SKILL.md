---
name: trace-bug
description: Investigate an emulation bug in ninge by the plan §7.2 procedure — a game or test ROM that freezes, draws wrong, sounds wrong, or diverges. Use when the user reports a bug, a hang, a glitch, a wrong frame, or asks why something behaves differently from hardware or a reference emulator. Enforces replay → trace diff → spec check → failing test → fix.
---

# Investigate a bug (plan §7.2)

The order matters: a fix without a pinned reproduction and a failing test is a guess, and
guesses are how emulators regress.

## 1. Pin the reproduction with an input replay

Make the bug happen the same way every time: the ROM, the input log, the frame number. The
core is deterministic, so a replay that reproduces once reproduces always — if it does not,
**that** is the bug (a determinism leak; see `verify`, determinism failures).

- Prefer our own test ROM. Write a minimal one in `testroms/` that triggers the same hardware
  path if you can.
- For a commercial game, the user runs it locally; you work from the replay, traces, and the
  emulator's own output. Do not read the ROM's bytes (AGENTS.md §3.1).

## 2. Find the first divergence

Run a trace diff against the reference emulator used **as a black-box oracle**: a melonDS
release binary driven over the GDB remote protocol (NooDS's binary if melonDS's stub cannot
reach both CPUs). Same ROM, same inputs; ninge-runner writes the line-oriented trace (CPU,
step, PC, opcode, r0–r15, CPSR) and the oracle's trace is collected in the same format.
Compare **per CPU** — ARM9/ARM7 interleaving differs between emulators by design — and find
the first point where state differs. Running the oracle's binary is fine; reading its source
is not (AGENTS.md §3.3).

Until `ninge-tracediff` exists (parallel track), do it by hand: narrow with frame hashes, then
with register dumps at event boundaries, then instruction by instruction. The scheduler event
log and the debugger panels are the tools — if they cannot show the state you need, that is
worth fixing too.

The first divergence is the bug's location. Everything after it is a consequence; do not
"fix" a consequence.

## 3. Check the spec at the divergence

Find the hardware feature at that point in `docs/hw/`. If there is no note, write it first
(the `add-hw-feature` skill, step 1). Compare the note, the code, and the oracle:

- note and oracle agree, code differs → our bug;
- note is silent or unsure → the behavior is Unverified: record what the oracle shows as an
  observation, not as fact, and mark the site `TODO(hw-unverified)`;
- note and oracle disagree → stop and report; do not pick one silently.

## 4. Failing test first, then the fix

Add the reproduction as a test that fails now (`test-author`): a unit test for an
instruction, a test ROM with a golden frame or memory condition for machine behavior. Name it
for the behavior, not the game. Then fix, and see it pass.

## 5. Verify and report

`./scripts/verify.sh`, then `architecture-guard` and `cleanroom-auditor` on fresh sessions.
The PR (`FIX: <behavior>`) states the first divergence, the hardware rule that was broken, the
test that pins it, and every golden hash the fix changes, with the reason.
