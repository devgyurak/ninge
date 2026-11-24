# Input log format

An input log is the record of what the host gave the machine, one frame at a time. The core's
contract is that one `Input` per frame is a complete input log (AGENTS.md §2 rule 6), so a log
plus a ROM reproduces a run: the same frames, audio, and savestate bytes.

The format is **version 1**. The module that reads and writes it is pure and lives in
`ninge-core` (`crates/core/src/input_log/`), like the savestate format: it turns text into
`Input` values and back and does no IO. The frontend (`ninge <rom> --record PATH`) writes logs
and `ninge-runner replay` reads them.

## Shape

Line-oriented UTF-8 text, one frame per line. The same text can go to a file or to a pipe
unchanged, and a file cut off at any point is still a valid prefix.

```
ninge-input 1 rom=1a2b save=flash-512k:00112233aabbccdd
# a comment line
f=1 b=A,START wall=789012345
f=2 b=A t=128,96 wall=789012345
f=3 hinge mic wall=789012346
f=4
```

Blank lines and lines that start with `#` are ignored, so a hand-written log can carry comments.

## Header

The first line that is neither blank nor a comment:

```
ninge-input <version> rom=<crc16 hex> save=<type>:<hash>
```

- `<version>` is the format version, `1`.
- `rom=` is the ROM's **header CRC16** (`Header::crc`), in lowercase hex. A log never records
  the ROM's name, its path, or its bytes.
- `save=` names the save memory the run started with. `<type>` is a name from `--save-type`
  (`flash-512k`, `none`, ...). `<hash>` is FNV-1a 64 over the save memory as the machine held it
  **before frame 1**, as 16 lowercase hex digits. It makes a replay against a different save an
  error instead of a silent divergence.

The hash is the one the golden frames use (`tests/golden/README.md`).

## Frame lines

`f=<n>` comes first. `n` starts at 1 and each line is the previous plus one. After it come
space-separated fields, in any order:

| Field | Meaning |
| --- | --- |
| `b=<names>` | Buttons held, comma-separated, any order: `A B SELECT START RIGHT LEFT UP DOWN R L X Y` |
| `t=<x>,<y>` | The panel is pressed at lower-screen pixel `x` 0-255, `y` 0-191 |
| `hinge` | The hinge is closed |
| `mic` | The player is blowing into the microphone |
| `wall=<u32>` | The frame's `wall_time`, decimal seconds since 2000-01-01 |

**A field at its default is omitted**: no buttons, no touch, hinge open, no mic, `wall_time` 0.
So `f=4` is a frame with nothing happening, and a log written before a field existed still
replays, with that field at its default. The writer emits buttons in bit order.

`wall=` is part of the log. The clock chip reads it, so a replay that takes the host's clock
instead of the recorded value does not reproduce the run.

## What is refused

A log is refused with an error that names the line, never half-loaded:

- a version other than `1`;
- an unknown key or a key given twice in one line;
- a malformed value, or a touch outside 0-255 by 0-191;
- a missing `f=`, or one that is not the previous frame plus one;
- a header whose ROM CRC or save hash does not match the ROM and save being replayed.

## A truncated tail

Only newline-terminated lines count. A final line with no `\n` is dropped, because a writer
may have been cut off in the middle of it, and the parser reports that it dropped one. This is
not an error: a log cut off mid-write replays as its prefix.

## Recording

```bash
ninge <rom> --record PATH
```

The frontend writes the header before frame 1, from the loaded ROM's header CRC and the save
memory it holds then, and one line per frame it runs, flushed as it goes. Each line is exactly
the `Input` the frame was run with, wall-clock time included. `PATH` must not exist: an existing
file is refused, not overwritten. A paused run (`--focus-pause`) runs no frame and records none.
If a write fails, recording stops and the run goes on; the error is printed when the program
ends. The log holds no ROM name, path, or bytes.

## Replaying

```bash
ninge-runner replay <rom> <log> [--frames N] [--save PATH]
```

It runs the log's frames, or `N` frames: frames past the end of the log get no input. The
save file is read the same way a plain run reads it (`--save`, else beside the ROM) and is
**never written**, so a replay can be repeated; it must hold what it held when the recording
began, which the header's hash checks. It prints the usual `frames=N top=... bottom=...` line.
A log that is refused, or that was recorded for another ROM or save, exits 5 before any frame
runs; a dropped partial last line is reported on stderr.

## Compatibility

A new field gets a new key and a default, so an old log stays valid; an old reader refuses the
new key as unknown rather than ignore it. A change to the meaning of an existing line bumps the
version. Before 1.0 there is no backward compatibility across versions, as for savestates.

## Out of scope

- A replay that starts from a savestate.
- Streaming input through a pipe, or reading frame results back. The format is designed so the
  same line can be sent one frame at a time; the run mode that does it comes later.
