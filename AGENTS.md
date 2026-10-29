# AGENTS.md — ninge

**ninge** (said "ninji"; a twist on *hinge*, the joint between two screens) is a
**terminal-native emulator for a dual-screen handheld** (Nintendo DS–compatible) written
from scratch in Rust. The core is our own, not melonDS, DeSmuME, or a libretro core. The
frontend is a TUI for **Ghostty and kitty only**, drawn with the kitty graphics protocol. Its
edge over GUI emulators is TUI-native tooling: an integrated debugger and keyboard-first
control.

This file is the **single canon for every coding agent**. Codex · Claude Code · Pi ·
Cursor · OpenCode each enter through their own file and converge here (see
`.agents/README.md`). The project plan is `docs/PLAN.md` ("plan §N" below points into
it). The current phase and the open decisions are in `docs/STATUS.md`.

---

## 1. Commands

```bash
./scripts/verify.sh                                        # the whole gate, in order — CI runs the same script
./scripts/build_testroms.sh                                # build testroms/ into git-ignored testroms/build/ (needs Docker)
cargo fmt --all                                            # format
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo deny --locked check licenses bans sources            # dependency licenses (deny.toml)
cargo llvm-cov nextest --workspace --locked --fail-under-lines 90   # tests + the 90% coverage floor
python3 scripts/check_agent_config.py                      # agent-config integrity
python3 scripts/agent_hooks/selftest.py                    # hook self-test
python3 scripts/agent_hooks/guard_cleanroom.py --scan-tracked   # no dumps / GPL text tracked
python3 scripts/agent_hooks/check_architecture.py --all    # core purity · crate graph · names · SPDX
python3 scripts/agent_hooks/check_tests.py --all           # GWT names · no control flow · asserts
python3 scripts/agent_hooks/check_language.py --all        # English-only text · translations have an original
```

**Before you call a task done, `./scripts/verify.sh` must pass.** If it fails, the task is
not finished. Until Phase 0 lands the Cargo workspace, verify.sh runs the Python gates and
says plainly that it skipped the cargo gates — do not report those as passed.

Do not route around a failure with `#[allow(...)]`, `#[ignore]`, `--no-verify`, or by
editing a gate. When a lint exception is genuinely right, use
`#[expect(clippy::<lint>, reason = "<why>")]` so the reason is on the item and the exception
disappears when it stops being needed. A skipped test carries
`#[ignore = "<why>"]`.

The hook scripts are Python, standard library only, and must run on Python 3.9 (macOS's
`/usr/bin/python3`); `scripts/check_agent_config.py` needs 3.11+ (`tomllib`).

---

## 2. Architecture — never violate

The workspace follows plan §4.1. Library packages are prefixed `ninge-`: a package named
`core` would collide with Rust's built-in `core` crate, so the directory is `crates/core` and
the package is `ninge-core`. The frontend package is plain `ninge`, so that
`cargo install ninge` installs the `ninge` binary. Below, "the frontend" means that package.

```
crates/core/       ninge-core       the emulator. No IO. Pure logic.
crates/runner/     ninge-runner     headless runner: tests, traces, golden frames
crates/frontend/   ninge            the TUI app and the `ninge` binary (`cargo install ninge`)
crates/tracediff/  ninge-tracediff  trace comparison against a reference oracle
testroms/          our own bare-metal test ROM sources (BlocksDS / asm)
tests/             integration tests, golden-frame hashes
docs/hw/           hardware notes in our own words (the clean-room boundary)
```

Dependencies point **at ninge-core**, never away from it:

```
ninge (frontend) ┐
ninge-tracediff ─┼──▶ ninge-core          ninge-core depends on no workspace crate
ninge-runner ────┘                        nothing depends on the frontend (`ninge`)
                                          ninge-runner does not depend on ninge-tracediff
```

Inside ninge-core: `cpu/` (shared ARMv4T/v5TE interpreter) · `bus/` (ARM9 view, ARM7 view,
shared memory) · `scheduler/` · `gpu2d/` (engines A/B) · `gpu3d/` · `spu/` · `cart/`
(protocol, AUXSPI saves) · `spi/` (synthetic firmware, touch, power) · `hle/` (HLE BIOS SWIs,
direct boot) · `debug/` (breakpoint and trace hooks).

### Rules you must keep (plan §4.2, §8)

