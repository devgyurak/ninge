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

Line 1 of every source file is the SPDX line (`@ ...` in ARM assembly). Addresses come from
`docs/hw/cart-header.md` and `docs/hw/memory-map.md`.

## Running a ROM headlessly

`ninge-runner` loads a ROM, runs N frames, and prints one line with the frame count and an
FNV-1a 64 hash of each screen (the hash is defined in [`tests/golden/README.md`](../tests/golden/README.md)):

```bash
cargo run -p ninge-runner -- testroms/build/smoke.nds --frames 3
# frames=3 top=<016x> bottom=<016x>
```

Exit code 0 on success, 1 on an IO error (missing ROM), 2 on a usage error. Tests find a
built ROM with `ninge_runner::testrom_path("smoke")`.
