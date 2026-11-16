# Colour special effects and master brightness (both engines)

Sources: GBATEK "LCD I/O Color Special Effects" (BLDCNT, BLDALPHA, BLDY, semi-transparent objects,
the OBJ layer); GBATEK "LCD I/O Window Feature" (the effect bit of each region); GBATEK "DS Video"
(MASTER_BRIGHT, `0400_006Ch`); GBATEK "DS Video Display System Block Diagram" (where each stage
sits); GBATEK "DS 3D 2D Integration" ("Special Effects", "Window Feature"); GBATEK "DS Video OBJs"
(the alpha value of a bitmap object).
Used by: `crates/core/src/gpu2d/`   Phase: 4 (#34); the debugger panel that shows the registers is
#58, the capture unit that reads the line before the master brightness is #36

`bg-text.md`, `bg-modes.md` and `obj.md` say how each layer makes a pixel, and `windows.md` says
which layers a pixel may use. This note says what the hardware does with the layers that remain:
it mixes the two top ones, or lightens or darkens the top one, and then (a separate stage) the
master brightness lightens or darkens the whole finished picture. Both engines have the same
circuits at the same offsets.

## Registers

Engine A's block starts at `0400_0000h`, engine B's at `0400_1000h`.

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `+0050h` (`0400_0050h`, `0400_1050h`) | BLDCNT | 16 | R/W | 0 |
| `+0052h` | BLDALPHA | 16 | R/W (see "Unverified") | 0 |
| `+0054h` | BLDY | 16 | W | 0 |
| `0400_006Ch`, `0400_106Ch` | MASTER_BRIGHT | 16 | R/W (see "Unverified") | 0 |

The reset values are not stated by GBATEK; zero means "no effect", which is also what makes a ROM
that never touches these registers draw the same picture as before (see "Unverified").
Engine B's `+0050h` block is behind POWCNT1 bit 9 like the rest of its 2D registers
(`bg-modes.md`, "Engine B"); its MASTER_BRIGHT at `106Ch` is not, since it lies outside the
`1008h-105Fh` range GBATEK gives for that bit.

### BLDCNT bits

| Bits | Meaning |
|---|---|
| 0-3 | BG0-BG3 is a first target |
| 4 | OBJ is a first target (the top-most object pixel) |
| 5 | the backdrop is a first target |
| 6-7 | effect: 0 none, 1 alpha blending, 2 brightness increase, 3 brightness decrease |
| 8-11 | BG0-BG3 is a second target |
| 12 | OBJ is a second target |
| 13 | the backdrop is a second target |
| 14-15 | not used |

### BLDALPHA bits

| Bits | Meaning |
|---|---|
| 0-4 | EVA, the weight of the first target: 0-16 mean 0/16 to 16/16, 17-31 mean 16/16 |
| 5-7 | not used |
| 8-12 | EVB, the weight of the second target, same scale |
| 13-15 | not used |

### BLDY bits

| Bits | Meaning |
|---|---|
| 0-4 | EVY, the brightness coefficient: 0-16 mean 0/16 to 16/16, 17-31 mean 16/16 |
| 5-15 | not used |

### MASTER_BRIGHT bits

| Bits | Meaning |
|---|---|
| 0-4 | factor: 0-16 mean 0/16 to 16/16, larger values mean 16/16 |
| 5-13 | not used |
| 14-15 | mode: 0 off, 1 brightness up, 2 brightness down, 3 reserved |

## Behavior

### The layers the effect sees

For one screen pixel the engine has up to six candidates: BG0-BG3, the top-most object pixel, and
the backdrop (palette entry 0, always present, always opaque). A candidate takes part only when it
exists at that pixel: its DISPCNT enable is set, the window region of the pixel lets the layer
through (`windows.md`), and the pixel is not transparent. They are ordered as `bg-modes.md` and
`obj.md` say: by priority, with the ties those notes give. The *top* pixel is the first of that
order; the *next* pixel is the one right below it. The backdrop is always last.

Only the top object pixel exists at this stage. The object layer is reduced to it before the
effects run, so an object never blends with another object (GBATEK, "The OBJ Layer").

### Alpha blending (effect 1)

The pixel is blended when, and only when, the top pixel's layer is a first target **and** the next
pixel's layer is a second target. Each of red, green and blue is mixed on its own 5-bit value:

```
I = min(31, I_top * EVA + I_next * EVB)     with EVA and EVB as sixteenths
```

In any other case the top pixel shows unchanged, whatever BLDALPHA holds: only one target, a
non-target pixel between the two, or the second target above the first. A layer may be selected as
both targets; the top pixel then acts as the first target and the one below as the second.

### Brightness increase and decrease (effects 2 and 3)

When the top pixel's layer is a first target, each 5-bit channel `I` becomes

```
increase:  I + (31 - I) * EVY
decrease:  I - I * EVY                      with EVY as sixteenths
```

The second-target bits and BLDALPHA play no part. When the top pixel's layer is not a first target
the pixel is unchanged.

### Semi-transparent objects

An object whose attr0 mode is 1 (`obj.md`) is always a first target, whatever BLDCNT bit 4 says,
and always blends as in effect 1, whatever BLDCNT bits 6-7 say:

- Its pixel is the top pixel and the next pixel is a second target: the two are mixed with
  BLDALPHA. Blending has priority, so no brightness change is applied, neither to this pixel nor
  to the one below.
- Its pixel is the top pixel and the next pixel is **not** a second target: it is not mixed. BLDCNT
  bits 6-7 may still give it a brightness effect, as GBATEK says ("the BLDCNT register may be used
  to perform Brightness effects on the OBJ"): effect 2 or 3 lightens or darkens it as the first
  target it always is; effect 0 or 1 leaves it as it is. (This reading is in "Unverified".)

An object that is not top-most is hidden like any other and takes no part.

### The effect bit of a window region

Bit 5 of each window group (`windows.md`, "The colour special effect bit") says whether the
region lets the colour special effect work. With no window on (DISPCNT bits 13-15 all clear)
the effect works everywhere. With a window on, a pixel in a region whose bit 5 is clear shows its
top pixel unchanged: no blending, no brightness change. A semi-transparent object in such a region
is covered in "Unverified". The region is the one `windows.md` gives for that pixel, evaluated per
pixel like the layer enables.

### The 3D layer

When BG0 is the 3D layer (DISPCNT bit 3), it works as a second target as any layer does and as the
target of the brightness effects. As a *first* target for blending it uses a per-pixel alpha that
comes from the 3D engine, not EVA and EVB; GBATEK is unsure of the exact formula. The 3D engine is
a later phase: `TODO(phase7)`.

### Master brightness

MASTER_BRIGHT is applied to the finished picture of an engine, after the layers and their effects:

```
up:    new = old + (63 - old) * factor / 16
down:  new = old - old * factor / 16
```

on **6-bit** red, green and blue values, with the factor clamped to 16. Mode 0 applies nothing; mode
3 is reserved and is treated as mode 0 (see "Unverified").

In the block diagram (GBATEK, "DS Video Display System Block Diagram") the master brightness
stage of engine A sits after the display-mode selection: it applies to the layered picture
(mode 1), the VRAM display (mode 2) and the main-memory display (mode 3), not only to the layers.
It sits after the capture unit's source tap, so a capture reads the line without it
(`display-capture.md`, #36). Engine B's stage follows its layering and effects stage.

### Colour depth

The special effects work on the 5-bit channels the layers hold and the result stays 5-bit, so the
only conversion left is the host one in `display-mode-vram.md`, "Colour expansion". The master
brightness works on 6 bits, which GBATEK states for the register. A pixel that goes through it is
widened from 5 to 6 bits first (replicating the top bit: `c6 = (c5 << 1) | (c5 >> 4)`), adjusted,
and then expanded from 6 to 8 bits for the host as `(c6 << 2) | (c6 >> 4)`. That last step is a
host choice like the 5-to-8 one; changing it changes every golden frame that uses the master
brightness. A pixel of an engine whose master brightness mode is 0 or 3 keeps the 5-to-8 expansion, so no frame
that does not use the master brightness changes. The 6-bit path is taken whenever the mode is 1 or
2, even with a factor of 0, since the register is then "on"; at factor 0 it can differ from the
5-bit path by one unit in the low bit of a host channel.

## Edge cases

- BLDCNT effect 1 with no second-target bit set: no pixel is ever blended; objects with mode 1 are
  also not blended (they have no second target), see "Semi-transparent objects".
- EVA and EVB above 16 count as 16, so `EVA = EVB = 16` mixes to the saturated sum.
- EVA = 0 and EVB = 16 shows the second target at full strength: the first target vanishes.
- Brightness decrease with EVY of 16 gives black; increase gives white.
- All layers selected as both targets: the top pixel is first, the next is second.
- The backdrop below the last layer is a legal second target and a legal first target.
- BLDCNT bits 14-15 are not used and do not change the picture.
- A write to a byte of a register changes only the bits that byte holds, as for the other 2D
  registers.

## Unverified

Each item carries `TODO(hw-unverified)` at its code site.

- **Rounding.** GBATEK gives the formulas in sixteenths and does not say how a fraction is
  rounded. We truncate each product's sum toward zero (`(I1 * EVA + I2 * EVB) / 16`,
  `I + (31 - I) * EVY / 16`, `I - I * EVY / 16`, and the same for the master brightness). —
  `gpu2d::effects` and `gpu2d::brightness` — verify with a test ROM on hardware, or a trace
  against the reference oracle.
- **The 5-to-6-bit widening** for the master brightness. GBATEK says the unit works on 6-bit
  values and the layers hold 5-bit ones; how the extra bit is filled is not stated. — `gpu2d::brightness` — verify likewise.
- **MASTER_BRIGHT mode 3** (reserved): treated as mode 0, no change. — `gpu2d::brightness` —
  verify likewise.
- **Master brightness on display mode 0 (the white screen) and on a powered-off engine B.** We
  apply it to the white of mode 0 (the block diagram has it on every path) and not to the black of
  a powered-off engine. — `gpu2d::Gpu2d::select_line_a`, `render_line_b` — verify likewise.
- **Read-back.** GBATEK marks BLDALPHA "R/W (not W)" and BLDY "W (not R/W)" and does not say what
  MASTER_BRIGHT reads. We make BLDCNT and BLDALPHA read back their used bits, BLDY read as zero, and
  MASTER_BRIGHT read back its used bits; the unused bits read as zero everywhere. — `gpu2d::Engine::read_byte`
  — verify likewise.
- **Reset values** of BLDCNT, BLDALPHA, BLDY and MASTER_BRIGHT: zero. — `gpu2d::Engine::new` —
  verify likewise.
- **A semi-transparent object in a region whose effect bit is clear.** GBATEK does not say whether
  the always-on blending of such an object honours the bit. We let the bit suppress it: the object
  shows unchanged, since the bit is documented as the effect enable. — `gpu2d::effects` —
  verify likewise.
- **A semi-transparent object over a non-second target** under BLDCNT effect 2 or 3: lightened or
  darkened as a first target (the reading of "the BLDCNT register may be used to perform
  Brightness effects on the OBJ"). — `gpu2d::effects` — verify likewise.
- **The alpha value of a bitmap object** (attr2 bits 12-15, `obj.md`). GBATEK calls it the
  "Alpha-OAM value" and gives no formula, nor says whether it uses BLDALPHA. Bitmap objects are
  drawn opaque, as if the field were not there, and no golden depends on a guess. —
  `gpu2d::obj::bitmap_color` — verify likewise.
- **3D BG0 as a first target for blending**: `TODO(phase7)`, not a guess about the 2D path.

## Evidence

Golden frames come from our own test ROMs `testroms/blend` and `testroms/mbright`; their expected
pixels are worked out from this note, never read back from the emulator.
