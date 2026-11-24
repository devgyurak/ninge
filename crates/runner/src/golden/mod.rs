// SPDX-License-Identifier: Apache-2.0
//! Golden-frame helpers: compare a ROM's screen hashes with reviewed values.
//!
//! Decided in #68. A golden file is `tests/golden/<rom>.txt`; each data line has the form of
//! a [`RunReport`]'s `Display` output, `frames=N top=<016x> bottom=<016x>`. `#` starts a
//! comment line and blank lines are allowed. See `tests/golden/README.md`.

use std::fmt;
use std::path::{Path, PathBuf};

use ninge_core::cart::SaveType;
use ninge_core::{Input, Machine, Screen};

use crate::log::InputLog;
use crate::{
    RunReport, RunnerError, frame_hash, load_rom_file, run_with_audio, run_with_blank_save,
    testrom_path,
};

/// Why a golden file or a comparison against it failed.
#[derive(Debug, PartialEq, Eq)]
pub enum GoldenError {
    /// The golden file could not be read.
    Unreadable {
        /// The IO error's message, kept as text so the error stays comparable.
        reason: String,
    },
    /// A data line is not `frames=N top=<hex> bottom=<hex>`.
    Malformed {
        /// 1-based line number in the golden file.
        line: usize,
        /// What is wrong with the line.
        reason: String,
    },
    /// The same `frames=N` appears on two lines.
    DuplicateFrames {
        /// 1-based line number of the second occurrence.
        line: usize,
        /// The repeated frame count.
        frames: u64,
    },
    /// The golden has no line for the frame count that was run.
    MissingFrames {
        /// The frame count that was run.
        frames: u64,
    },
    /// The golden line for this frame count differs from the actual hashes.
    Mismatch {
        /// The reviewed line.
        expected: RunReport,
        /// What the run produced.
        actual: RunReport,
    },
}

impl fmt::Display for GoldenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { reason } => write!(f, "{reason}"),
            Self::Malformed { line, reason } => write!(f, "golden line {line}: {reason}"),
            Self::DuplicateFrames { line, frames } => {
                write!(f, "golden line {line}: frames={frames} is already defined")
            }
            Self::MissingFrames { frames } => {
                write!(f, "golden has no line for frames={frames}")
            }
            Self::Mismatch { expected, actual } => {
                write!(f, "expected: {expected}\n  actual: {actual}")
            }
        }
    }
}

impl std::error::Error for GoldenError {}

fn malformed(line: usize, reason: &str) -> GoldenError {
    GoldenError::Malformed {
        line,
        reason: reason.to_owned(),
    }
}

fn field<'a>(part: Option<&'a str>, key: &str, line: usize) -> Result<&'a str, GoldenError> {
    part.and_then(|text| text.strip_prefix(key))
        .and_then(|text| text.strip_prefix('='))
        .ok_or_else(|| malformed(line, &format!("expected `{key}=...` in this position")))
}

fn hash_field(part: Option<&str>, key: &str, line: usize) -> Result<u64, GoldenError> {
    let text = field(part, key, line)?;
    // Exactly what `{:016x}` prints, so a golden line is the runner's output verbatim.
    let canonical =
        text.len() == 16 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !canonical {
        return Err(malformed(
            line,
            &format!("`{key}` must be 16 lowercase hex digits"),
        ));
    }
    Ok(u64::from_str_radix(text, 16).expect("16 lowercase hex digits always fit in u64"))
}

fn frames_field(part: Option<&str>, line: usize) -> Result<u64, GoldenError> {
    let text = field(part, "frames", line)?;
    // Exactly what `{}` prints: ASCII digits only, no sign, no leading zero (`0` alone is fine).
    let canonical = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text.len() == 1 || !text.starts_with('0'));
    if !canonical {
        return Err(malformed(line, "`frames` must be a plain decimal number"));
    }
    text.parse().map_err(|_| {
        malformed(
            line,
            "`frames` must be a plain decimal number that fits in u64",
        )
    })
}