1. **ninge-core is pure.** Its inputs are button/touch state (and a ROM image as bytes); its
   outputs are two frame buffers and audio samples. No `std::fs` / `io` / `net` / `process` /
   `thread` / `env`, no `SystemTime` / `Instant`, no `print!` / `dbg!`, no randomness. IO
   belongs to ninge-runner and the frontend, which hand bytes in and take frames out.
   **Forbidden crates in `ninge-core`:** `anyhow` · `base64` · `chrono` · `cpal` · `crossbeam` · `crossterm` · `fastrand` · `flate2` · `getrandom` · `gilrs` · `libc` · `memmap2` · `mio` · `nix` · `parking_lot` · `rand` · `ratatui` · `rayon` · `time` · `tokio`.
   `rayon` is on that list although plan §3.2 names it for the Phase 7 rasterizer (decided,
   `docs/STATUS.md`): the rasterizer is a pure per-scanline function in ninge-core, driven
   through a `ScanlineExecutor` trait that ninge-core defines with a sequential
   implementation. The `rayon` executor lives outside core and is passed in as a generic
   parameter — static dispatch, no thread dependency in core. Each scanline reads only an
   immutable snapshot and writes only its own line, so the result cannot depend on
   scheduling, and a test pins parallel output = sequential output.

2. **Single ownership.** One top-level struct owns all emulator state and everything is
   reached through `&mut self`. Plan §4.2 names it `Nds`; plan §8 rule 10 keeps trademarks
   out of identifiers, so it is **`Machine`** (decided, `docs/STATUS.md`). No
   `Rc`, `Weak`, `RefCell`, `Cell`, `Arc`, `Mutex`, `static mut`, `thread_local!`,
   `LazyLock` / `OnceLock`.

3. **Static dispatch on the hot path.** The CPU is generic over a bus trait and
   monomorphized; ARM9 vs ARM7 is a const generic (`Cpu<const ARM9: bool>`). No `dyn` in
   ninge-core. No heap allocation, formatting, or `collect()` in CPU execution, memory
   access, or pixel processing.

4. **Decode tables are built at compile time** (`build.rs` or `const fn`), per variant.
   `build.rs` runs at build time and may write to `OUT_DIR`, so it sits outside the purity
   scan — but its output must be a pure function of the source.

5. **All state is plain data** and `serde`-serializable; savestates work from day one. No
   pointers, closures, boxed trait objects, or references with lifetimes in state.
   Collections are arrays, `Vec`, or `BTreeMap`: `HashMap` / `HashSet` seed their iteration
   order per process, so two runs would serialize the same state to different bytes.
   **Savestate format** (decided): a binary serde encoding behind a magic number and a format
   version. Before 1.0 there is no backward compatibility — a savestate whose version does
   not match is refused with a clear error, never half-loaded.

6. **Full determinism.** The same ROM + the same input log gives the same frames, audio, and
   savestate bytes (rewind, replay tests, and run-ahead all depend on it). Guest arithmetic
   is integer or fixed point, as on the hardware; a float in core state needs a reason in
   the code.

7. **Event scheduler.** Events sit in a binary heap keyed by timestamp; ARM9 clock = ARM7
   clock × 2. The goal is approximate timing plus determinism, not cycle accuracy. Order
   ties explicitly (timestamp, then a monotonically increasing sequence number) — never
   rely on the heap's internal order.

8. **The bus is two views over shared state** (ARM9, ARM7), because WRAMCNT and the VRAM
   banks move regions between owners.

9. **Memory fast path is a 16 KiB page table.** Entries are indices or offsets into buffers
   the `Machine` owns, never pointers (rule 5), so the table is plain data too.

10. **`unsafe` needs a measurement and a `// SAFETY:` comment.** The comment says why the
    code is sound; the PR cites the benchmark (`criterion`) that justified it. Workspace
    lints deny `unsafe_code`; a site that needs it opts in with
    `#[expect(unsafe_code, reason = "...")]`.

11. **Errors**: `thiserror` in ninge-core, `anyhow` in binaries. Guest-controlled input — a
    ROM, a register write, a DMA length — never panics the core. What the hardware does
    with an odd value (open bus, mirroring, masking) is the behavior; `unwrap()` /
    `expect()` in core only on invariants we own, with the invariant in the message.

