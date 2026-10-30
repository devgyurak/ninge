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
    let actual = run(load_rom(rom), frames);
    if let Err(error) = check_frame_hash(&golden, &actual) {
        panic!("{}", failure_message(rom, frames, &path, &error));
    }
}

/// Runs `rom` for `frames` frames and returns the `(top, bottom)` hash after each frame.
pub fn run_frame_hashes(rom: Vec<u8>, frames: u64) -> Vec<(u64, u64)> {
    let mut machine = Machine::new();
    machine.load_rom(rom);
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

/// Index of the first position where `a` and `b` differ; the shorter length if one is a prefix.
pub fn first_divergence(a: &[(u64, u64)], b: &[(u64, u64)]) -> Option<usize> {
    let common = a.len().min(b.len());
    (0..common)
        .find(|&index| a[index] != b[index])
        .or((a.len() != b.len()).then_some(common))
}

/// Asserts that two fresh machines running `rom` for `frames` frames hash identically.
///
/// TODO(phase1): take an input log once the core has an input API (#61).
/// Savestate-byte equality is a separate check that belongs with savestates (#19).
///
/// # Panics
///
/// On `frames == 0` (nothing would be compared), a missing ROM, or a difference; the message
/// names the first diverging frame.
pub fn assert_deterministic(rom: &str, frames: u64) {
    assert!(
        frames > 0,
        "assert_deterministic needs at least one frame to compare; got frames=0 for {rom}"
    );
    let bytes = load_rom(rom);
    let first = run_frame_hashes(bytes.clone(), frames);
    let second = run_frame_hashes(bytes, frames);
    if let Some(index) = first_divergence(&first, &second) {
        panic!(
            "{rom} is not deterministic: the two runs first differ at frame {}",
            index + 1
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(frames: u64, top_hash: u64, bottom_hash: u64) -> RunReport {
        RunReport {
            frames,
            top_hash,
            bottom_hash,
        }
    }

    /// Parses each malformed shape and returns the ones that were not rejected as `Malformed`.
    fn malformed_shapes_not_rejected() -> Vec<&'static str> {
        let shapes = [
            "frames=3 top=0000000000000001",
            "frames=3 bottom=0000000000000001 top=0000000000000002",
            "frames=3 top=zzzzzzzzzzzzzzzz bottom=0000000000000001",
            "frames=x top=0000000000000001 bottom=0000000000000002",
            "top=0000000000000001 bottom=0000000000000002",
            "garbage",
            "frames=3 top=0000000000000001 bottom=0000000000000002 extra",
            "frames=3 top=0x00000000000001 bottom=0000000000000002",
            "frames=3 top=+000000000000001 bottom=0000000000000002",
            "frames=3 top=00000000000000AB bottom=0000000000000002",
            "frames=3 top=000000000000001 bottom=0000000000000002",
            "frames=3 top=00000000000000001 bottom=0000000000000002",
            "frames=3 top=0000000000000001 bottom=000000000000001",
            "frames=+3 top=0000000000000001 bottom=0000000000000002",
            "frames=03 top=0000000000000001 bottom=0000000000000002",
            "frames=99999999999999999999 top=0000000000000001 bottom=0000000000000002",
        ];
        shapes
            .into_iter()
            .filter(|s| !matches!(parse_golden(s), Err(GoldenError::Malformed { line: 1, .. })))
            .collect()
    }

    #[test]
    fn given_valid_data_line_when_parsed_then_report_has_its_fields() {
        let parsed = parse_golden("frames=3 top=00000000000000ab bottom=00000000000000cd\n");
        assert_eq!(parsed, Ok(vec![report(3, 0xab, 0xcd)]));
    }

    #[test]
    fn given_comment_and_blank_lines_when_parsed_then_only_data_lines_remain() {
        let text =
            "# header\n\nframes=1 top=0000000000000001 bottom=0000000000000002\n  \n# tail\n";
        assert_eq!(parse_golden(text), Ok(vec![report(1, 1, 2)]));
    }

    #[test]
    fn given_bad_line_after_comment_when_parsed_then_error_carries_one_based_line() {
        let err = parse_golden("# c\nnot a line\n").unwrap_err();
        assert!(
            matches!(err, GoldenError::Malformed { line: 2, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn given_duplicate_frames_when_parsed_then_error_names_second_line_and_frames() {
        let text = "frames=5 top=0000000000000001 bottom=0000000000000002\n\
                    # c\n\
                    frames=5 top=0000000000000003 bottom=0000000000000004\n";
        assert_eq!(
            parse_golden(text),
            Err(GoldenError::DuplicateFrames { line: 3, frames: 5 })
        );
    }

    #[test]
    fn given_malformed_shapes_when_parsed_then_every_one_is_rejected_as_malformed() {
        assert_eq!(malformed_shapes_not_rejected(), Vec::<&str>::new());
    }

    #[test]
    fn given_matching_line_when_checked_then_result_is_unit_ok() {
        let golden = [report(2, 7, 8), report(3, 9, 10)];
        assert_eq!(check_frame_hash(&golden, &report(3, 9, 10)), Ok(()));
    }

    #[test]
    fn given_no_line_for_frame_count_when_checked_then_missing_frames_names_count() {
        let golden = [report(2, 7, 8)];
        assert_eq!(
            check_frame_hash(&golden, &report(4, 7, 8)),
            Err(GoldenError::MissingFrames { frames: 4 })
        );
    }

    #[test]
    fn given_only_top_differs_when_checked_then_mismatch_carries_both_reports() {
        let golden = [report(2, 7, 8)];
        let actual = report(2, 99, 8);
        assert_eq!(
            check_frame_hash(&golden, &actual),
            Err(GoldenError::Mismatch {
                expected: report(2, 7, 8),
                actual
            })
        );
    }

    #[test]
    fn given_only_bottom_differs_when_checked_then_mismatch_carries_both_reports() {
        let golden = [report(2, 7, 8)];
        let actual = report(2, 7, 99);
        assert_eq!(
            check_frame_hash(&golden, &actual),
            Err(GoldenError::Mismatch {
                expected: report(2, 7, 8),
                actual
            })
        );
    }

    #[test]
    fn given_equal_sequences_when_compared_then_no_divergence() {
        assert_eq!(first_divergence(&[(1, 2), (3, 4)], &[(1, 2), (3, 4)]), None);
    }

    #[test]
    fn given_difference_at_index_one_when_compared_then_divergence_is_one() {
        assert_eq!(
            first_divergence(&[(1, 2), (3, 4)], &[(1, 2), (3, 5)]),
            Some(1)
        );
    }

    #[test]
    fn given_shorter_prefix_when_compared_then_divergence_is_shorter_length() {
        assert_eq!(
            first_divergence(&[(1, 2)], &[(1, 2), (3, 4), (5, 6)]),
            Some(1)
        );
    }

    #[test]
    fn given_both_empty_when_compared_then_no_divergence() {
        assert_eq!(first_divergence(&[], &[]), None);
    }

    #[test]
    fn given_malformed_error_when_displayed_then_message_has_line_and_reason() {
        let msg = GoldenError::Malformed {
            line: 7,
            reason: "bad-reason-text".to_string(),
        }
        .to_string();
        assert!(
            msg.contains('7') && msg.contains("bad-reason-text"),
            "{msg}"
        );
    }

    #[test]
    fn given_duplicate_error_when_displayed_then_message_has_line_and_frames() {
        let msg = GoldenError::DuplicateFrames {
            line: 12,
            frames: 345,
        }
        .to_string();
        assert!(msg.contains("12") && msg.contains("345"), "{msg}");
    }

    #[test]
    fn given_missing_error_when_displayed_then_message_has_frames() {
        let msg = GoldenError::MissingFrames { frames: 678 }.to_string();
        assert!(msg.contains("678"), "{msg}");
    }

    #[test]
    fn given_mismatch_error_when_displayed_then_message_has_expected_and_actual_lines() {
        let expected = report(2, 7, 8);
        let actual = report(2, 9, 8);
        let msg = GoldenError::Mismatch { expected, actual }.to_string();
        assert!(
            msg.contains(&expected.to_string()) && msg.contains(&actual.to_string()),
            "{msg}"
        );
    }

    #[test]
    fn given_rom_name_when_golden_path_built_then_it_ends_with_tests_golden_rom_txt() {
        let path = golden_path("smoke");
        assert!(path.ends_with("tests/golden/smoke.txt"), "{path:?}");
    }

    // Pins the order: the golden is read before any ROM is loaded, so a missing golden is
    // reported as such even when the ROM is missing too.
    #[test]
    #[should_panic(expected = "cannot read golden")]
    fn given_rom_with_no_golden_file_when_asserting_frame_hash_then_panic_is_about_the_golden_before_any_rom_load()
     {
        assert_frame_hash("nonexistent_rom_for_test", 1);
    }

    #[test]
    fn given_smoke_rom_when_hashing_frames_then_one_entry_per_frame() {
        let rom = crate::load_rom_file(&crate::testrom_path("smoke")).unwrap();
        assert_eq!(run_frame_hashes(rom, 3).len(), 3);
    }

    #[test]
    fn given_smoke_rom_when_hashing_frames_then_last_entry_equals_run_report_hashes() {
        let rom = crate::load_rom_file(&crate::testrom_path("smoke")).unwrap();
        let report = crate::run(rom.clone(), 3);
        let hashes = run_frame_hashes(rom, 3);
        assert_eq!(hashes.last(), Some(&(report.top_hash, report.bottom_hash)));
    }

    #[test]
    #[should_panic(expected = "nonexistent_rom_for_test")]
    fn given_nonexistent_rom_when_asserting_deterministic_then_panic_names_the_rom() {
        assert_deterministic("nonexistent_rom_for_test", 1);
    }

    fn missing_substrings<'a>(message: &str, wanted: &[&'a str]) -> Vec<&'a str> {
        wanted
            .iter()
            .copied()
            .filter(|part| !message.contains(part))
            .collect()
    }

    #[test]
    fn given_frames_zero_when_parsed_then_report_has_zero_frames() {
        let parsed = parse_golden("frames=0 top=0000000000000001 bottom=0000000000000002");
        assert_eq!(parsed, Ok(vec![report(0, 1, 2)]));
    }

    #[test]
    fn given_mismatch_when_building_failure_message_then_it_has_both_lines_repro_and_readme() {
        let error = GoldenError::Mismatch {
            expected: report(3, 1, 2),
            actual: report(3, 1, 9),
        };
        let message = failure_message("smoke", 3, Path::new("g/smoke.txt"), &error);
        let wanted = [
            "golden mismatch",
            "frames=3 top=0000000000000001 bottom=0000000000000002",
            "frames=3 top=0000000000000001 bottom=0000000000000009",
            "cargo run -p ninge-runner -- testroms/build/smoke.nds --frames 3",
            "tests/golden/README.md",
        ];
        assert_eq!(missing_substrings(&message, &wanted), Vec::<&str>::new());
    }

    #[test]
    fn given_missing_frames_when_building_failure_message_then_it_has_repro_and_no_mismatch_wording()
     {
        let error = GoldenError::MissingFrames { frames: 7 };
        let message = failure_message("smoke", 7, Path::new("g/smoke.txt"), &error);
        let wanted = [
            "missing golden line in g/smoke.txt",
            "frames=7",
            "cargo run -p ninge-runner -- testroms/build/smoke.nds --frames 7",
            "tests/golden/README.md",
        ];
        assert_eq!(
            (
                missing_substrings(&message, &wanted),
                message.matches("frames=7").count(),
                message.contains("mismatch")
            ),
            (Vec::<&str>::new(), 1, false)
        );
    }

    #[test]
    fn given_malformed_golden_when_building_failure_message_then_it_names_file_line_and_readme() {
        let error = GoldenError::Malformed {
            line: 4,
            reason: "bad".to_owned(),
        };
        let message = failure_message("smoke", 3, Path::new("g/smoke.txt"), &error);
        let wanted = [
            "bad golden",
            "g/smoke.txt",
            "line 4",
            "tests/golden/README.md",
        ];
        assert_eq!(missing_substrings(&message, &wanted), Vec::<&str>::new());
    }

    #[test]
    #[should_panic(expected = "at least one frame")]
    fn given_zero_frames_when_asserting_deterministic_then_panic_says_nothing_would_be_compared() {
        assert_deterministic("nonexistent_rom_for_test", 0);
    }

    #[test]
    fn given_unreadable_golden_when_building_failure_message_then_it_has_path_repro_and_readme() {
        let error = GoldenError::Unreadable {
            reason: "No such file".to_owned(),
        };
        let message = failure_message("smoke", 3, Path::new("g/smoke.txt"), &error);
        let wanted = [
            "cannot read golden g/smoke.txt",
            "No such file",
            "cargo run -p ninge-runner -- testroms/build/smoke.nds --frames 3",
            "tests/golden/README.md",
        ];
        assert_eq!(missing_substrings(&message, &wanted), Vec::<&str>::new());
    }

    #[test]
    fn given_unreadable_error_when_displayed_then_message_is_the_io_reason() {
        let error = GoldenError::Unreadable {
            reason: "No such file".to_owned(),
        };
        assert_eq!(error.to_string(), "No such file");
    }
}
