# CLAUDE.md

@AGENTS.md

The file above is the canon for this repository — the crate graph and core purity rules,
the clean-room and legal invariants (no dumps · no GPL source · spec first · trademarks),
the Apache-2.0 license rules, Rust conventions, and the test rules (GWT names, no control
flow, determinism, golden frames, the 90% coverage floor). What follows is only what matters on Claude Code.

## Subagents

- `architecture-guard` — catches crate-graph violations, impurity in ninge-core (IO, time,
  randomness, threads), shared or interior mutability, non-serializable state, `dyn` or
  allocation on the hot path, scheduler nondeterminism, and work past the current phase.
  Call it after touching `crates/` or any `Cargo.toml`.
- `cleanroom-auditor` — proves no dump, extracted data, or GPL-derived code entered the
  change, that every hardware behavior traces to a `docs/hw/` note, and that guesses are
  marked `TODO(hw-unverified)`. **Must be called** after touching `hle/`, `spi/` (synthetic
  firmware), `cart/`, `testroms/`, test fixtures, `docs/hw/`, the README, or anything that
  adds a binary file.
- `test-author` — writes tests that follow the rules (GWT names, no control flow, harness
  functions for tables, determinism, golden frames). Call it to put the failing test in place
  before an implementation, or when a change lacks coverage.

Run `cleanroom-auditor` and `architecture-guard` **in parallel, in one message**, and never
on the session that wrote the code — a fresh reviewer does not share the author's blind spots.

## Skills

- `verify` — run the whole gate (`./scripts/verify.sh`) and diagnose each failure.
- `add-hw-feature` — implement a hardware feature by plan §7.1. **Step 0 is the phase check
  and step 1 is the `docs/hw/` note** — before any test or code.
- `trace-bug` — investigate an emulation bug by plan §7.2: replay → trace diff → spec →
  failing test → fix.
- `commit` — commit-message, PR-title, and branch-name conventions
  (`ADD:`/`FIX:`/`REF:`/`DEL:`/`DOCS:`/`UPT:`) and the PR body rules. Read before committing.

## Hooks

`.claude/settings.json` wires six hooks; the bodies live in `scripts/agent_hooks/` and are
shared with Codex, Cursor, OpenCode, and Pi.

| Event | Action |
| --- | --- |
| `SessionStart` | Print the current phase (`docs/STATUS.md`) and the non-negotiables. |
| `PreToolUse` | `guard_cleanroom.py` — block BIOS/firmware dumps, reading or committing ROM/save bytes, GPL emulator source (checkouts and code-host URLs), and GPL notices written into code. |
| `PostToolUse` (Write/Edit) | `format_rust.py` — `rustfmt` the edited `*.rs`. |
| `PostToolUse` (Write/Edit) | `check_architecture.py` — block core impurity, `unsafe` without `SAFETY:`, a missing Apache-2.0 SPDX line or package license, crate-graph and naming violations. |
| `PostToolUse` (Write/Edit) | `check_tests.py` — block non-GWT test names, control flow in a test body, a test that asserts nothing. |
| `PostToolUse` (Write/Edit) | `check_language.py` — block non-English (non-Latin script) text outside an approved `-{code}` translation. |

If a hook returns `block` or `deny`, fix the reported problem and retry — do not work
around it. Hooks are best-effort (they only fire on tool edits), so `./scripts/verify.sh`
and CI are the real gate; the clean-room guard fails closed.

## Tool use

- Run `./scripts/verify.sh` before saying a task is done.
- Delegate broad exploration to an Explore subagent and take only the conclusion.
- Enter plan mode before a change to the crate graph, the savestate format, the scheduler
  model, or any other design decision — and record the decision in a GitHub issue
  labeled `decision` (AGENTS.md §6).
- Commit only when the user asks.