12. **The frontend owns the terminal.** Threads (emu, render, input), triple-buffered frame
    hand-off, audio-based pacing, the kitty graphics/keyboard protocols, and cell-to-pixel
    mapping for touch all live in the frontend (plan §4.3). ninge-core never names a
    terminal type, and the frontend never makes an emulation decision. Frame transfer prefers
    kitty shared memory (`t=s`) where the terminal supports it and falls back to direct zlib
    transfer (`t=d`, `o=z`); the fallback is the default until Phase 0 confirms Ghostty's shm
    support.

**Platforms** (decided): macOS is first-class (Apple Silicon is the target machine); Linux is
guaranteed to pass CI; **Windows support is planned**, not yet provided. Nothing needs a
Windows code path today, but do not close the door: ninge-core is platform-independent by
construction, and the frontend keeps OS-specific pieces (shared memory, raw terminal setup,
paths) behind a small module of its own, so a Windows backend is an addition, not a rewrite.

`check_architecture.py` (edit hook and CI) enforces rules 1, 2, 3, 5, and 10 mechanically.
For a justified exception, put `// PURITY-OK: <reason>` on the line or the line above; the
reviewer must agree with the reason, and "it was easier" is not one.

---

## 3. Clean-room and legal invariants — the most dangerous part

A mistake here cannot be undone. Once a commercial ROM, a BIOS byte, or a translated GPL
function is in git history, every clone and fork keeps it; a force-push recalls nothing.
These rules come before every other consideration (plan §2, §8 rules 1, 2, 8, 10).

### 3.1 No dumps, no extracted data

- Never add a commercial ROM (`.nds` `.srl` `.dsi` `.ids` `.gba`), a save (`.sav` `.dsv`),
  data extracted from either, or a screenshot of a commercial game — not to the repo, a test
  fixture, an issue, a PR, or a commit message.
- Never describe where to obtain ROMs, and never link to such a source.
- Golden frames and fixtures come **only from our own test ROMs** in `testroms/`.
- **Built test ROMs are never committed — ours included.** Only their sources in `testroms/`
  are tracked; tests build the ROMs from source with BlocksDS, locally and in CI. The guard
  blocks `git` on any ROM file, and the clean-room scan fails on a tracked one.
- Running the emulator on a user's ROM, locally, is fine. Reading its bytes into the
  conversation (`xxd`, `strings`, the Read tool) is not — that is how extracted data ends
  up in a fixture.

### 3.2 Dump-free by default

- ninge boots on **HLE BIOS + direct boot + synthetic firmware**. A user-supplied BIOS or
  firmware dump is an optional path that no test requires.
- **Agents never open a BIOS or firmware dump**, not even to check an HLE routine. An HLE SWI
  written by someone who has read the BIOS disassembly is no longer clean-room. HLE
  behavior comes from GBATEK's description of what each SWI does.
- Synthetic firmware values (user name, language, touch calibration, …) are our own; the
  layout and the CRC come from GBATEK, computed in code, never copied from a dump.

### 3.3 GPL code is not a source

- GBATEK is the primary reference (plan §2, §9), then the ARM ARM and the ARM7TDMI /
  ARM946E-S TRMs.
- Plan §2 lets a **human** read GPL emulator code to understand how a problem was solved.
  **Agents do not open GPL emulator source at all** — not a local checkout, not the
  project's pages on a code host. An agent with a GPL function in its context while writing
  core will translate it line by line without meaning to; a prose summary is the only safe
  channel. Plan §9 already lists the melonDS blog as "concepts, no code".
  **GPL emulators agents do not open:** `melonDS` · `DeSmuME` · `NooDS`.
  For any other emulator, check its license first and treat an unknown license as GPL.
  Blogs, talks, and write-ups are fine.
- To learn how another emulator solved something: read its blog or notes, or ask a human,
  then write the **idea** (not the code, not the structure) into `docs/hw/`.
- Never write "ported from", "based on <emulator>", or a GPL notice into our code. The guard
  hook refuses a source file with a GPL notice.

### 3.4 Spec first, no guessing

- Every hardware behavior is implemented from `docs/hw/<feature>.md`, written **in our own
  words** from GBATEK (template in `docs/hw/README.md`).
