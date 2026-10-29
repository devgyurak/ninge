# Display: the two screens

Sources: GBATEK "DS Technical Data" (video list: two LCD screens, 256x192 pixels each,
18-bit colour depth); "DS Display Dimensions / Timings" (256 visible dots per line, 192
visible lines); "DS Touch Screen Controller (TSC)" (the touch controller belongs to the lower
screen); the `4000304h POWCNT1` register description (bit 15, display swap between the upper
and lower screen).
Used by: `crates/core/src/lib.rs` (`Machine`, `Screen`, `SCREEN_WIDTH`, `SCREEN_HEIGHT`)   Phase: 0

## Behavior

The handheld has two physical LCD screens, one above the other: the upper (top) screen and
the lower (bottom) screen. Each shows 256 visible dots per line over 192 visible lines, so
each frame is 49,152 pixels per screen. The touch controller belongs to the lower screen.

The words top and bottom name the physical screens, not the graphics engines. POWCNT1 bit 15
swaps which engine's output goes to the upper and which to the lower screen.

Phase 0 implements only the shape of the output: two frame buffers of that size, readable
per screen.

## Output format (not hardware)

The emulator hands frames to the host as one `u32` per pixel in xRGB8888 (`0x00RRGGBB`).
This is a host-side choice, not a hardware fact: the LCDs have 18-bit colour depth ("DS
Technical Data"), and xRGB8888 is wide enough to hold it. The all-black initial contents are
a placeholder, not the power-on state.

## Edge cases

None in Phase 0.

## Not implemented yet

- Routing of the two graphics engines to the two screens through POWCNT1 bit 15 (Phase 2–3).

## Unverified

None.
