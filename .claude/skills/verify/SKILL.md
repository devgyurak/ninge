---
name: verify
description: Run ninge's full quality gate (./scripts/verify.sh) in order and diagnose failures. Use before finishing a task, before committing, or whenever the user asks to verify, run the gate, check that it passes, or run the tests. Covers the agent-config check, hook selftest, clean-room scan, architecture, test conventions, language, fmt, clippy, and tests, with each failure's usual cause.
---

# Run the gate

**`./scripts/verify.sh` must pass for the task to be done.** It runs every step in order and
stops at the first failure — a fix in an earlier step changes a later step's result, so fix
and start over.

```bash
./scripts/verify.sh
```

What it runs, in order:

| # | Step | Command |
|---|---|---|
| 1 | Agent config | `python3 scripts/check_agent_config.py` |
| 2 | Hook selftest | `python3 scripts/agent_hooks/selftest.py` |
| 3 | Clean-room scan | `python3 scripts/agent_hooks/guard_cleanroom.py --scan-tracked` |
| 4 | Architecture | `python3 scripts/agent_hooks/check_architecture.py --all` |
| 5 | Test conventions | `python3 scripts/agent_hooks/check_tests.py --all` |
| 6 | Language | `python3 scripts/agent_hooks/check_language.py --all` |
| 7 | Format | `cargo fmt --all --check` |
| 8 | Lint | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| 9 | Dependency licenses | `cargo deny --locked check licenses bans sources` |
| 10 | Tests + coverage ≥ 90% | `cargo llvm-cov nextest --workspace --locked --fail-under-lines 90` |
| 11 | Doc tests | `cargo test --workspace --locked --doc` |

Until Phase 0 adds `Cargo.toml`, steps 7–11 print `SKIPPED`. Once it exists, a missing
`cargo-deny` or `cargo-llvm-cov` fails the gate with the install command. Say that when you report — the
Rust gates did not pass, they did not run.

## Diagnosing a failure

### 1. Agent config
- *body differs from the claude body* — a subagent was edited in one copy. Edit
  `.claude/agents/<name>.md` and make `.opencode/agents/` and `.codex/agents/` match exactly.
- *list differs from the hook* — AGENTS.md §2 (core-forbidden crates) or §3.3 (GPL emulators)
  no longer matches the hook's set. Change both together.
- *does not mention skill/subagent* — add it to `CLAUDE.md` and `.agents/README.md`.

### 2. Hook selftest
A hook changed behavior. Read the failing case's name: it states the decision that was
expected. Fix the hook, not the case — unless the case is wrong, in which case say why.

### 3. Clean-room scan
A ROM, save, dump, GPL-named directory, or GPL notice is tracked. **Do not just delete it in
a new commit** if it was ever pushed: history keeps it. Stop and tell the human.

### 4. Architecture
- *line 1 must carry `SPDX-License-Identifier: Apache-2.0`* — add the header comment
  (`// …` in Rust and C, `@ …` in ARM assembly).
- *license is missing / not Apache-2.0* — `license.workspace = true` in the package.
- *in ninge-core: std IO / clock / printing* — move the IO to ninge-runner or the frontend (`ninge`)
  and pass bytes or values in.
- *shared or interior mutability / thread-shared state* — give the state to `Machine` and pass
  `&mut`.
- *hash container* — `BTreeMap`, an array indexed by a small id, or a sorted `Vec`.
- *dyn* — make it generic, or an enum of the known cases.
- *raw pointer* — an index or offset into an owned buffer.
- *`unsafe` without `SAFETY:`* — write the soundness argument, and cite the benchmark in the PR.
- A `// PURITY-OK: <reason>` waiver is for a reason a reviewer will accept, not for convenience.

### 5. Test conventions
- *not given_…_when_…_then_…* — rename to the behavior; the `then_` part is observable.
- *control flow in the test body* — move the loop into a harness function that returns the
  failures; assert the list is empty.
- *asserts nothing* — add the assertion, or `#[should_panic(expected = "...")]`.

### 6. Language
Non-English text outside an approved `-{code}.md` translation — write it in English. A test
that needs a non-Latin character writes it as an escape (`'\u{AC00}'`). A translation with no
English original — add the original, or remove the translation.

### 7–8. fmt / clippy
`cargo fmt --all` fixes format. For clippy, fix the code; a justified exception is
`#[expect(clippy::<lint>, reason = "...")]` on the item, never a crate-wide `allow`.

### 9. cargo-deny
A dependency's license is not on the `deny.toml` allow-list. Pick another crate. Widening the
list is a `decision` issue, never part of the change that needed it.

### 10. Tests and coverage
- **Coverage below 90%** — **do not lower the floor or exclude files**; write tests that
  assert behavior (delegate to `test-author`). `cargo llvm-cov --html` shows the uncovered
  lines. `scripts/check_agent_config.py` fails if the floor in `verify.sh` drifts from
  AGENTS.md §5.9.
- **A golden frame mismatch is a regression until shown otherwise.** Do not regenerate the
  hash. Find which hardware behavior changed; if the new frame is right, the PR says why.
- **A pending insta snapshot** — review each one (`cargo insta review`); never
  `cargo insta accept` in bulk.
- **A proptest failure** — keep the `proptest-regressions/` entry and commit it; fix the code.
- **A determinism failure** (two runs differ) — look for a hash container, a host clock, a
  float, or a scheduler tie ordered by heap internals.
- **A JSON-suite failure** — compare against the case's expected state and the `docs/hw/`
  note; the suite is the reference, not our output.

## Last

Report what you ran and the result, factually, including the steps that were skipped. **Do
not claim it passed if it did not.**