- Uncertain behavior is not guessed: record it under "Unverified" in the note and leave
  `TODO(hw-unverified)` at the code site (plan §8 rule 8).

### 3.5 Trademarks

- No `Nintendo`, `DS`, `NDS`, `DSi`, `GBA`, `Pokémon`, … in a crate, binary, or repository
  name, or in an identifier. Hardware names in hardware context are exempt: register names
  (`DISPCNT`, `IPCSYNC`, `WRAMCNT`, `POWCNT1`), part names (ARM946E-S, ARM7TDMI), and format
  names in comments ("the NDS ROM header").
- The README may name the compatible hardware descriptively and minimally. Never style the
  leading N like a company mark.

### 3.6 No monetization

No sponsorship links, `FUNDING.yml`, or paid features (plan §2).

### 3.7 License: Apache-2.0

ninge is licensed under **Apache-2.0** (`LICENSE`; decided, `docs/STATUS.md`).

- Line 1 of every source file under `crates/`, `tests/`, and `testroms/` is a comment
  carrying `SPDX-License-Identifier: Apache-2.0` (`// …` in Rust and C, `@ …` in ARM
  assembly). `check_architecture.py` enforces it.
- Every package declares `license.workspace = true` against
  `[workspace.package] license = "Apache-2.0"`.
- Dependencies are permissive and Apache-2.0-compatible only: the allow-list is `deny.toml`,
  checked by `cargo deny` in `./scripts/verify.sh`. No GPL, LGPL, AGPL, or other copyleft
  crate. Widening the list is a `decision` issue.
- Do not paste third-party code without its license and attribution; for anything GPL, see
  §3.3 — it does not come in at all.

---

## 4. Rust conventions

- **Rust stable, edition 2024** (plan §3.1), **pinned to an exact version** in
  `rust-toolchain.toml` (`channel = "1.NN.0"`, at least 1.85 for edition 2024, with
  `rustfmt`, `clippy`, `llvm-tools-preview`). A floating `stable` would let a new rustfmt or
  clippy break CI overnight; bumping the pin is an `UPT:` PR of its own. `Cargo.lock` is
  committed (the workspace ships a binary). `./scripts/verify.sh` fails if the pin is missing
  or floating once `Cargo.toml` exists.
- `cargo fmt` and `cargo clippy -D warnings` are clean at all times.
- **Guest arithmetic wraps explicitly**: `wrapping_add`, `overflowing_sub`, `rotate_right`.
  Never rely on debug-build overflow panics for guest values, and write narrowing casts
  deliberately (`as u8` where truncation is the hardware behavior, `u32::from` where it is a
  widening).
- **Name the bits.** Register fields go through named constants or accessors, and the doc
  comment names the `docs/hw/` note. A bare `0x0400_0180` or `>> 13` in logic needs a comment.
- **Comments explain why.** Emulator code is full of deliberate oddities (open bus,
  mirroring, an off-by-one that the hardware really has). Cite the `docs/hw/` note so the next
  person does not "fix" it into a regression.
- Visibility is `pub(crate)` by default; `pub` items carry a doc comment.
- `#[inline]` / `#[inline(always)]` on the hot path only with a measurement.
- **Adding a dependency** asks first (`cargo add` is an `ask` permission). It must be
  permissively licensed (§3.7), and ninge-core's list in §2 rule 1 still applies.
- **Comments and docs are English** (§9).

---

## 5. Tests — enforced rules

### 5.1 Every core change comes with a test

Plan §7.3 and §8 rule 7. A `FIX:` comes with a test that failed before the fix. The kinds:

- **Unit** — `#[cfg(test)] mod tests` beside the code: instruction semantics, barrel
  shifter, flag rules, decoders, scheduler ordering, page-table mapping.
- **CPU JSON suites** — `SingleStepTests/ARM7TDMI` (MIT), pinned to commit `e3097d8`
  (plan §5 Phase 1). About 1 GB of `.json.bin` files, so a script fetches them into
  `tests/data/` (git-ignored) and verifies checksums; never vendored. The suite covers ARMv4T
  only — every ARMv5TE extension and ARM946E-S difference gets a self-written test.
- **Test ROMs** — our own bare-metal ROMs in `testroms/`, each with an expected result: a
  golden frame hash of both screens after N frames, or a memory-dump condition.
- **Integration** — `crates/*/tests/` and `tests/`: whole-machine runs through
  ninge-runner.

