<!-- README-KO.md is its approved Korean translation; change both together. -->

<div align="center">

<h1 align="center">ninge</h1>

**A terminal-native dual-screen handheld emulator, written from scratch in Rust**

[![CI](https://github.com/devgyurak/ninge/actions/workflows/ci.yml/badge.svg?branch=develop)](https://github.com/devgyurak/ninge/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Rust 1.99.1](https://img.shields.io/badge/rust-1.99.1-orange?logo=rust)](rust-toolchain.toml)
[![Terminals: Ghostty · kitty](https://img.shields.io/badge/terminals-Ghostty%20%C2%B7%20kitty-informational)](#run)
[![Last commit](https://img.shields.io/github/last-commit/devgyurak/ninge/develop)](https://github.com/devgyurak/ninge/commits/develop)

[Status](#status) · [Install](#install) · [Run](#run) · [Documentation](#documentation) ·
[Contributing](#contributing) · [Korean](README-KO.md)

</div>

**ninge** (said "ninji"; a twist on *hinge*, the joint between two screens) emulates a
Nintendo DS–compatible handheld inside the terminal. Both screens are drawn with the kitty
graphics protocol, so it runs in **Ghostty and kitty** only. The core is our own, with no
code from another emulator, and it boots without any BIOS or firmware dump.

## Status

ninge is early. Work follows the phases in [`docs/PLAN.md`](docs/PLAN.md); the current one is
in [`docs/STATUS.md`](docs/STATUS.md).

| Works today | Not yet |
|---|---|
| Both CPUs (ARMv5TE and ARMv4T) on a deterministic event scheduler | The bottom screen (2D engine B) |
| Shared work RAM, video RAM bank A, IPC, interrupts, timers, DMA | Input: the keypad always reads "nothing pressed" |
| HLE BIOS calls and direct boot: no dump needed | Touch screen, cartridge reads and its file system |
| Text backgrounds on the top screen | Sprites, 3D and sound |
| A homebrew program's console output on the top screen | The real-time clock (reads 0) |

The limits of each release are in [`docs/release-notes/`](docs/release-notes/).

## Install

ninge needs Rust (the version pinned in [`rust-toolchain.toml`](rust-toolchain.toml) is the
one CI uses). From crates.io, from version 0.1.0 on:

```sh
cargo install ninge
```

From a clone of this repository:

```sh
cargo install --locked --path crates/frontend
```

macOS (Apple Silicon first) and Linux are supported; Windows is planned.

## Run

Open Ghostty or kitty, then:

```sh
ninge path/to/program.nds   # run a ROM
ninge                       # no ROM: show a test pattern
```

Quit with `q`, `Esc` or `Ctrl-C`. ninge runs programs you have the right to run, such as
homebrew you built yourself; it ships no ROMs and never needs a BIOS or firmware file.

## Build and test

```sh
./scripts/verify.sh            # the whole gate: config checks, fmt, clippy, tests, 90% coverage
./scripts/build_testroms.sh    # build our own test ROMs into testroms/build/ (needs Docker)
```

The test ROMs are bare-metal programs in [`testroms/`](testroms/), built with the
[BlocksDS](https://blocksds.skylyrac.net/) SDK from a Docker image pinned by digest, so
`verify.sh` needs [Docker](https://www.docker.com/). `ninge-runner` runs a ROM headlessly
and prints a hash of each screen:

```sh
cargo run -p ninge-runner -- testroms/build/smoke.nds --frames 3
# frames=3 top=<hash> bottom=<hash>
```

## Branches and releases

`develop` and `main` only change through pull requests, and CI must be green:

```
<prefix>/<summary>  ──PR──▶  develop  ──release PR──▶  main  ──tag vX.Y.Z──▶  crates.io
```

A branch off `develop` is merged by a PR whose title starts with a prefix (`ADD:`, `FIX:`,
`REF:`, `DEL:`, `DOCS:`, `UPT:`). A release is a PR from `develop` into `main` that bumps the
version, followed by a `vX.Y.Z` tag on `main`. The tag runs
[`release.yml`](.github/workflows/release.yml), which publishes `ninge-core` and `ninge` to
crates.io after the maintainer approves it, and creates the GitHub Release with the notes
from [`docs/release-notes/`](docs/release-notes/). `ninge-runner` and `ninge-tracediff` are
repository tools and are not published.

## Clean room

- No commercial ROM, save, BIOS or firmware dump, or anything extracted from one, is in this
  repository — not even our own built test ROMs. Tests use only programs built from
  [`testroms/`](testroms/).
- Hardware behavior is implemented from notes written in our own words in
  [`docs/hw/`](docs/hw/), mainly from GBATEK. Uncertain behavior is marked unverified, not
  guessed.
- No code from another emulator is copied or translated. A reference emulator's release
  binary is used only as a black-box oracle.

The full rules are in [`AGENTS.md`](AGENTS.md) §3.

## Repository layout

```
crates/core/       ninge-core       the emulator: pure logic, no IO
crates/frontend/   ninge            the TUI and the `ninge` binary
crates/runner/     ninge-runner     headless runner: tests, traces, golden frames
crates/tracediff/  ninge-tracediff  trace comparison against a reference emulator
testroms/          our own bare-metal test ROM sources
tests/             integration tests, golden-frame hashes
docs/hw/           hardware notes in our own words
docs/              the plan, the current status, release notes
scripts/           verify.sh, test-ROM build, agent hooks
AGENTS.md          project conventions for every coding agent and contributor
```

## Documentation

| Document | What is in it |
|---|---|
| [`docs/PLAN.md`](docs/PLAN.md) ([Korean](docs/PLAN-KO.md)) | Goals, architecture, the phased roadmap ([§5](docs/PLAN.md#5-roadmap)), references ([§9](docs/PLAN.md#9-references)) |
| [`docs/STATUS.md`](docs/STATUS.md) | The current phase, each phase's done-when, settled decisions |
| [`docs/release-notes/`](docs/release-notes/) | What each release can and cannot do |
| [`docs/hw/`](docs/hw/README.md) | Hardware notes in our own words: the clean-room boundary |
| [`AGENTS.md`](AGENTS.md) | Architecture rules ([§2](AGENTS.md#2-architecture--never-violate)), clean room ([§3](AGENTS.md#3-clean-room-and-legal-invariants--the-most-dangerous-part)), tests ([§5](AGENTS.md#5-tests--enforced-rules)) |
| [`testroms/README.md`](testroms/README.md) | Each test ROM and what it checks |
| [`tests/golden/README.md`](tests/golden/README.md) | How golden-frame hashes are made and when they may change |
| [Issues labeled `decision`](https://github.com/devgyurak/ninge/issues?q=label%3Adecision) | Design decisions with their context and alternatives |

## Contributing

Read [`AGENTS.md`](AGENTS.md) first: it holds the architecture rules, the clean-room rules
and the test conventions, and `./scripts/verify.sh` must pass before a pull request. Pull
requests go to `develop`.

## License

[Apache License 2.0](LICENSE).
