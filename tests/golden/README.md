# Golden frames

A golden test runs one of our own test ROMs (`testroms/`) for N frames and compares a hash
of each screen with a reviewed value stored here. The helpers live in
`ninge_runner::golden` (`assert_frame_hash`, `assert_deterministic`). The hash and the pixel
format were decided in [#68](https://github.com/devgyurak/ninge/issues/68).

## What is hashed

- **Pixels:** the xRGB8888 `u32` values that `Machine::frame()` returns, not the hardware's
  native 15/18-bit colors.
- **Hash:** FNV-1a 64 (`ninge_runner::frame_hash`) over the little-endian bytes of each
  pixel, in row order.
- **Screens:** top and bottom are hashed separately and never merged.
- **Consequence:** if the 5-to-8-bit color expansion changes, every golden changes (#68).

- **Audio (optional):** `audio=<16 lowercase hex digits>`, the FNV-1a 64 hash
  (`ninge_runner::audio_hash`) over the little-endian bytes of every `i16` sample the run made,
  all frames, in order (`Machine::audio()` after each frame; interleaved left then right). The
  samples are 10-bit mixer output scaled to `i16` (`docs/hw/spu.md`, "Output samples"). A line
  without `audio=` does not check the audio, so every golden that exists before the SPU is
  unchanged; a line with it must match. The format extension is recorded on #68.

## File format

`tests/golden/<rom>.txt`, where `<rom>` is the name of a ROM in `testroms/` (one exception: a replay of an input log whose run differs from the ROM's run with no input has its own file named after the log, such as `rtc-wall.txt`, so the two never share a file):

```text
# Comment lines start with '#'; blank lines are allowed.
frames=3 top=<16 lowercase hex digits> bottom=<16 lowercase hex digits> [audio=<16 lowercase hex digits>]
```

Each data line is the output of `ninge-runner`. Fields are separated by whitespace and
whitespace around a line is ignored; the field values themselves must be exact:

- `frames` is a plain decimal: ASCII digits only, no sign, no leading zero (`0` alone is fine).
- `top` and `bottom` are 16 lowercase hex digits: no `0x` prefix, no uppercase, no shorter or
  longer value.
- The fields come in the order `frames`, `top`, `bottom`, then the optional `audio`, with
  nothing after them.
- A `frames=N` value appears at most once per file.

## Review flow

- **New golden:** run the ROM with the CLI, paste the line by hand, and say in the PR why
  that frame is the right one.

  ```bash
  ./scripts/build_testroms.sh
  cargo run -p ninge-runner -- testroms/build/<rom>.nds --frames N          # screens only
  cargo run -p ninge-runner -- testroms/build/<rom>.nds --frames N --audio  # and audio=
  ```

  A golden with `audio=` is pasted from the `--audio` line. **Listen first:** `--wav PATH`
  writes the run's audio as a stereo 16-bit WAV (32728 Hz) for a local listen; the file is
  git-ignored (`*.wav`) and never committed. Say in the PR that you listened, and what you
  heard. A hash alone is not a review.

- **Changed golden:** the PR names the hardware behavior that changed and why the new frame
  is right (AGENTS.md §5.6). A golden mismatch is a regression until shown otherwise.
- **No bulk regeneration.** There is no update mode and the helpers never write files; do
  not accept hashes in bulk to turn a test green.
- **Only our own ROMs.** Goldens come from `testroms/` and never from a commercial game.