### 5.2 Names are Given–When–Then

Every test function is `given_<state>_when_<action>_then_<observable>`. The part after the
last `_then_` is something you can observe — `then_r1_includes_carry`, not `then_works`,
`then_ok`, or `then_passes`. `check_tests.py` enforces this on edit and in CI.

### 5.3 No control flow inside a test body

No `if` / `else` / `for` / `while` / `loop` / `match` (so no `let … else`) and no `for_each`
in a test body. A table of cases goes through a harness function that returns the list of
failures, and the test asserts that list is empty — every failing case shows at once,
instead of stopping at the first.

```rust
// Bad — stops at the first failing case and hides the others
#[test]
fn given_adc_suite_when_run_then_all_match() {
    for case in load_cases("arm/adc.json") {
        assert_eq!(run_case(&case), case.expected);
    }
}

// Good — the loop lives in the harness; the test asserts the outcome once
#[test]
fn given_adc_json_suite_when_each_case_runs_then_no_case_differs() {
    let failures = run_json_suite::<false>("arm/adc.json");
    assert_eq!(failures, Vec::<CaseDiff>::new());
}
```

### 5.4 Every test asserts

An `assert*!`, a `prop_assert*!`, an insta snapshot, or a helper named `assert_*` — or
`#[should_panic(expected = "...")]`. A test that only runs code raises coverage and proves
nothing.

### 5.5 Determinism is tested, not assumed

- Running the same ROM with the same input log twice gives identical frame hashes and
  identical savestate bytes.
- A savestate round-trip (save → load → continue) matches an uninterrupted run.
- Add these properties as features land; `proptest` for pure functions (shifter, flags,
  fixed-point math). Commit `proptest-regressions/`.

### 5.6 Golden frames and snapshots are evidence

- A golden hash covers both screens. **Changing one requires the reason in the PR**
  (plan §7.3): which hardware behavior changed, and why the new frame is the right one.
- Never bulk-accept snapshots (`cargo insta accept`) or regenerate hashes to make a red test
  green. A golden mismatch is a regression until shown otherwise.

### 5.7 Nothing commercial, nothing networked

Tests use only our own test ROMs and synthetic data. No network. A local-only test that
needs a user's ROM reads its path from `NINGE_USER_ROM`, is
`#[ignore = "needs a user-supplied ROM"]`, and never runs in CI.

### 5.8 Boundary behavior

Emulator bugs live at the edges. Cover: PC-relative reads (pipeline offset +8 / +4), writes
to r15 and mode switches, banked registers on exception entry, ARM/Thumb interworking
(including the v5-only `LDR PC` / `POP {PC}` switch), unaligned accesses per CPU, open bus
and mirrors, VRAM bank remaps and WRAMCNT changes, an IRQ raised while IME=0, FIFO
full/empty error flags, timer overflow cascade, DMA start modes, and scheduler ties.

### 5.9 Coverage floor is 90%

**Coverage floor:** `90%` line coverage across the workspace, measured by `cargo-llvm-cov` in
`./scripts/verify.sh` and CI (`--fail-under-lines 90`). Falling below fails the gate.

- Raise coverage by writing tests that assert hardware behavior — never by lowering the
  floor, excluding files (`--ignore-filename-regex`, `#[coverage(off)]`), or writing tests
  that only execute code (§5.4).
- The frontend (`ninge`) is included. Keep terminal IO behind thin adapters so the logic around them
  (layout, input mapping, pacing, the kitty encoder) is testable without a terminal.
- Coverage is necessary, not sufficient: a covered line whose behavior no test checks is
  still untested.

---

## 6. Hardware work and phase discipline

- **Implementing a hardware feature** follows plan §7.1 — spec note → test first →
  implement in ninge-core → expose to the debugger → verify → record any design
  decision in a GitHub issue. The `add-hw-feature`
  skill is the procedure.
- **Investigating a bug** follows plan §7.2 — pin it with an input replay → find the first
  divergence with a trace diff against a reference emulator used as a black-box oracle →
  check the `docs/hw/` note (write it first if missing) → add the failing test → fix. The
  `trace-bug` skill is the procedure. Running a reference emulator's *binary* as an oracle
  is fine; reading its source is not (§3.3).
