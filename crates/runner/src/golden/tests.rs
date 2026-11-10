// SPDX-License-Identifier: Apache-2.0
//! Tests of `golden`.

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
    let text = "# header\n\nframes=1 top=0000000000000001 bottom=0000000000000002\n  \n# tail\n";
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
    let report = crate::run(rom.clone(), 3).unwrap();
    let hashes = run_frame_hashes(rom, 3);
    assert_eq!(hashes.last(), Some(&(report.top_hash, report.bottom_hash)));
}

#[test]
#[should_panic(expected = "nonexistent_rom_for_test")]
fn given_nonexistent_rom_when_asserting_deterministic_then_panic_names_the_rom() {
    assert_deterministic("nonexistent_rom_for_test", 1);
}

#[test]
#[should_panic(expected = "nonexistent_rom_for_test")]
fn given_nonexistent_rom_when_asserting_savestate_round_trip_then_panic_names_the_rom() {
    assert_savestate_round_trip("nonexistent_rom_for_test", 1, 1);
}

#[test]
#[should_panic(expected = "at least one frame after the load")]
fn given_zero_frames_after_when_asserting_savestate_round_trip_then_it_is_refused() {
    assert_savestate_round_trip("nonexistent_rom_for_test", 1, 0);
}

#[test]
fn given_equal_byte_slices_when_compared_then_no_difference() {
    assert_eq!(first_byte_difference(&[1, 2, 3], &[1, 2, 3]), None);
}

#[test]
fn given_difference_at_byte_two_when_compared_then_offset_is_two() {
    assert_eq!(first_byte_difference(&[1, 2, 3], &[1, 2, 4]), Some(2));
}

#[test]
fn given_shorter_byte_prefix_when_compared_then_offset_is_shorter_length() {
    assert_eq!(first_byte_difference(&[1, 2], &[1, 2, 3, 4]), Some(2));
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
fn given_missing_frames_when_building_failure_message_then_it_has_repro_and_no_mismatch_wording() {
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
