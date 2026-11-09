# testroms

Our own bare-metal test ROM sources. **Built ROMs are never committed** (AGENTS.md §3.1):
only sources live here; `testroms/build/` is git-ignored.

## Build

```bash
./scripts/build_testroms.sh          # needs Docker; writes testroms/build/*.nds and SHA256SUMS
```

The script runs `make -C testroms` inside the BlocksDS image pinned by digest in the script
itself. Local runs, `./scripts/verify.sh`, and CI all go through it.

## Layout

| Path | Purpose |
|---|---|
| `Makefile` | Builds every ROM in `ROMS` into `build/` and writes `build/SHA256SUMS`. |
| `smoke/` | Minimal ROM: ARM9 and ARM7 each spin in an infinite loop from their own entry point. No libnds, no BlocksDS crt0. |
| `boot/` | Direct-boot ROM: loads at `0x02000000` but enters at `0x02000100`, records r0-r14, CPSR and the IRQ/Supervisor stack pointers at `0x02000200`, then spins. Its addresses come from per-ROM variables in the `Makefile`. |
| `memmap/` | Memory-map ROM: exercises main RAM and its mirrors, both TCMs, VRAM bank A and an undefined I/O port from `0x02000100`, and records the values it read at `0x02004000` (slot layout at the top of `memmap/arm9.s`). It records observed values only; the expected ones are in `crates/runner/tests/memmap.rs`. |
| `vramdisp/` | VRAM-display ROM: writes POWCNT1 (display swap clear), VRAMCNT_A (MST 0) and DISPCNT (mode 2, block 0), then computes a test picture into bank A: colour bars, then red/green/blue/grey ramps, with bit 15 set on odd bar rows (layout at the top of `vramdisp/arm9.s`). Entered at `0x02000100`. No image asset. Expected pixels are in `crates/runner/tests/vramdisp.rs`. |
| `sched/` | Scheduler ROM: the ARM9 and the ARM7 each count forever in their own loop, storing the counter at `0x02001000` (ARM9) and `0x02002000` (ARM7) after every increment. The counters show how many instructions each CPU ran per frame (`docs/hw/scheduler.md`). Entered at its load address; expected values and the derivation are in `crates/runner/tests/scheduler.rs`. |
| `wramcnt/` | Shared-WRAM ROM: the ARM9 sets WRAMCNT to 0, 1, 2 and 3 in turn and writes a marker at `0x03000000` and `0x03004000`; the ARM7 reads what it sees at `0x03000000`, `0x03004000`, `0x037F8000` and `0x03800000`, reads WRAMSTAT, writes its own markers and reads them back. They hand over through a flag at `0x02003000` (no IPC yet). ARM9 observations are at `0x02004000`, ARM7 ones at `0x02005000` (slot layouts at the top of each `.s`). It records observed values only; the expected ones are in `crates/runner/tests/wramcnt.rs` (`docs/hw/wramcnt.md`). Entered at its load address. |
| `irq/` | Interrupt-controller ROM: each CPU reads IME, IE and IF at reset, writes all ones to them, probes byte and halfword writes to IE and IME and a write of ones to IF, and writes a pattern to its own IE; they hand over through a flag at `0x02003000` (no IPC yet) so each reads its IE again after the other has written its pattern. Nothing requests an interrupt and I stays set. ARM9 observations are at `0x02004000`, ARM7 ones at `0x02005000` (slot layouts at the top of each `.s`). It records observed values only; the expected ones are in `crates/runner/tests/irq.rs` (`docs/hw/interrupts.md`). Entered at its load address. |
| `disptime/` | Display-timing ROM: each CPU polls VCOUNT over one frame (264 values) and records its DISPSTAT and IF at chosen lines: the VBlank flag around lines 100, 192, 261, 262 and the wrap, the HBlank flag in line 50, the VCount-match flag with the setting 100, a write of FFFF to DISPSTAT, and IF latching of HBlank, VCount-match and VBlank with IME = 0 and clear-by-writing-1. The ARM9 also enables the ITCM, clears the high-vectors bit and takes the VCount and VBlank IRQs through a handler at `0x18`; the ARM7 checks IF only (its vector is BIOS, #27). The two CPUs run independently, no handshake. ARM9 observations are at `0x02004000`, ARM7 ones at `0x02005000`, the VCOUNT sequences at `0x02010000` and `0x02012000` (layouts at the top of each `.s`). Expected values are in `crates/runner/tests/display_timing.rs` (`docs/hw/display-timing.md`). |
| `ipc/` | IPC ROM: the two CPUs take turns through IPCSYNC itself (each step posts a nibble in IPCSYNC bits 8-11 and the next owner waits for it in bits 0-3; no RAM flag). They pin IPCSYNC's two nibbles and its IRQ (sent to a CPU that is not listening, then to one that is), the IPC FIFOs (send and receive order, the empty/full flags, the 17th send and the empty read with their error bit and its acknowledge, a flush by the sender, a disabled FIFO) and IF.17 and IF.18 (edge behavior and enable-on-condition). The ARM9 enables the ITCM, clears the high-vectors bit and takes IF.16, IF.17 and IF.18 through a handler at `0x18`; the ARM7 checks IF only (its vector is BIOS, #27). ARM9 observations are at `0x02004000`, ARM7 ones at `0x02005000` (layouts and the step table at the top of each `.s`). Expected values are in `crates/runner/tests/ipc.rs` (`docs/hw/ipc.md`). |
| `timers/` | Timers ROM: each CPU measures its own four timers against the frame (the edge into VCOUNT 192 starts and stops them): F/1024 and F/256 counts per frame, an F/1 timer feeding a count-up timer (8 overflows a frame), a stop that freezes the counter, a restart that loads the reload, a reload written while running, a 32-bit write of reload and start, IF latching only when the timer's IRQ enable is set, and timer 0's ignored count-up bit. The ARM9 enables the ITCM, clears the high-vectors bit and takes timer 3's IRQ through a handler at `0x18`; the ARM7 checks IF only (its vector is BIOS, #27). The two CPUs run independently, no handshake. ARM9 observations are at `0x02004000`, ARM7 ones at `0x02005000` (layouts at the top of each `.s`). Expected values are in `crates/runner/tests/timers.rs` (`docs/hw/timers.md`). |

The header is built with a 0x4000-byte header size (`ndstool -h`), so the ARM9 offset meets the
range in `docs/hw/cart-header.md`. Line 1 of every source file is the SPDX line (`@ ...` in ARM assembly). Addresses come from
`docs/hw/cart-header.md` and `docs/hw/memory-map.md`.

## Running a ROM headlessly

`ninge-runner` loads a ROM, runs N frames, and prints one line with the frame count and an
FNV-1a 64 hash of each screen (the hash is defined in [`tests/golden/README.md`](../tests/golden/README.md)):

```bash
cargo run -p ninge-runner -- testroms/build/smoke.nds --frames 3
# frames=3 top=<016x> bottom=<016x>
```

Exit code 0 on success, 1 on an IO error (missing ROM), 2 on a usage error, 3 when the machine
refuses the ROM (an invalid header, `docs/hw/cart-header.md`). Tests find a
built ROM with `ninge_runner::testrom_path("smoke")`.
