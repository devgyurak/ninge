// SPDX-License-Identifier: Apache-2.0
//! The input log format (`docs/contributing/input-log.md`).

use super::*;
use proptest::prelude::*;

// `cbf29ce484222325` below is FNV-1a 64 of no bytes: the save hash of a machine with no save
// memory (pinned by the empty-save test).
const HEADER: &str = "ninge-input 1 rom=1a2b save=flash-512k:00112233aabbccdd\n";

fn header() -> LogHeader {
    LogHeader {
        rom_crc: 0x1a2b,
        save_type: SaveType::Flash512K,
        save_hash: 0x0011_2233_aabb_ccdd,
    }
}

/// The log text of `body` under the standard header.
fn log(body: &str) -> String {
    format!("{HEADER}{body}")
}

/// The error kind of reading `text`, or `None` when it reads.
fn kind_of(text: &str) -> Option<LogErrorKind> {
    parse(text).err().map(|e| e.kind)
}

/// The line number of the error from reading `text`, or `None` when it reads.
fn line_of(text: &str) -> Option<usize> {
    parse(text).err().map(|e| e.line)
}

/// The cases whose text does not give the expected error kind, as `(text, got)`.
fn kind_failures(cases: &[(&str, LogErrorKind)]) -> Vec<(String, Option<LogErrorKind>)> {
    cases
        .iter()
        .map(|(body, want)| (log(body), want))
        .filter_map(|(text, want)| {
            let got = kind_of(&text);
            (got.as_ref() != Some(want)).then_some((text, got))
        })
        .collect()
}

fn bad_value(key: &'static str, value: &str) -> LogErrorKind {
    LogErrorKind::BadValue {
        key,
        value: value.to_string(),
    }
}

fn frame(body: &str) -> Input {
    parse(&log(body)).unwrap().frames[0]
}

// ---- The header ----

#[test]
fn given_a_header_when_encoded_then_it_is_the_documented_line() {
    assert_eq!(encode_header(&header()), HEADER);
}

#[test]
fn given_an_encoded_header_when_parsed_then_it_is_the_same_header() {
    assert_eq!(parse(HEADER).unwrap().header, header());
}

#[test]
fn given_a_header_when_the_rom_crc_differs_then_the_check_names_the_rom() {
    assert_eq!(
        header().check(0x0001, SaveType::Flash512K, 0x0011_2233_aabb_ccdd),
        Err(MismatchError::Rom {
            log: 0x1a2b,
            actual: 1
        })
    );
}

#[test]
fn given_a_header_when_the_save_type_differs_then_the_check_names_the_save_type() {
    assert_eq!(
        header().check(0x1a2b, SaveType::None, 0x0011_2233_aabb_ccdd),
        Err(MismatchError::SaveType {
            log: SaveType::Flash512K,
            actual: SaveType::None
        })
    );
}

#[test]
fn given_a_header_when_the_save_hash_differs_then_the_check_names_the_hash() {
    assert_eq!(
        header().check(0x1a2b, SaveType::Flash512K, 7),
        Err(MismatchError::SaveHash {
            log: 0x0011_2233_aabb_ccdd,
            actual: 7
        })
    );
}

#[test]
fn given_a_header_when_everything_matches_then_the_check_passes() {
    assert_eq!(
        header().check(0x1a2b, SaveType::Flash512K, 0x0011_2233_aabb_ccdd),
        Ok(())
    );
}