fn parse_line(text: &str, line: usize) -> Result<RunReport, GoldenError> {
    let mut parts = text.split_whitespace();
    let frames = frames_field(parts.next(), line)?;
    let top_hash = hash_field(parts.next(), "top", line)?;
    let bottom_hash = hash_field(parts.next(), "bottom", line)?;
    // An optional fourth field: the audio hash. Lines without it do not check the audio.
    let audio_hash = match parts.next() {
        Some(part) => Some(hash_field(Some(part), "audio", line)?),
        None => None,
    };
    if parts.next().is_some() {
        return Err(malformed(line, "unexpected text after `audio`"));
    }
    Ok(RunReport {
        frames,
        top_hash,
        bottom_hash,
        audio_hash,
    })
}

/// Parses golden-file text into one [`RunReport`] per data line.
///
/// Lines are numbered from 1 over the raw text. A comment (`#`) or whitespace-only line is
/// skipped; any other line must be `frames=N top=<hex> bottom=<hex>`, and `N` may appear once.
pub fn parse_golden(text: &str) -> Result<Vec<RunReport>, GoldenError> {
    let mut reports: Vec<RunReport> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let report = parse_line(trimmed, line)?;
        if reports.iter().any(|seen| seen.frames == report.frames) {
            return Err(GoldenError::DuplicateFrames {
                line,
                frames: report.frames,
            });
        }
        reports.push(report);
    }
    Ok(reports)
}

/// Checks `actual` against the golden line with the same frame count.
pub fn check_frame_hash(golden: &[RunReport], actual: &RunReport) -> Result<(), GoldenError> {
    let expected = golden
        .iter()
        .find(|line| line.frames == actual.frames)
        .ok_or(GoldenError::MissingFrames {
            frames: actual.frames,
        })?;
    // A golden line without `audio=` does not check the audio; one with it must match.
    let audio_matches = expected
        .audio_hash
        .is_none_or(|audio| actual.audio_hash == Some(audio));
    let screens_match = (expected.frames, expected.top_hash, expected.bottom_hash)
        == (actual.frames, actual.top_hash, actual.bottom_hash);
    if screens_match && audio_matches {
        Ok(())
    } else {
        Err(GoldenError::Mismatch {
            expected: *expected,
            actual: *actual,
        })
    }
}

/// Path of a ROM's golden file, `tests/golden/<rom>.txt`.
///
/// For in-repo tests only: it resolves into this source checkout.
pub fn golden_path(rom: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .join(format!("{rom}.txt"))
}

const README: &str = "tests/golden/README.md";

fn load_rom(rom: &str) -> Vec<u8> {
    let path = testrom_path(rom);
    load_rom_file(&path).unwrap_or_else(|error| {
        panic!("{error}\nbuild the test ROMs first: ./scripts/build_testroms.sh")
    })
}

/// The panic text for a golden that is bad or does not match; pure so a test can pin it.
fn failure_message(rom: &str, frames: u64, path: &Path, error: &GoldenError) -> String {
    let headline = match error {
        GoldenError::Unreadable { .. } => format!("cannot read golden {}", path.display()),
        GoldenError::Mismatch { .. } => format!("golden mismatch for {rom} ({})", path.display()),
        GoldenError::MissingFrames { .. } => {
            format!("missing golden line in {}", path.display())
        }
        GoldenError::Malformed { .. } | GoldenError::DuplicateFrames { .. } => {
            format!("bad golden {}", path.display())
        }
    };
    format!(
        "{headline}:\n  {error}\nreproduce: cargo run -p ninge-runner -- testroms/build/{rom}.nds --frames {frames}\nreview flow: {README}"
    )
}

/// Asserts that running the built test ROM `rom` for `frames` frames matches its golden file.
///
/// Reads the golden first, then runs the ROM. Never writes the golden file: a new or changed
/// golden is pasted in by hand and reviewed (`tests/golden/README.md`).
///
/// # Panics
///
/// On an unreadable or malformed golden, a missing ROM, a frame count the golden has no line
/// for, or a mismatch. Every message carries the command that reproduces the run and the
/// `tests/golden/README.md` pointer; the expected and actual lines appear only for a mismatch.
pub fn assert_frame_hash(rom: &str, frames: u64) {
    // The audio is hashed too; a golden line checks it only when it has an `audio=` field.
    assert_golden(rom, frames, |bytes| {
        run_with_audio(bytes, frames).map(|(report, _)| report)
    });
}

