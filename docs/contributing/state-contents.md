# What a savestate holds: large buffers, the ROM, the save generation and the size

Longer form of `AGENTS.md` §2 rule 5 (all state is plain data, savestates from day one) and
`docs/hw/` notes' "Savestate" paragraphs. Written for #322, part 4; the decisions are the
maintainer's, recorded in the pull request that added this page.

## One convention for a large buffer

A saved buffer takes one of three shapes. New state picks by what the hardware says about its
length:

| The buffer is | Use | Check on load | Example |
|---|---|---|---|
| at most 32 elements | a plain array `[T; N]` (serde derives it) | none when every bit pattern is a value (the matrix stacks); a mask check in `well_formed` when bits are unused (`RenderRegs::edge_color`) | the matrix stacks, `RenderRegs::edge_color` |
| a fixed size that serde cannot derive as an array | `Vec<T>` made at full length, never resized | `well_formed` checks the length | the four frame buffers (`Machine::top` and the three others), `gpu3d::queue::Queue::fifo`, main RAM |
| a count that varies up to a hardware bound | `Vec<T>` made with `with_capacity(bound)` | `well_formed` checks the count against the bound; `Checked::restore` reserves the room again, since a decoded `Vec` has exactly its length | `gpu3d::polygon::Ram` (Polygon RAM and Vertex RAM) |

Rules for all three:

- A buffer is never grown past its bound in a command, an access or a pixel path
  (`AGENTS.md` §2 rule 3). A `Vec` that is allowed to hold fewer than its capacity says so in its
  doc comment, and drops what does not fit.
- A load never trusts a buffer's length: the length is checked in `well_formed`, and a state whose
  buffer has the wrong one is refused as corrupt, not padded or truncated.
- A buffer that is derived from other saved state is not saved, or is rebuilt in
  `Checked::restore`, or is compared with its source in `well_formed` (see "Derived values").

## Derived values

`Machine::load_state` names no device. What a load must do after the checks passed is
`Checked::restore(now)`, which `Io` walks over every device (`io/registry.rs`), and
`Io::after_load`.
A value computed from other saved fields is one of:

- **rebuilt** in `restore`: the clip matrix (a product of the position and projection matrices),
  the room in Polygon RAM and Vertex RAM;
- **checked** against its source in `well_formed`, and a state that disagrees is refused: a sound
  channel's end time (from the latched registers and the sample clock), what a started channel
  latched (it must fit what the registers hold and name its own channel), and the parsed header
  (from the ROM);
- **not saved**: the page tables, the event queue, the frame's audio (`#[serde(skip)]`), rebuilt by
  `load_state`.

VCOUNT and the HBlank flag are derived from the master time too: the line is
`(t / LINE_CYCLES) % 263` and the flag is set from `HBLANK_START_CYCLES` into the line, where `t` is
the time the line events have reached (`now`, or `now - 1` in a round a stop interrupted after the
ARM9 ran, because the events due at `now` have not fired there). A load checks them against that
time at the machine level, not in `Timing`'s own `well_formed`, which does not know it; the check
arrives with the run control that makes such a round possible (`scheduler.md`, "Savestate
checks"). A line start only adds one to VCOUNT and never looks at the time, so a wrong value would
be kept for good, and a frame boundary would sit 192 lines off. A lone `step_arm9` therefore fires
every due event (`scheduler.md`, "Debugger single-step"). A machine with no program runs no events
and is not checked.

Derived but unchecked, known: the level IF bit of the geometry FIFO, and a few due times.

Not derived, and not checked against a source on purpose, because the hardware lets a program reach
any value: the division and square-root results (`divsqrt`) are a function of the inputs written
and a power-on state that does not satisfy the function, so a check would need the power-on case
apart. They are listed here as the known unchecked part; CP15's masks are enforced at write time
only.

## The ROM as a `#[serde(skip)]` field of `Machine`

The ROM image is an input, not state: a savestate never embeds a possibly commercial image, and a
state loads onto a machine that was handed the same ROM again (the length and a hash in the header
say which). So it is skipped on save and moved by hand in `load_state`.

Decided: it stays a field of `Machine`. The cost is that `Clone` copies the whole image, which
matters for rewind and run-ahead; no phase plans those yet. Moving the ROM out of `Machine` (an
`Arc<[u8]>` or a borrow held by the host) changes the host API and the AGENTS.md rule against
`Arc` in core, so it is its own pull request, taken when rewind or run-ahead is planned, if at all.

## The save generation

`Generation` (in the save chip, `cart/save/chip.rs`) counts the changes to the save memory for the
host. It is host bookkeeping, so it is `#[serde(skip)]`, and its `PartialEq` is always true so that
a loaded state equals the machine it was saved from. `Io::after_load` moves it one past the
machine's count before the load, since the save memory may differ from what the host last wrote.

Decided: kept as it is. The always-true `PartialEq` is the price of keeping the counter inside the
chip; moving the counter to a host-side field of `Machine` would remove it and is not worth a
layout change now.

## The size of a state

The savestate layout test pins the length: 5,479,892 bytes at power-on and 6,069,816 for the
counter test ROM after two frames. The large parts are main RAM (4 MiB), VRAM (656 KiB), the
firmware image (256 KiB) and the save memory, and the four frame buffers (`top`, `bottom`,
`work_top`, `work_bottom`: 49,152 `u32` pixels each, saved so that save, load and continue equals
an uninterrupted run, #28).

Decided: no change now. A state of this size is fine for a save slot and a replay checkpoint. It is
too large to take every frame: rewind or run-ahead would have to delta or compress what changes,
keep the frame buffers out of the checkpoint, or both. That work is planned with those features,
not before.
