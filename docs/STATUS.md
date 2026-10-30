# Status

The current phase is the single line that starts with `▶`. The SessionStart hook prints it
and `scripts/check_agent_config.py` fails unless there is exactly one. Move the marker only
when the phase's DoD (plan §5) is met — that is a human decision.

▶ Phase 0 — harness and tools (not started; the agent workflow is in place)

| Phase | Scope (plan §5) | DoD | State |
|---|---|---|---|
| 0 | Workspace, CI, bare-metal test-ROM toolchain, golden-frame infra, TUI skeleton | A dummy frame buffer shows in Ghostty and kitty; one test ROM builds and `ninge-runner` loads it | Not started |
| 1 | CPU: ARMv4T (ARM + Thumb), ARMv5TE extensions, minimal CP15, disassembler | ARM7TDMI JSON suite passes; a self-written test passes for each v5 extension | — |
| 2 | First pixel, ARM9 only: header parse, direct boot, main RAM, TCM, VRAM A LCDC, display mode 2 | A test ROM's VRAM image shows in Ghostty, pinned by a golden-frame test | — |
| 3 | Both CPUs: scheduler, two bus views, WRAMCNT, IPC, IRQ, timers, DMA, HLE SWI, text BG | A libnds homebrew's console printf output displays correctly | — |
| 4 | 2D complete: engines A/B, sprites, ext palettes, windows, blending, brightness, VRAM page table, capture | Every 2D test ROM's golden frame passes | — |
| 5 | Commercial boot: cart protocol, AUXSPI saves, synthetic firmware, RTC, touch SPI, power, sound-register stubs | Pokémon Diamond boots, starts a new game, saves, and reloads the save (3D may be a non-drawing stub) | — |
| 6 | Sound: 16 channels, PCM/ADPCM/PSG, capture, `cpal`, audio pacing | Waveform snapshot tests pass; a commercial game's sound is correct | — |
| 7 | 3D: geometry engine, software rasterizer | 3D test ROM golden frames pass; Pokémon Diamond is playable | — |
| 8 | Performance and accuracy | Full speed on Apple Silicon for the main games | — |

Parallel track from Phase 1: debugger panels, trace diff (a reference emulator as the
oracle), replay-based regression tests.

## Decided

Settled by the maintainer on 2026-10-29. Each one is already reflected in AGENTS.md and the
gates; reopening one is a new `decision` issue.

- [x] **Package names** — library crates are `ninge-*` (`ninge-core`, `ninge-runner`,
      `ninge-tracediff`) and the frontend package is plain `ninge`, so `cargo install ninge`
      installs the binary; a package named `core` would collide with Rust's built-in crate.
      Enforced by `check_architecture.py`.
- [x] **Top-level state struct** — `Machine`, not the plan's `Nds` (plan §8 rule 10).
- [x] **`rayon` stays out of ninge-core.** The Phase 7 rasterizer is a pure per-scanline
      function in ninge-core, driven through a `ScanlineExecutor` trait that ninge-core defines
      with a sequential implementation. The `rayon` implementation lives outside core (in
      the frontend, or a dedicated crate added in Phase 7) and is passed in as a generic
      parameter — static dispatch, no thread dependency in core. Each scanline reads only an
      immutable snapshot and writes only its own line, so the output does not depend on
      scheduling; a test pins parallel output = sequential output. (AGENTS.md §2 rule 1.)
- [x] **Agent GPL-source rule** — agents open no GPL emulator source at all; humans may read
      it for ideas (plan §2). (AGENTS.md §3.3.)
- [x] **Built test ROMs are never committed.** Only their sources in `testroms/` are tracked.
      Golden tests build the ROMs from source (BlocksDS); CI installs the SDK. The clean-room
      scan fails on any tracked ROM, ours included, and the guard blocks `git` on one.
      (AGENTS.md §3.1.)
- [x] **License: Apache-2.0** (single license). `LICENSE` holds the text; every source
      carries `SPDX-License-Identifier: Apache-2.0` on line 1; dependencies are limited to
      the permissive allow-list in `deny.toml`. (AGENTS.md §3.7.)
- [x] **Coverage floor: 90% line coverage**, workspace-wide, through `cargo-llvm-cov` in
      `./scripts/verify.sh` and CI. (AGENTS.md §5.9.)