/// [`assert_frame_hash`] with the host's input for each frame taken from `log`, for a ROM whose
/// result depends on it (a touch press, a held key).
///
/// # Panics
///
/// As [`assert_frame_hash`].
pub fn assert_frame_hash_with_input(rom: &str, frames: u64, log: &InputLog) {
    assert_golden(rom, frames, |bytes| {
        let mut machine = booted(bytes);
        let hashes = step_frame_hashes(&mut machine, frames, 0, log);
        let (top_hash, bottom_hash) = hashes.last().copied().unwrap_or_default();
        Ok(RunReport {
            frames: machine.frame_count(),
            top_hash,
            bottom_hash,
            audio_hash: None,
        })
    });
}

/// [`assert_frame_hash`] for a ROM that talks to its save chip: the machine is given a blank chip
/// of `kind` first ([`run_with_blank_save`]). The chip's image is synthesized, never a file.
///
/// # Panics
///
/// As [`assert_frame_hash`].
pub fn assert_frame_hash_with_save(rom: &str, frames: u64, kind: SaveType) {
    assert_golden(rom, frames, |bytes| {
        run_with_blank_save(bytes, frames, kind)
    });
}

fn assert_golden(
    rom: &str,
    frames: u64,
    run_rom: impl FnOnce(Vec<u8>) -> Result<RunReport, RunnerError>,
) {
    let path = golden_path(rom);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        let error = GoldenError::Unreadable {
            reason: error.to_string(),
        };
        panic!("{}", failure_message(rom, frames, &path, &error))
    });
    let golden = parse_golden(&text)
        .unwrap_or_else(|error| panic!("{}", failure_message(rom, frames, &path, &error)));
    let actual = run_rom(load_rom(rom)).expect("the golden ROM has a valid header");
    if let Err(error) = check_frame_hash(&golden, &actual) {
        panic!("{}", failure_message(rom, frames, &path, &error));
    }
}

/// Runs `rom` for `frames` frames and returns the `(top, bottom)` hash after each frame.
pub fn run_frame_hashes(rom: Vec<u8>, frames: u64) -> Vec<(u64, u64)> {
    step_frame_hashes(&mut booted(rom), frames, 0, &InputLog::default())
}

/// Index of the first position where `a` and `b` differ; the shorter length if one is a prefix.
pub fn first_divergence(a: &[(u64, u64)], b: &[(u64, u64)]) -> Option<usize> {
    let common = a.len().min(b.len());
    (0..common)
        .find(|&index| a[index] != b[index])
        .or((a.len() != b.len()).then_some(common))
}

/// Hashes both screens after every frame of `machine`'s next `frames` frames, which are the
/// frames of `log` from index `first_frame`.
fn step_frame_hashes(
    machine: &mut Machine,
    frames: u64,
    first_frame: u64,
    log: &InputLog,
) -> Vec<(u64, u64)> {
    (first_frame..first_frame + frames)
        .map(|frame| {
            machine.run_frame(log.input_for(frame));
            (
                frame_hash(machine.frame(Screen::Top)),
                frame_hash(machine.frame(Screen::Bottom)),
            )
        })
        .collect()
}

/// A fresh machine that booted `rom`.
fn booted(rom: Vec<u8>) -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("the ROM has a valid header");
    machine
}

/// Index of the first byte where `a` and `b` differ; the shorter length if one is a prefix.
fn first_byte_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let common = a.len().min(b.len());
    (0..common)
        .find(|&index| a[index] != b[index])
        .or((a.len() != b.len()).then_some(common))
}

