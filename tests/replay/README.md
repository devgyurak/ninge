# Replay logs

Hand-written input logs (`*.ninput`, format in
[docs/contributing/input-log.md](../../docs/contributing/input-log.md)) for our own test ROMs in
`testroms/`. They are synthetic data: none was recorded from a commercial game, and none holds a
ROM's name, path, or bytes. `crates/runner/tests/replay.rs` replays each one and checks that the
header fits the built ROM, that the final frame matches its golden (`tests/golden/`), that two
runs and a save-load-continue run agree, and that the log changes the run.

| Log | ROM | Frames | What it shows |
| --- | --- | --- | --- |
| `keyirq.ninput` | `keyirq` | 10 | the same input as `crates/runner/tests/keyirq.rs`; matches `tests/golden/keyirq.txt` |
| `touch.ninput` | `touch` | 4 | the same input as `crates/runner/tests/touch.rs`; matches `tests/golden/touch.txt` |
| `keypad.ninput` | `keypad` | 4 | what the ROM reads in KEYINPUT; the record is checked, the screens show nothing |
| `rtc-wall.ninput` | `rtc` | 4 | `wall=` reaches the clock chip: the chip starts on the recorded date; golden in `tests/golden/rtc-wall.txt` |

A log's `rom=` is the ROM's header CRC16, so changing a test ROM's source changes it. The
mismatch error names the new value (`the log is for a ROM with header CRC 0000, not 79ca`); put
it in the header line. Do not regenerate a golden to make a replay pass (AGENTS.md §5.6).
