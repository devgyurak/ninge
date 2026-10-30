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