- **The oracle** (decided) is a melonDS release binary driven over the GDB remote protocol —
  a standard protocol, so no source is read; NooDS's binary is the fallback if melonDS's stub
  cannot reach both CPUs. Each developer installs it; it is never vendored or redistributed.
  Traces are line-oriented text, one instruction per line (CPU, step, PC, opcode, r0–r15,
  CPSR), written by ninge-runner. ARM9/ARM7 interleaving differs between emulators by design,
  so streams are compared **per CPU**, narrowing from frame hashes to event boundaries to
  instructions.
- **Stay in the current phase** (`docs/STATUS.md`). A later phase's feature is a stub plus
  `TODO(phaseN)` (plan §8 rule 9). Each phase's DoD must be met before the next starts.
- **ARM9 first.** Phase 2 brings up ARM9 alone with ARM7 stubbed; ARM7 joins in Phase 3.
- **Debugger exposure** (plan §7.1 step 4): new state is visible in a debugger panel, or
  the PR says which panel will show it and when.
- **Design decisions are GitHub issues**, not files in the repo. A decision that is expensive
  to reverse or that someone will later want to "tidy" (the crate graph, the savestate
  format, the scheduler model, what may use `unsafe`) gets an issue labeled `decision` from
  `.github/ISSUE_TEMPLATE/decision.md`: context, decision, consequences, alternatives. The PR
  that acts on it links the issue, and code that exists because of it cites it
  (`// Decided in #42.`). An agent may draft such an issue only when asked (`gh issue create`
  is an `ask` permission); deciding and closing it is a human's call.
- Markers: `TODO(hw-unverified)` (spec unsure, §3.4), `TODO(phaseN)` (deferred to phase N),
  `PURITY-OK:` (§2 exception), `SAFETY:` (§2 rule 10).

---

## 7. CI/CD and the agent gates

- **Branch model** — working PRs target `develop`; only release PRs (`develop` → `main`)
  target `main`. Both branches are protected by the rulesets in `.github/rulesets/`
  (applied with `apply.sh`): no deletion or force push, green CI required with **no bypass**,
  and a PR with one approval — which the admin (`devgyurak`) may bypass when merging a PR.
  Direct pushes are blocked for everyone.
- **Repository** — `github.com/devgyurak/ninge`.
- **Releases** (decided) — SemVer `0.x`; a `vX.Y.Z` tag on `main` is the release, and the
  release PR bumps `[workspace.package] version`. The first release, `0.1.0`, follows the
  Phase 3 DoD. Each release publishes macOS arm64 and Linux x86_64 binaries to GitHub
  Releases and all crates to crates.io in lockstep (`cargo install ninge`); a Homebrew tap
  comes later. The release workflow is added with the first release, not before.
- **CI** (`.github/workflows/ci.yml`) runs `./scripts/verify.sh` on Linux and macOS. The
  Rust toolchain, `cargo-nextest`, and `cargo-llvm-cov` steps switch on the moment
  `Cargo.toml` exists.
- **BlocksDS is required** (decided). Built test ROMs are never committed (§3.1), so the
  golden tests — and therefore the 90% coverage floor — need the SDK. It comes from the
  official BlocksDS Docker image **pinned by digest**, the same image locally and in CI.
  The digest lives only in `scripts/build_testroms.sh`. Bumping it is an `UPT:` PR, and
  every golden hash it changes needs its reason. macOS runners cannot run Docker, so CI
  builds the ROMs once on ubuntu (`testroms` job) and the macOS verify job checks that
  artifact against its `SHA256SUMS` (`NINGE_PREBUILT_TESTROMS=1`, set in `ci.yml` only).
  Without Docker and without that variable `verify.sh` fails — it never skips the step.
- **The coverage floor is not CI-only and is never disabled.** `check_agent_config.py` fails
  if the floor in §5.9 and the one in `scripts/verify.sh` differ.
- **Agent config** (`.github/workflows/agents.yml`) runs `scripts/check_agent_config.py` and
  the hook self-test, so a broken pointer or hook fails in the PR, not on a contributor's
  machine.
- **Local hooks are best-effort; CI is the real gate.** A file changed by `Bash`/`sed`, or by
  a harness whose hook is not wired, skips the edit hooks. Every rule that matters has a
  whole-repo mode (`--all`, `--scan-tracked`) that verify.sh and CI run.
