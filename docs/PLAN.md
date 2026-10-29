<!-- The project plan. AGENTS.md cites it as "plan §N". docs/PLAN-KO.md is its approved Korean translation; change both together. Changing the plan is a human decision. -->

# ninge — a terminal-native NDS emulator

> The project name `ninge` (pronounced "ninji") is a twist on *hinge*, taken from the
> structure of a folding device whose two screens are joined by a hinge. The crate names,
> the binary name, and the repository name all use `ninge`. The README and logo do not
> emphasize the first letter N in the manner of a particular company's trademark.

## 1. Project overview

### 1.1 Goals
- Implement a **Nintendo DS emulator core in Rust, from scratch**. No external emulator core
  (melonDS, DeSmuME, libretro cores, …) is used.
- The frontend is a **terminal UI (TUI)**, and the supported terminals are **limited to
  Ghostty and kitty**.
- Platforms: macOS first-class (Apple Silicon), Linux verified in CI. **Windows support is
  planned.**
- The differentiator is **TUI-native features that GUI emulators do not offer (an integrated
  debugger, keyboard-first control)**.

### 1.2 Non-goals
- GBA emulation and the DS's GBA compatibility mode
- DSi extensions
- Wi-Fi / local wireless
- Terminals without the kitty graphics protocol
- A JIT in the early stages (start with an interpreter)

### 1.3 Strategy in brief
- **Go straight to NDS**, without a GBA emulator first.
- The two main risks this creates, and the answers:
  1. The CPU must be verified without a system around it → per-instruction JSON tests +
     self-written bare-metal test ROMs
  2. Two CPUs must be debugged from the start → **bring up ARM9 alone**, then add ARM7

---

## 2. Legal and licensing principles (apply before everything else)

| Rule | Content |
|---|---|
| No ROMs | Do not put commercial ROMs, extracted data, or screenshots in the repository, issues, or test fixtures. Do not explain how to obtain ROMs or link to sources. |
| No BIOS/firmware | Do not include Nintendo BIOS or firmware dumps. The default is to run without dumps on **HLE BIOS + direct boot + synthetic firmware**. A user's own dump is an optional path. |
| Clean room | GBATEK is the primary documentation. GPL code such as melonDS is read **only to understand how a problem was solved**; code is not copied or translated line by line. |
| Trademarks | Do not put Nintendo, DS, Pokémon, or other trademarks in the project, crate, or binary names. The README uses them only descriptively and minimally (to describe compatibility). |
| No monetization | No donations, Sponsors, or paid features. |
| License | **`Apache-2.0`** (decided; on the premise that no GPL code is used). |

---

## 3. Technology stack

### 3.1 Language and toolchain
- Rust stable, edition 2024 (or the latest stable edition)
- cargo workspace
- Test ROM builds: **BlocksDS** (a DS homebrew SDK), the `arm-none-eabi` assembler

### 3.2 Main crates (candidates)

| Area | crate | Use |
|---|---|---|
| TUI layout | `ratatui` | panels, layout, widgets |
| Terminal IO | `crossterm` | raw mode, kitty keyboard protocol (`KeyboardEnhancementFlags`), mouse |
| Gamepad | `gilrs` | input straight from the OS, bypassing the terminal |
| Audio | `cpal` | audio output, ring buffer |
| Serialization | `serde` + a binary format (bincode, …) | savestates |
| Parallelism | `rayon` | scanline-parallel 3D rasterizer |
| Compression / encoding | `flate2`, `base64` | kitty graphics `o=z` transfer |
| Errors | `thiserror` (core), `anyhow` (bin) | |
| Tests | `insta` (snapshots), `proptest` (property-based), `criterion` (benchmarks) | |

- The kitty graphics protocol is **implemented directly**. `ratatui-image` is a reference
  only; the 60 fps path needs a dedicated implementation.
- The `core` crate does not depend on the IO-related crates above.

### 3.3 Terminal features used (Ghostty / kitty)

| Feature | How it is used | Notes |
|---|---|---|
| Kitty graphics protocol | Replace frame data under the same image id; place it on the cell grid with Unicode placeholders | Local: shared memory (`t=s`); otherwise: direct zlib transfer (`t=d`, `o=z`). **Ghostty's shm support must be confirmed** |
| Kitty keyboard protocol | Receive press / repeat / release events | A precondition for game input |
| Mouse | SGR 1006 + SGR-Pixels (1016) | Maps to touch coordinates on the bottom screen. Support is detected at runtime |

