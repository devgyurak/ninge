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

## File format

`tests/golden/<rom>.txt`, where `<rom>` is the name of a ROM in `testroms/`:

```text
# Comment lines start with '#'; blank lines are allowed.
frames=3 top=<16 hex digits> bottom=<16 hex digits>
```

Each data line is exactly the output of `ninge-runner`. A `frames=N` value appears at most
once per file.

## Review flow

- **New golden:** run the ROM with the CLI, paste the line by hand, and say in the PR why
  that frame is the right one.

  ```bash
  ./scripts/build_testroms.sh
  cargo run -p ninge-runner -- testroms/build/<rom>.nds --frames N
  ```

- **Changed golden:** the PR names the hardware behavior that changed and why the new frame
  is right (AGENTS.md §5.6). A golden mismatch is a regression until shown otherwise.
- **No bulk regeneration.** There is no update mode and the helpers never write files; do
  not accept hashes in bulk to turn a test green.
- **Only our own ROMs.** Goldens come from `testroms/` and never from a commercial game.