#[test]
fn given_empty_save_memory_when_hashed_then_it_is_the_fnv_offset_basis() {
    assert_eq!(save_hash(&[]), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn given_one_save_byte_when_hashed_then_it_is_the_fnv1a_step() {
    assert_eq!(save_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
}

#[test]
fn given_bad_headers_when_parsed_then_each_is_refused_for_its_reason() {
    let cases = [
        (
            "ninge-input 2 rom=1a2b save=none:cbf29ce484222325\n",
            LogErrorKind::UnsupportedVersion {
                found: 2,
                expected: 1,
            },
        ),
        (
            "ninge-input x rom=1a2b save=none:cbf29ce484222325\n",
            LogErrorKind::BadVersion,
        ),
        ("ninge-input\n", LogErrorKind::BadVersion),
        (
            "ninge-input 1 save=none:cbf29ce484222325\n",
            LogErrorKind::MissingHeaderKey("rom"),
        ),
        (
            "ninge-input 1 rom=1a2b\n",
            LogErrorKind::MissingHeaderKey("save"),
        ),
        (
            "ninge-input 1 rom=1a2b save=none:cbf29ce484222325 extra=1\n",
            LogErrorKind::UnknownKey("extra".to_string()),
        ),
        (
            "ninge-input 1 rom=1a2b rom=1a2b save=none:cbf29ce484222325\n",
            LogErrorKind::DuplicateKey("rom".to_string()),
        ),
        (
            "ninge-input 1 rom=1a2 save=none:cbf29ce484222325\n",
            bad_value("rom", "1a2"),
        ),
        (
            "ninge-input 1 rom=+a2b save=none:cbf29ce484222325\n",
            bad_value("rom", "+a2b"),
        ),
        (
            "ninge-input 1 rom=1a2b save=tape:cbf29ce484222325\n",
            bad_value("save", "tape:cbf29ce484222325"),
        ),
        (
            "ninge-input 1 rom=1a2b save=none:cbf29ce4\n",
            bad_value("save", "none:cbf29ce4"),
        ),
        (
            "ninge-input 1 rom=1a2b save=none\n",
            bad_value("save", "none"),
        ),
        ("f=1\n", LogErrorKind::MissingHeader),
    ];
    let failures: Vec<_> = cases
        .iter()
        .filter(|(text, want)| kind_of(text).as_ref() != Some(want))
        .map(|(text, _)| (*text, kind_of(text)))
        .collect();
    assert_eq!(failures, Vec::new());
}

#[test]
fn given_a_text_with_no_lines_when_parsed_then_the_header_is_missing_at_line_zero() {
    assert_eq!(
        parse("").unwrap_err(),
        InputLogError {
            line: 0,
            kind: LogErrorKind::MissingHeader
        }
    );
}

#[test]
fn given_only_comments_when_parsed_then_the_header_is_missing_after_the_last_line() {
    assert_eq!(line_of("# one\n# two\n"), Some(2));
}

// ---- Frame lines ----

#[test]
fn given_a_frame_with_nothing_happening_when_encoded_then_only_its_number_is_written() {
    assert_eq!(encode_frame(4, &Input::new()), "f=4\n");
}

#[test]
fn given_a_frame_with_every_field_when_encoded_then_the_fields_follow_in_the_documented_order() {
    let input = Input::new()
        .with_buttons(Buttons::START | Buttons::A | Buttons::Y)
        .with_touch(Some(Touch { x: 128, y: 96 }))
        .with_hinge_closed(true)
        .with_mic(true)
        .with_wall_time(789_012_345);
    assert_eq!(
        encode_frame(2, &input),
        "f=2 b=A,START,Y t=128,96 hinge mic wall=789012345\n"
    );
}

#[test]
fn given_a_wall_time_of_zero_when_encoded_then_the_wall_field_is_omitted() {
    assert_eq!(
        encode_frame(1, &Input::new().with_buttons(Buttons::B)),
        "f=1 b=B\n"
    );
}

#[test]
fn given_the_documented_example_when_parsed_then_each_frame_has_its_fields() {
    let text = "ninge-input 1 rom=1a2b save=flash-512k:00112233aabbccdd\n\
        # a comment line\n\
        f=1 b=A,START wall=789012345\n\
        f=2 b=A t=128,96 wall=789012345\n\
        f=3 hinge mic wall=789012346\n\
        f=4\n";
    assert_eq!(
        parse(text).unwrap().frames,
        vec![
            Input::new()
                .with_buttons(Buttons::A | Buttons::START)
                .with_wall_time(789_012_345),
            Input::new()
                .with_buttons(Buttons::A)
                .with_touch(Some(Touch { x: 128, y: 96 }))
                .with_wall_time(789_012_345),
            Input::new()
                .with_hinge_closed(true)
                .with_mic(true)
                .with_wall_time(789_012_346),
            Input::new(),
        ]
    );
}

#[test]
fn given_fields_in_another_order_when_parsed_then_the_frame_is_the_same() {
    assert_eq!(
        frame("f=1 wall=5 mic t=1,2 b=B,A hinge\n"),
        frame("f=1 b=A,B t=1,2 hinge mic wall=5\n")
    );
}

#[test]
fn given_a_frame_naming_only_some_fields_when_parsed_then_the_rest_take_their_defaults() {
    assert_eq!(
        frame("f=1 mic\n"),
        Input {
            mic: true,
            ..Input::new()
        }
    );
}

#[test]
fn given_the_touch_corners_when_parsed_then_they_are_accepted() {
    assert_eq!(
        (frame("f=1 t=0,0\n").touch, frame("f=1 t=255,191\n").touch),
        (Some(Touch { x: 0, y: 0 }), Some(Touch { x: 255, y: 191 }))
    );
}

#[test]
fn given_bad_frames_when_parsed_then_each_is_refused_for_its_reason() {
    let failures = kind_failures(&[
        (
            "f=1 t=0,192\n",
            LogErrorKind::TouchOutOfRange { x: 0, y: 192 },
        ),
        (
            "f=1 t=256,0\n",
            LogErrorKind::TouchOutOfRange { x: 256, y: 0 },
        ),
        ("f=1 t=1\n", bad_value("t", "1")),
        ("f=1 t=a,b\n", bad_value("t", "a,b")),
        ("f=1 t\n", bad_value("t", "")),
        ("f=1 b=\n", bad_value("b", "")),
        ("f=1 b\n", bad_value("b", "")),
        ("f=1 b=ZZ\n", LogErrorKind::BadButton("ZZ".to_string())),
        ("f=1 b=a\n", LogErrorKind::BadButton("a".to_string())),
        ("f=1 b=A,,B\n", LogErrorKind::BadButton(String::new())),
        ("f=1 b=A,A\n", LogErrorKind::BadButton("A".to_string())),
        ("f=1 hinge=1\n", bad_value("hinge", "1")),
        ("f=1 mic=yes\n", bad_value("mic", "yes")),
        ("f=1 wall=x\n", bad_value("wall", "x")),
        ("f=1 wall=+5\n", bad_value("wall", "+5")),
        ("f=1 wall=4294967296\n", bad_value("wall", "4294967296")),
        ("f=1 wall\n", bad_value("wall", "")),
        ("f=x\n", bad_value("f", "x")),
        ("f=\n", bad_value("f", "")),
    ]);
    assert_eq!(failures, Vec::new());
}

#[test]
fn given_an_unknown_key_when_parsed_then_it_is_refused_not_ignored() {
    let failures = kind_failures(&[
        (
            "f=1 rumble\n",
            LogErrorKind::UnknownKey("rumble".to_string()),
        ),
        ("f=1 gps=1,2\n", LogErrorKind::UnknownKey("gps".to_string())),
    ]);
    assert_eq!(failures, Vec::new());
}

#[test]
fn given_a_repeated_key_when_parsed_then_it_is_refused() {
    let failures = kind_failures(&[
        ("f=1 b=A b=B\n", LogErrorKind::DuplicateKey("b".to_string())),
        (
            "f=1 t=1,1 t=2,2\n",
            LogErrorKind::DuplicateKey("t".to_string()),
        ),
        (
            "f=1 hinge hinge\n",
            LogErrorKind::DuplicateKey("hinge".to_string()),
        ),
        (
            "f=1 mic mic\n",
            LogErrorKind::DuplicateKey("mic".to_string()),
        ),
        (
            "f=1 wall=1 wall=2\n",
            LogErrorKind::DuplicateKey("wall".to_string()),
        ),
        ("f=1 f=1\n", LogErrorKind::DuplicateKey("f".to_string())),
    ]);
    assert_eq!(failures, Vec::new());
}

#[test]
fn given_frame_numbers_that_are_not_consecutive_when_parsed_then_the_log_is_refused() {
    let failures = kind_failures(&[
        (
            "f=2\n",
            LogErrorKind::FrameOutOfOrder {
                expected: 1,
                found: 2,
            },
        ),
        (
            "f=0\n",
            LogErrorKind::FrameOutOfOrder {
                expected: 1,
                found: 0,
            },
        ),
        (
            "f=1\nf=1\n",
            LogErrorKind::FrameOutOfOrder {
                expected: 2,
                found: 1,
            },
        ),
        (
            "f=1\nf=3\n",
            LogErrorKind::FrameOutOfOrder {
                expected: 2,
                found: 3,
            },
        ),
    ]);
    assert_eq!(failures, Vec::new());
}

#[test]
fn given_a_frame_line_without_a_leading_number_when_parsed_then_it_is_refused() {
    let failures = kind_failures(&[
        ("b=A\n", LogErrorKind::MissingFrameNumber),
        ("b=A f=1\n", LogErrorKind::MissingFrameNumber),
        ("f\n", bad_value("f", "")),
        (
            "ninge-input 1 rom=1a2b save=none:cbf29ce484222325\n",
            LogErrorKind::MissingFrameNumber,
        ),
    ]);
    assert_eq!(failures, Vec::new());
}

#[test]
fn given_a_bad_line_after_comments_when_parsed_then_the_error_names_its_physical_line() {
    assert_eq!(line_of(&log("# note\n\nf=1\nf=3\n")), Some(5));
}

#[test]
fn given_a_refused_log_when_displayed_then_the_message_names_the_line_and_the_problem() {
    assert_eq!(
        parse(&log("f=2\n")).unwrap_err().to_string(),
        "input log line 2: frame number 2 where 1 is next"
    );
}

// ---- Comments, blanks, and the truncated tail ----

#[test]
fn given_comments_and_blank_lines_when_parsed_then_they_are_ignored() {
    let text = "# first\n\n".to_string() + HEADER + "\n# between\nf=1 b=A\n   \n#\nf=2\n";
    assert_eq!(parse(&text).unwrap().frames.len(), 2);
}

#[test]
fn given_crlf_line_ends_when_parsed_then_the_frames_read_the_same() {
    assert_eq!(
        parse(&log("f=1 b=A\r\nf=2\r\n").replace(
            HEADER,
            "ninge-input 1 rom=1a2b save=flash-512k:00112233aabbccdd\r\n"
        ))
        .unwrap()
        .frames
        .len(),
        2
    );
}

#[test]
fn given_a_last_line_with_no_newline_when_parsed_then_it_is_dropped_and_reported() {
    let parsed = parse(&log("f=1 b=A\nf=2 b=B")).unwrap();
    assert_eq!(
        (parsed.frames.len(), parsed.dropped_partial_line),
        (1, true)
    );
}

#[test]
fn given_a_cut_off_mid_field_when_parsed_then_the_prefix_replays_without_error() {
    let parsed = parse(&log("f=1 b=A\nf=2 t=12")).unwrap();
    assert_eq!(parsed.frames, vec![Input::new().with_buttons(Buttons::A)]);
}

#[test]
fn given_a_log_ending_in_a_newline_when_parsed_then_nothing_is_dropped() {
    assert!(!parse(&log("f=1\n")).unwrap().dropped_partial_line);
}

#[test]
fn given_a_header_with_no_newline_when_parsed_then_the_header_is_missing() {
    assert_eq!(
        kind_of(HEADER.trim_end()),
        Some(LogErrorKind::MissingHeader)
    );
}

#[test]
fn given_a_header_only_log_when_parsed_then_it_has_no_frames() {
    assert_eq!(parse(HEADER).unwrap().frames, Vec::new());
}

// ---- The line-by-line reader ----

#[test]
fn given_lines_fed_one_at_a_time_when_read_then_each_gives_its_meaning() {
    let mut reader = LogReader::new();
    let fed = [
        reader.feed(HEADER),
        reader.feed("# note\n"),
        reader.feed("f=1 mic\n"),
        reader.feed("f=2"),
    ];
    assert_eq!(
        fed,
        [
            Ok(Some(LogLine::Header(header()))),
            Ok(None),
            Ok(Some(LogLine::Frame(Input::new().with_mic(true)))),
            Ok(Some(LogLine::Frame(Input::new()))),
        ]
    );
}

#[test]
fn given_a_refused_line_when_the_next_is_fed_then_the_reader_counts_on_from_the_same_frame() {
    let mut reader = LogReader::new();
    reader.feed(HEADER).unwrap();
    let skipped = reader.feed("f=2\n").is_err();
    assert_eq!(
        (skipped, reader.feed("f=1\n").map_err(|e| e.line)),
        (true, Ok(Some(LogLine::Frame(Input::new()))))
    );
}

// ---- Time ----

#[test]
fn given_a_recorded_wall_time_when_the_line_is_parsed_then_the_frame_carries_it() {
    assert_eq!(frame("f=1 wall=789012345\n").wall_time, 789_012_345);
}

#[test]
fn given_a_frame_with_no_wall_field_when_parsed_then_wall_time_is_zero_not_the_host_clock() {
    assert_eq!(frame("f=1 b=A\n").wall_time, 0);
}

// ---- Round trips ----

/// What a reader makes of `input`'s line as frame `number`, after `number - 1` empty frames.
fn feed_as_frame(number: u32, input: &Input) -> Result<Option<LogLine>, InputLogError> {
    let mut reader = LogReader::new();
    reader.feed(HEADER).unwrap();
    (1..number).for_each(|n| {
        reader.feed(&encode_frame(n, &Input::new())).unwrap();
    });
    reader.feed(&encode_frame(number, input))
}

fn any_input() -> impl Strategy<Value = Input> {
    (
        0u16..0x1000,
        proptest::option::of((any::<u8>(), 0u8..=191)),
        any::<bool>(),
        any::<bool>(),
        any::<u32>(),
    )
        .prop_map(|(buttons, touch, hinge, mic, wall)| {
            Input::new()
                .with_buttons(Buttons::from_bits_truncate(buttons))
                .with_touch(touch.map(|(x, y)| Touch { x, y }))
                .with_hinge_closed(hinge)
                .with_mic(mic)
                .with_wall_time(wall)
        })
}

proptest! {
    #[test]
    fn given_any_input_when_written_as_a_line_and_read_back_then_it_is_the_same_input(
        input in any_input(),
        number in 1u32..1000
    ) {
        let read = feed_as_frame(number, &input);
        prop_assert_eq!(read, Ok(Some(LogLine::Frame(input))));
    }

    #[test]
    fn given_any_inputs_when_written_as_a_log_and_parsed_then_the_frames_are_the_same(
        inputs in proptest::collection::vec(any_input(), 0..20)
    ) {
        let text = inputs.iter().enumerate().fold(HEADER.to_string(), |mut text, (i, input)| {
            text.push_str(&encode_frame(i as u32 + 1, input));
            text
        });
        let parsed = parse(&text).unwrap();
        prop_assert_eq!((parsed.header, parsed.frames), (header(), inputs));
    }

    #[test]
    fn given_any_text_when_parsed_then_it_never_panics(text in "\\PC{0,120}") {
        let outcome = parse(&text);
        prop_assert!(outcome.is_ok() || outcome.is_err());
    }
}

#[test]
fn given_a_touch_below_the_last_row_when_encoded_then_the_reader_accepts_it_as_the_last_row() {
    let input = Input::new().with_touch(Some(Touch { x: 7, y: 200 }));
    let text = log(&encode_frame(1, &input));
    let read = parse(&text).map(|parsed| parsed.frames.first().and_then(|frame| frame.touch));
    assert_eq!(read, Ok(Some(Touch { x: 7, y: 191 })));
}

#[test]
fn given_a_touch_below_the_last_row_when_latched_then_the_machine_holds_the_last_row() {
    let mut machine = crate::Machine::new();
    machine.run_frame(Input::new().with_touch(Some(Touch { x: 7, y: 200 })));
    assert_eq!(
        machine.inspect().input().touch,
        Some(Touch { x: 7, y: 191 })
    );
}