---

## 4. Architecture

### 4.1 Workspace layout

```
ninge/
├── crates/
│   ├── core/        # The emulator core. No IO. Pure logic.
│   │   ├── cpu/         # Shared ARMv4T/v5TE interpreter (const generic)
│   │   ├── bus/         # ARM9 bus view, ARM7 bus view, shared memory
│   │   ├── scheduler/   # Event-driven scheduler
│   │   ├── gpu2d/       # Engines A/B
│   │   ├── gpu3d/       # Geometry engine, rasterizer
│   │   ├── spu/         # Sound
│   │   ├── cart/        # Cartridge protocol, AUXSPI saves
│   │   ├── spi/         # Firmware (synthetic), touch controller, power
│   │   ├── hle/         # HLE BIOS (SWI), direct boot
│   │   └── debug/       # Debugger hooks (breakpoints, traces)
│   ├── runner/      # Headless runner. Tests, traces, golden frames
│   ├── frontend/    # TUI app: renderer, input, audio, layout, debugger panels
│   └── tracediff/   # Trace comparison tool
├── testroms/        # Self-written test ROM sources (BlocksDS / asm)
├── tests/           # Integration tests, golden frame hashes
└── docs/            # Hardware notes (spec summaries we write ourselves)
```

### 4.2 Core design principles
1. **A pure core with no IO.** Input: button/touch state. Output: two frame buffers, audio
   samples.
2. **Single ownership.** A top-level `Nds` struct owns all state. No `Rc<RefCell<_>>`.
   Access goes through `&mut self`.
3. **The CPU is generic over a bus trait** and monomorphized. Dynamic dispatch is forbidden
   on the hot path.
4. **One implementation for ARMv4T/v5TE**, in the form `Cpu<const ARM9: bool>`. The
   per-variant decode tables are generated at compile time with `build.rs` or `const fn`.
5. **All state is plain data.** It must be serde-serializable, and savestates are supported
   from the start. No pointers or closures in state.
6. **Full determinism.** The same ROM + the same input log = the same result. No system
   time, randomness, or thread timing enters the core. (The precondition for rewind, replay
   tests, and run-ahead.)
7. **Event-driven scheduler.** A binary heap keyed by timestamp. ARM9 clock = ARM7 clock × 2.
   Cycle accuracy is not the goal; the goal is approximate timing + determinism.
8. **The bus is two views over shared state.** ARM9 and ARM7 views are separate, because
   regions such as WRAMCNT and the VRAM banks change owners.
9. **Memory fast path.** VRAM bank mapping and similar are handled with a page table in
   16 KB units.
10. `unsafe` is used only where a measurement proves a performance need, with a comment
    stating why it is safe.

### 4.3 Frontend design
- **Threads:** emu (core execution + audio pacing), render (consumes the latest frame),
  input. Frames are handed over through a triple buffer. If rendering is slow, frames are
  dropped.
- **Pacing:** audio-driven. When there is no audio, switch to frame-driven pacing.
- **Rendering:** separate emulation (about 60 fps) from terminal output fps, measure the
  terminal write time, and adapt the output fps.
- **Screen layout modes:** vertical (original arrangement) / horizontal / focus (one screen
  large, the other small) / bottom only. Swap and switch with shortcuts.
- **Input:**
  - Keyboard: kitty keyboard protocol press/release
  - Gamepad: `gilrs`
  - Touch: SGR-Pixels mouse → bottom-screen coordinates. Without support, map cell centers
  - Keyboard virtual cursor: hjkl to move, space to tap, a drag-mode toggle
  - Microphone (blowing): mapped to one key