/// Asserts that two fresh machines running `rom` for `frames` frames hash identically and end
/// in the same savestate bytes.
///
/// Until display mode 2 lands (#20) every frame is black, so the savestate bytes are the
/// check that can actually see a difference.
///
/// # Panics
///
/// On `frames == 0` (nothing would be compared), a missing ROM, or a difference; the message
/// names the first diverging frame, or the first differing savestate byte.
pub fn assert_deterministic(rom: &str, frames: u64) {
    assert_deterministic_with_input(rom, frames, &InputLog::default());
}

/// [`assert_deterministic`] with the host's input for each frame taken from `log`.
///
/// # Panics
///
/// As [`assert_deterministic`].
pub fn assert_deterministic_with_input(rom: &str, frames: u64, log: &InputLog) {
    assert!(
        frames > 0,
        "assert_deterministic needs at least one frame to compare; got frames=0 for {rom}"
    );
    let bytes = load_rom(rom);
    let mut first = booted(bytes.clone());
    let mut second = booted(bytes);
    let first_hashes = step_frame_hashes(&mut first, frames, 0, log);
    let second_hashes = step_frame_hashes(&mut second, frames, 0, log);
    if let Some(index) = first_divergence(&first_hashes, &second_hashes) {
        panic!(
            "{rom} is not deterministic: the two runs first differ at frame {}",
            index + 1
        );
    }
    if let Some(offset) = first_byte_difference(&first.save_state(), &second.save_state()) {
        panic!(
            "{rom} is not deterministic: the two runs end in different savestates \
             (first differing byte at offset {offset})"
        );
    }
}

/// Asserts that saving after `frames_before` frames, loading into a fresh machine that was
/// handed the same ROM, and running `frames_after` more frames matches an uninterrupted run of
/// `frames_before + frames_after` frames, in every frame hash and in the final savestate bytes.
///
/// # Panics
///
/// On `frames_after == 0` (nothing would be compared), a missing ROM, a refused savestate, or
/// a difference; the message names the first diverging frame, or the first differing
/// savestate byte.
pub fn assert_savestate_round_trip(rom: &str, frames_before: u64, frames_after: u64) {
    assert_savestate_round_trip_with_input(rom, frames_before, frames_after, &InputLog::default());
}

/// [`assert_savestate_round_trip`] with the host's input for each frame taken from `log`; the
/// resumed run takes the frames after `frames_before` from the same log.
///
/// # Panics
///
/// As [`assert_savestate_round_trip`].
pub fn assert_savestate_round_trip_with_input(
    rom: &str,
    frames_before: u64,
    frames_after: u64,
    log: &InputLog,
) {
    assert!(
        frames_after > 0,
        "assert_savestate_round_trip needs at least one frame after the load; got \
         frames_after=0 for {rom}"
    );
    let bytes = load_rom(rom);

    let mut uninterrupted = booted(bytes.clone());
    let expected_hashes =
        step_frame_hashes(&mut uninterrupted, frames_before + frames_after, 0, log);

    let mut saver = booted(bytes.clone());
    step_frame_hashes(&mut saver, frames_before, 0, log);
    let mut resumed = booted(bytes);
    resumed
        .load_state(&saver.save_state())
        .unwrap_or_else(|error| panic!("{rom}: the savestate was refused: {error}"));
    let resumed_hashes = step_frame_hashes(&mut resumed, frames_after, frames_before, log);

    let after_load = &expected_hashes[usize::try_from(frames_before).expect("frame count fits")..];
    if let Some(index) = first_divergence(after_load, &resumed_hashes) {
        panic!(
            "{rom}: the run resumed from a savestate taken after {frames_before} frames first \
             differs from the uninterrupted run at frame {}",
            frames_before + index as u64 + 1
        );
    }
    if let Some(offset) = first_byte_difference(&uninterrupted.save_state(), &resumed.save_state())
    {
        panic!(
            "{rom}: the run resumed from a savestate taken after {frames_before} frames ends in \
             a different savestate than the uninterrupted run (first differing byte at offset \
             {offset}, after frame {})",
            frames_before + frames_after
        );
    }
}

