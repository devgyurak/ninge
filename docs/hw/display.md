# Display: the two screens

Sources: GBATEK "DS Technical Data" (the hardware diagram labels each LCD as 256x192 pixels)
and "DS Video" (the video hardware overview).
Used by: `crates/core/src/lib.rs` (`Machine`, `Screen`, `SCREEN_WIDTH`, `SCREEN_HEIGHT`)   Phase: 0

## Behavior

The handheld has two physical LCD screens, one above the other: the top screen and the
bottom screen. Each is 256 pixels wide and 192 pixels tall, so each shows 49,152 pixels per
frame. The bottom screen carries the touch panel.

Phase 0 implements only the shape of the output: two frame buffers of that size, readable
per screen.

## Output format (not hardware)

The emulator hands frames to the host as one `u32` per pixel in xRGB8888 (`0x00RRGGBB`).
This is a host-side choice that keeps 6-bit hardware channels expandable later. It is not a
hardware fact. The all-black initial contents are a placeholder, not the power-on state.

## Edge cases

None in Phase 0.

## Unverified

- Which graphics engine drives which screen, and how that is swapped — governed by POWCNT1;
  Phase 2–3 scope, not implemented yet.
- The exact GBATEK section that gives 256x192 in prose; the diagram label in "DS Technical
  Data" is the source cited here.