- **Debugger panels (the TUI's key differentiator):**
  - ARM9/ARM7 separately: registers, disassembly, breakpoints, watchpoints, stepping
  - Memory viewer and search
  - VRAM bank mapping status, tile, palette, and OAM views
  - IPC FIFO contents
  - Scheduler event timeline
  - (From Phase 7) 3D command log, polygon wireframe

---

## 5. Roadmap

Each phase must meet all of its **definition of done (DoD)** before the next begins.

### Phase 0. Harness and tools
- Workspace skeleton, CI (build, test, clippy, fmt)
- Test ROM toolchain: must be able to build **bare-metal test ROMs that run without libnds
  initialization**. (libnds initialization waits for ARM7 and IPC synchronization, so it
  cannot be used early on.)
- Golden-frame test infrastructure: run a ROM for N frames → compare frame buffer hashes
- TUI skeleton: output an arbitrary frame buffer through kitty graphics, with placeholders
  for the register/disassembly panels

**DoD:** A dummy frame buffer is shown in both Ghostty and kitty. One test ROM builds and
`runner` loads it.

### Phase 1. CPU
- The ARMv4T common part (ARM + Thumb): barrel shifter, flags, pipeline (PC+8/+4),
  exceptions, mode switches
- Verified with a per-instruction JSON test suite for the ARM7TDMI
- ARMv5TE extensions: BLX, CLZ, the QADD family, LDRD/STRD, PLD, the Thumb switch on
  `LDR PC` / `POP {PC}` (v5 only)
- Minimal CP15: TCM configuration, exception vector location. The MPU is ignored
- A disassembler (for the debugger panel)

**DoD:** All JSON tests pass. A self-written test passes for each v5 extension.

### Phase 2. First pixel (ARM9 alone)
- Parse the NDS ROM header, direct-boot the ARM9 binary
- Main RAM, ITCM/DTCM, LCDC mapping of VRAM bank A
- Only engine A display mode 2 (direct VRAM output)
- ARM7 is a stub

**DoD:** The image a test ROM writes to VRAM is shown in Ghostty and pinned by a
golden-frame test.

### Phase 3. Dual CPU and the system
- Add ARM7; the scheduler alternates the two CPUs
- The two bus views, WRAMCNT
- IPC SYNC/FIFO, interrupts (IE/IF/IME), timers, DMA
- HLE SWIs (CpuSet, LZ77 decompression, Div, …)
- Engine A text BG modes

**DoD:** A libnds-based homebrew's console printf output displays correctly.

### Phase 4. 2D complete
- All of engines A/B: every BG mode, sprites, extended palettes, windows, blending, master
  brightness
- Full VRAM bank mapping (page table)
- Display capture

**DoD:** The golden frames of the self-written test ROM for each 2D feature pass.

### Phase 5. Conditions for booting commercial games
- Cartridge protocol (ROM read commands, assuming a decrypted dump)
- AUXSPI save chips (EEPROM / FLASH / FRAM) + type detection
- **Synthetic firmware** (user name, language, touch calibration)
- RTC, touch controller SPI, power management
- Respond to sound registers first (for games that stall waiting on sound)
- Save file save/load

**DoD:** Pokémon Diamond boots, starts a new game, saves, and loads that save again after a
restart. Its overworld is 3D, so the 3D engine may be a stub that accepts geometry commands
without drawing until Phase 7.

### Phase 6. Sound
- 16-channel mixing, PCM / ADPCM / PSG, sound capture
- Connect `cpal`, switch to audio-driven pacing

**DoD:** Snapshot tests of test-ROM output waveforms pass, and commercial game sound is
correct.

### Phase 7. 3D
- Geometry engine: command FIFO, matrix stacks, vertex transform, clipping, polygon RAM
- Rendering engine: a software rasterizer. Edge rules, Z/W buffer, the 7 texture formats,
  toon shading, edge marking, fog, anti-aliasing
- `rayon` scanline parallelism

**DoD:** The golden frames of the test ROM for each 3D feature pass, and a 3D commercial
game (Pokémon Diamond) is playable.
> Note: generation 4 and 5 Pokémon games (DP, HGSS, BW) have 3D overworlds, so this phase
> is required for them.

### Phase 8. Performance and accuracy
- Profiling-driven optimization: memory fast path, interpreter hot path
- Block caching if needed. A JIT is the last resort, given macOS's `MAP_JIT` constraints

**DoD:** The main games run at full speed on the target machine (Apple Silicon Mac).

### Parallel track (from Phase 1 onward)
- Expanding the debugger panels
- A trace diff tool (using a reference emulator **as an oracle**, automatically finding the
  first divergence)
- Replay-based regression tests

---

## 6. Risks and responses

| Risk | Response |
|---|---|
| Verifying the CPU without a system around it | JSON unit tests + self-written bare-metal test ROMs |
| Debugging two CPUs at once | Bring up ARM9 alone, then add ARM7 |
| Losing motivation because the first screen takes long | The shortest path to a first pixel through display mode 2 |
| Commercial games freezing for no visible reason | Synthetic firmware, save chip detection, the scheduler event log, trace diff |
| Little test material for v5TE | Self-written test ROMs first, trace comparison against a reference emulator |
| GPL code contamination | Clean-room principles; write our own spec docs (`docs/`) from GBATEK before implementing |

---

## 7. Development workflow (for AI agents)

### 7.1 Standard procedure for implementing a hardware feature
1. **Write up the spec:** read the relevant GBATEK section and summarize it **in your own
   words** in `docs/hw/<feature>.md`. Record register addresses, bit fields, behavior, edge
   cases, and unverified points.
2. **Tests first:** write tests that verify the feature. Unit tests for the CPU; otherwise a
   test ROM in `testroms/` + the expected result (a golden frame hash or a memory-dump
   condition).
3. **Implement:** implement it in `core`. Check it against the principles (4.2).
4. **Expose to the debugger:** when new state appears, make it visible in a debugger panel.
5. **Verify:** all tests + golden-frame regression + clippy + fmt.
6. **Record:** design decisions are recorded as GitHub issues labeled `decision`.

### 7.2 Standard procedure for investigating a bug
1. Pin the reproduction with an input replay
2. Find the first divergence from the reference with a trace diff
3. Check the hardware feature at the divergence in `docs/hw/`; if it is missing, write up
   the spec first
4. Add the reproduction test first, then fix

### 7.3 Commit and PR rules
- One PR = one hardware feature or one bug
- A PR that changes the core must come with tests
- When a golden frame hash changes, the PR description states the reason

---

## 8. Agent rules (never violate)

1. Do not add commercial ROMs, BIOS, firmware dumps, or anything extracted from them to the
   repository.
2. Do not copy GPL emulator code or translate it line by line. When referring to it,
   summarize only "what problem was solved, and how".
3. Do not add IO, system time, randomness, or thread dependencies to the `core` crate.
4. Do not put `Rc`, `RefCell`, closures, or raw pointers in state. All state must be
   serde-serializable.
5. Do not put dynamic dispatch or heap allocation on the hot path (CPU execution, memory
   access, pixel processing).
6. Do not use `unsafe` without a measured justification and a `// SAFETY:` comment.
7. Do not change the core without tests.
8. Do not implement behavior with an uncertain spec by guessing. Record it as "unverified" in
   `docs/hw/` and leave a `TODO(hw-unverified)` comment.
9. Do not implement features beyond the current phase's scope. If needed, a stub +
   `TODO(phaseN)`.
10. Do not use trademarks (Nintendo, DS, Pokémon, …) in crate names, binary names, or
    identifiers. Register and part names needed in a hardware context are exempt.

---

## 9. References

- **GBATEK** (Martin Korth): the primary GBA/NDS hardware documentation
- **The melonDS blog**: hardware oddities and implementation notes (no code reference;
  concepts only)
- **NooDS**, **Dust** (Rust): single-developer / small NDS emulator implementations
  (concepts only)
- **BlocksDS**: the DS homebrew SDK used to build test ROMs
- ARM Architecture Reference Manual (ARMv5TE), ARM7TDMI / ARM946E-S Technical Reference
  Manuals
- A per-instruction JSON test suite for the ARM7TDMI
- kitty documentation: graphics protocol, keyboard protocol
- Ghostty documentation: supported protocols

---

## 10. Decisions needed

- [x] Project name: `ninge` (available on crates.io); repository `github.com/devgyurak/ninge`
- [x] License: `Apache-2.0`
- [ ] Whether Ghostty supports kitty graphics shared-memory transfer (confirmed in Phase 0;
      direct zlib transfer is the default until then)
- [x] Savestates: a binary serde encoding with a magic number and a format version; no
      backward compatibility before 1.0, and a version mismatch is refused with a clear error
- [x] The first commercial target game: Pokémon Diamond (Phase 5 boot-to-save, Phase 7 fully
      playable)