- [x] **Repository** — `github.com/devgyurak/ninge`.
- [x] **Branch model** — PRs to `develop`, releases `develop` → `main`. After the first
      commit on `main`, create `develop` and protect both (PR + green CI). (AGENTS.md §7.)
- [x] **Toolchain** — an exact stable version in `rust-toolchain.toml` (≥ 1.85), bumped by
      its own `UPT:` PR; `verify.sh` refuses a floating channel. (AGENTS.md §4.)
- [x] **BlocksDS is required** — the official Docker image pinned by digest, the same locally
      and in CI. macOS CI cannot run Docker: the ubuntu `testroms` job builds the ROMs, the
      ubuntu verify job rebuilds and must match them, and the macOS job verifies the
      artifact's checksums. (AGENTS.md §7, decision #65.)
- [x] **Platforms** — macOS first-class, Linux CI-verified, **Windows planned**: no Windows
      code path yet, OS-specific frontend pieces stay behind one module. (AGENTS.md §2.)
- [x] **Savestate format** — binary serde encoding with a magic number and a format version;
      no backward compatibility before 1.0; a mismatch is refused with a clear error.
      (AGENTS.md §2 rule 5.)
- [x] **First commercial target: Pokémon Diamond.** Its overworld is 3D, so it cannot meet
      the plan's original "a 2D game plays from boot to save": Phase 5's DoD is now boot →
      new game → save → reload after restart, with the 3D engine allowed to be a stub that
      accepts geometry commands without drawing; Phase 7's DoD is Diamond fully playable.
      It is a **local, manual** check on the maintainer's own cartridge dump — never a CI
      test, a fixture, a golden hash, or a name in code (AGENTS.md §3.1, §3.5, §5.7). Its use
      of RTC, touch, and FLASH saves makes it a good Phase 5 exercise.
- [x] **Frame transfer** — kitty shared memory (`t=s`) where supported, direct zlib transfer
      (`t=d`, `o=z`) as the fallback and the default until Phase 0 confirms Ghostty's shm
      support. (AGENTS.md §2 rule 12.)

## Phase 0 checks

Facts to confirm while building Phase 0, not decisions:

- [ ] Ghostty's kitty-graphics shared-memory (`t=s`) support (plan §3.3).
- [x] **BlocksDS image** — `skylyrac/blocksds` tag `slim-v1.24.0`, multi-arch index digest
      `sha256:779f6193d1030c62e60d1f0f39c3e32cf4045c0bd224a45c0ceadf9d9ab01a51`. Code reads the
      digest only from `scripts/build_testroms.sh`; this line records it and
      `check_agent_config.py` keeps the two equal. It builds the bare-metal `smoke` ROM; the
      build is byte-identical across clean rebuilds and across arm64 and amd64.
- [x] The current stable Rust version to pin — `1.99.1` (released 2026-10-13).
- [x] **ARM7TDMI JSON suite** — `SingleStepTests/ARM7TDMI` (MIT, Apache-2.0-compatible),
      pinned to commit `e3097d88d428752736b949d94c094d5da50f0db6`. About 1 GB of `.json.bin`
      files across 46 instruction groups: fetched into `tests/data/` (git-ignored) with
      checksums, never vendored. ARMv4T only — v5TE gets self-written tests. The fetch script
      and the `.json.bin` reader are Phase 1 work. (AGENTS.md §5.1.)
- [x] **Trace-diff oracle** — a melonDS release binary over the GDB remote protocol (NooDS's
      binary as the fallback), installed by each developer, never redistributed. Line-oriented
      text traces, compared per CPU. Whether the stub reaches both CPUs is checked when the
      track starts. (AGENTS.md §6.)
- [x] **Releases** — SemVer `0.x`, tag-driven; `0.1.0` after the Phase 3 DoD; GitHub Releases
      binaries (macOS arm64, Linux x86_64) and crates.io in lockstep; Homebrew tap later.
      (AGENTS.md §7.)

## Open decisions

None. A new one is decided by a human and recorded in a GitHub issue labeled `decision`
(`.github/ISSUE_TEMPLATE/decision.md`); the PR that acts on it links the issue.