- **One implementation of each gate.** Decision logic lives only in `scripts/agent_hooks/`.
  The CI modes and the tool adapters call it; nothing copies it. `check_agent_config.py`
  imports the hooks' lists to prove §2 and §3.3 state the same ones.
- **One permission policy across harnesses.** Allow read-only verification
  (`cargo fmt|clippy|check|test|nextest|llvm-cov|tree|doc`, `python3 scripts/…`, `./scripts/verify.sh`,
  `git status|diff|log|show`) and `git add|commit|push` — an agent still commits or pushes
  only when the user asks (§8), so a prompt adds nothing; **ask** for dependency and
  outward-facing commands (`cargo add`, `cargo install`, `cargo update`, `gh issue create`,
  `gh pr create`); **deny** destructive ones (`rm -rf`, force-push). Wired in `.claude/settings.json` and `.opencode/opencode.json`; Pi
  and Codex follow their own settings, and the shared guard still runs.

---

## 8. Workflow

1. **Read before you change.** Emulator code has reason-shaped oddities; the `docs/hw/`
   note says why.
2. **Do only the requested scope** and only the current phase. Do not "tidy" unrelated code.
3. If you touched ninge-core, **prove it with a test** and say which one.
4. `./scripts/verify.sh`, all green.
5. Report factually what changed and what you verified. If something failed or was skipped,
   say so.
6. **Commit and push only when the user asks.** The permissions do not prompt for
   `git commit` / `git push`; the user's request is the approval.

### Commit · PR · branch names

The commit subject, PR title, and branch share **one prefix**.

```
<PREFIX>: <summary>
```

`ADD:` new — feature · hardware behavior · test · workflow · dependency ·
`FIX:` bug fix · `REF:` refactor (behavior unchanged) · `DEL:` removal ·
`DOCS:` documentation only · `UPT:` dependency version bump

`ADD:`/`DEL:` refer to **code and dependencies**. Documentation, `docs/hw/` notes, and
agent settings (`.claude/` · `.agents/` · `.codex/` · `.cursor/` · `.opencode/` · `.pi/`) are
always `DOCS:`. `scripts/` and `.github/workflows/` are code — they run.

**One PR = one hardware feature or one bug** (plan §7.3). Branches are
`<prefix lowercase>/<summary-kebab>` (`add/ipc-fifo`, `fix/ldm-writeback-base-in-list`),
cut from `develop`. Full guidance and the pre-commit checklist live in the `commit` skill
(`.claude/skills/commit/SKILL.md`). Read it before committing.

---

## 9. Documentation and language — English only

- **Everything written in this repository is English**: code, comments, doc comments,
  identifiers, user-facing strings, documentation, commit messages, PR and issue text, and
  test names. This holds even when the conversation with the user is in another language.
- **The one exception is an approved translation.** A document may have a translated copy
  next to it, named with a `-{language-code}` suffix before the extension
  (`README-KO.md`, `docs/PLAN-KO.md`). The unsuffixed English file is canonical, a
  translation must have one, and a change to the English file updates or flags its
  translations in the same PR.
- **Approval is a human's.** Do not create a translation, or add a language, unless the user
  asked for that document in that language. Updating an existing approved translation to
  match its English original is in scope.
- Never translate identifiers or code, and keep `AGENTS.md`, the skills, and the subagents
  English — they are the canon every agent reads.
- `check_language.py` (edit hook and CI) fails on non-Latin script outside a `-{code}` file
  and on a translation with no English original. A non-Latin character that is genuinely
  needed (a test of text rendering, say) is written as an escape (`'\u{AC00}'`), so the
  source stays English.

---

## 10. Do not

- Add, read, or describe how to get a ROM, BIOS, firmware dump, or anything extracted from one.
- Open GPL emulator source, or copy or translate any emulator's code.
- Guess hardware behavior — write it down as Unverified instead.
- Put IO, time, randomness, threads, `Rc`/`RefCell`, or `dyn` in ninge-core.
- Change ninge-core without a test, or update a golden hash without the reason.
- Commit a built ROM, even one of our own test ROMs.
- Lower a gate (the 90% coverage floor included), add `#[allow]` without a reason, or
  `#[ignore]` a failing test to go green.
- Build a later phase's feature now.
- Report "fixed" or "passing" without having run the gate.
