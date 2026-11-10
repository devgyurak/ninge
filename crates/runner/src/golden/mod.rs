// SPDX-License-Identifier: Apache-2.0
//! Golden-frame helpers: compare a ROM's screen hashes with reviewed values.
//!
//! Decided in #68. A golden file is `tests/golden/<rom>.txt`; each data line has the form of
//! a [`RunReport`]'s `Display` output, `frames=N top=<016x> bottom=<016x>`. `#` starts a
//! comment line and blank lines are allowed. See `tests/golden/README.md`.

use std::fmt;
use std::path::{Path, PathBuf};

use ninge_core::{Machine, Screen};

use crate::{RunReport, frame_hash, load_rom_file, run, testrom_path};

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
    if parts.next().is_some() {
        return Err(malformed(line, "unexpected text after `bottom`"));
    }
    Ok(RunReport {
        frames,
        top_hash,
        bottom_hash,
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
    if expected == actual {
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
    let path = golden_path(rom);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        let error = GoldenError::Unreadable {
            reason: error.to_string(),
        };
        panic!("{}", failure_message(rom, frames, &path, &error))
    });
    let golden = parse_golden(&text)
        .unwrap_or_else(|error| panic!("{}", failure_message(rom, frames, &path, &error)));
    let actual = run(load_rom(rom), frames).expect("the golden ROM has a valid header");
    if let Err(error) = check_frame_hash(&golden, &actual) {
        panic!("{}", failure_message(rom, frames, &path, &error));
    }
}

/// Runs `rom` for `frames` frames and returns the `(top, bottom)` hash after each frame.
pub fn run_frame_hashes(rom: Vec<u8>, frames: u64) -> Vec<(u64, u64)> {
    step_frame_hashes(&mut booted(rom), frames)
}

/// Index of the first position where `a` and `b` differ; the shorter length if one is a prefix.
pub fn first_divergence(a: &[(u64, u64)], b: &[(u64, u64)]) -> Option<usize> {
    let common = a.len().min(b.len());
    (0..common)
        .find(|&index| a[index] != b[index])
        .or((a.len() != b.len()).then_some(common))
}

/// Hashes both screens after every frame of `machine`'s next `frames` frames.
fn step_frame_hashes(machine: &mut Machine, frames: u64) -> Vec<(u64, u64)> {
    (0..frames)
        .map(|_| {
            machine.run_frame();
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
// TODO(phase1): take an input log once the core has an input API (#61).
pub fn assert_deterministic(rom: &str, frames: u64) {
    assert!(
        frames > 0,
        "assert_deterministic needs at least one frame to compare; got frames=0 for {rom}"
    );
    let bytes = load_rom(rom);
    let mut first = booted(bytes.clone());
    let mut second = booted(bytes);
    let first_hashes = step_frame_hashes(&mut first, frames);
    let second_hashes = step_frame_hashes(&mut second, frames);
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
    assert!(
        frames_after > 0,
        "assert_savestate_round_trip needs at least one frame after the load; got \
         frames_after=0 for {rom}"
    );
    let bytes = load_rom(rom);

    let mut uninterrupted = booted(bytes.clone());
    let expected_hashes = step_frame_hashes(&mut uninterrupted, frames_before + frames_after);

    let mut saver = booted(bytes.clone());
    step_frame_hashes(&mut saver, frames_before);
    let mut resumed = booted(bytes);
    resumed
        .load_state(&saver.save_state())
        .unwrap_or_else(|error| panic!("{rom}: the savestate was refused: {error}"));
    let resumed_hashes = step_frame_hashes(&mut resumed, frames_after);

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

#[cfg(test)]
mod tests;