/// The audio of `machine`'s next `frames` frames, in order.
fn frames_audio(machine: &mut Machine, frames: u64) -> Vec<i16> {
    let mut samples = Vec::new();
    (0..frames).for_each(|_| {
        machine.run_frame(Input::default());
        samples.extend_from_slice(machine.audio());
    });
    samples
}

/// The audio of the built test ROM `rom` over `frames` frames, interleaved stereo `i16`.
///
/// # Panics
///
/// On a missing ROM.
pub fn rom_audio(rom: &str, frames: u64) -> Vec<i16> {
    run_with_audio(load_rom(rom), frames)
        .expect("the ROM has a valid header")
        .1
}

/// The first offsets, one for each of `N` channels, for which `matches` holds: each channel's first
/// sounding output sample is `base + 0` or `base + 1` for a `base` in `bases`. Channels a ROM starts
/// with a few instructions between them begin within one output sample of each other, so which of
/// two consecutive output samples each is the first to carry its sound depends on where the writes
/// fall on the 2048-cycle grid; a test states the model and lets this find the phase.
pub fn find_offsets<const N: usize>(
    bases: std::ops::Range<i64>,
    matches: impl Fn([i64; N]) -> bool,
) -> Option<[i64; N]> {
    bases.into_iter().find_map(|base| {
        (0..1u32 << N).find_map(|mask| {
            let offsets = std::array::from_fn(|channel| base + i64::from((mask >> channel) & 1));
            matches(offsets).then_some(offsets)
        })
    })
}

/// Index of the first sample where `a` and `b` differ; the shorter length if one is a prefix.
pub fn first_sample_difference(a: &[i16], b: &[i16]) -> Option<usize> {
    let common = a.len().min(b.len());
    (0..common)
        .find(|&index| a[index] != b[index])
        .or((a.len() != b.len()).then_some(common))
}

/// Asserts that running `rom` twice for `frames` frames makes the same audio, sample for sample.
///
/// # Panics
///
/// On `frames == 0`, a missing ROM, or a difference; the message names the first differing
/// sample.
pub fn assert_audio_deterministic(rom: &str, frames: u64) {
    assert!(
        frames > 0,
        "assert_audio_deterministic needs at least one frame to compare; got frames=0 for {rom}"
    );
    let bytes = load_rom(rom);
    let first = frames_audio(&mut booted(bytes.clone()), frames);
    let second = frames_audio(&mut booted(bytes), frames);
    if let Some(index) = first_sample_difference(&first, &second) {
        panic!("{rom} makes different audio in two runs: the first difference is sample {index}");
    }
}

/// Asserts that a machine resumed from a savestate taken after `frames_before` frames makes, over
/// the next `frames_after` frames, the same audio as the uninterrupted run (the decoder, the noise
/// register and the playback positions are all state).
///
/// # Panics
///
/// On `frames_after == 0`, a missing ROM, a refused savestate, or a difference; the message
/// names the first differing sample of the resumed part.
pub fn assert_audio_round_trip(rom: &str, frames_before: u64, frames_after: u64) {
    assert!(
        frames_after > 0,
        "assert_audio_round_trip needs at least one frame after the load; got frames_after=0 \
         for {rom}"
    );
    let bytes = load_rom(rom);
    let mut uninterrupted = booted(bytes.clone());
    frames_audio(&mut uninterrupted, frames_before);
    let expected = frames_audio(&mut uninterrupted, frames_after);

    let mut saver = booted(bytes.clone());
    frames_audio(&mut saver, frames_before);
    let mut resumed = booted(bytes);
    resumed
        .load_state(&saver.save_state())
        .unwrap_or_else(|error| panic!("{rom}: the savestate was refused: {error}"));
    let after_load = frames_audio(&mut resumed, frames_after);
    if let Some(index) = first_sample_difference(&expected, &after_load) {
        panic!(
            "{rom}: the audio resumed from a savestate taken after {frames_before} frames first \
             differs from the uninterrupted run at sample {index} of the resumed part"
        );
    }
}

#[cfg(test)]
mod tests;
