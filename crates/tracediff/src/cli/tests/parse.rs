// SPDX-License-Identifier: Apache-2.0
//! Argument parsing: every command and flag, the defaults, the malformed lines, and the usage text
//! with its rule about traces of a ROM that is not ours (`trace-oracle.md`, "Traces of a user's
//! ROM": run it only yourself, locally; never commit or paste such a trace).

use ninge_core::trace::TraceCpu;

use super::{live_args, os};
use crate::cli::{CliError, Command, LiveArgs, USAGE, parse};

fn parsed(args: &[&str]) -> Result<Command, CliError> {
    parse(os(args))
}

/// The names of the lines that did not fail with a usage error that carries the usage text.
fn not_usage_errors(lines: &[(&'static str, &[&str])]) -> Vec<&'static str> {
    lines
        .iter()
        .filter(|(_, args)| {
            !matches!(parsed(args), Err(CliError::Usage(_)))
                || !parsed(args)
                    .err()
                    .is_some_and(|error| error.to_string().contains(USAGE))
        })
        .map(|(name, _)| *name)
        .collect()
}

// ---- the commands ----

#[test]
fn given_files_with_two_paths_when_parsed_then_both_cpus_are_compared() {
    assert_eq!(
        parsed(&["files", "a.trace", "b.trace"]).unwrap(),
        Command::Files {
            ninge: "a.trace".into(),
            oracle: "b.trace".into(),
            cpu: None,
        }
    );
}

#[test]
fn given_files_with_cpu_9_when_parsed_then_only_the_arm9_is_selected() {
    assert_eq!(
        parsed(&["files", "a", "b", "--cpu", "9"]).unwrap(),
        Command::Files {
            ninge: "a".into(),
            oracle: "b".into(),
            cpu: Some(TraceCpu::Arm9),
        }
    );
}

#[test]
fn given_files_with_cpu_7_when_parsed_then_only_the_arm7_is_selected() {
    assert_eq!(
        parsed(&["files", "a", "b", "--cpu", "7"]).unwrap(),
        Command::Files {
            ninge: "a".into(),
            oracle: "b".into(),
            cpu: Some(TraceCpu::Arm7),
        }
    );
}

#[test]
fn given_live_with_the_required_flags_when_parsed_then_the_options_have_their_defaults() {
    assert_eq!(
        parsed(&["live", "t.trace", "--port", "1234", "--cpu", "9"]).unwrap(),
        Command::Live(LiveArgs {
            ninge: "t.trace".into(),
            port: 1234,
            ..live_args(TraceCpu::Arm9)
        })
    );
}

#[test]
fn given_live_with_every_flag_when_parsed_then_each_value_lands_in_its_field() {
    let command = parsed(&[
        "live",
        "t.trace",
        "--max",
        "500",
        "--from-hit",
        "3",
        "--breakpoint",
        "02000004",
        "--cpu",
        "7",
        "--port",
        "65535",
    ])
    .unwrap();
    assert_eq!(
        command,
        Command::Live(LiveArgs {
            ninge: "t.trace".into(),
            port: 65535,
            cpu: TraceCpu::Arm7,
            breakpoint: Some(0x0200_0004),
            from_hit: 3,
            hits: u32::MAX,
            max: Some(500),
        })
    );
}

#[test]
fn given_boundaries_with_a_breakpoint_when_parsed_then_hits_default_to_all() {
    assert_eq!(
        parsed(&[
            "boundaries",
            "t",
            "--port",
            "9",
            "--cpu",
            "9",
            "--breakpoint",
            "2000000"
        ])
        .unwrap(),
        Command::Boundaries(LiveArgs {
            ninge: "t".into(),
            port: 9,
            breakpoint: Some(0x0200_0000),
            ..live_args(TraceCpu::Arm9)
        })
    );
}

#[test]
fn given_boundaries_with_hits_when_parsed_then_the_limit_is_kept() {
    let command = parsed(&[
        "boundaries",
        "t",
        "--port",
        "9",
        "--cpu",
        "7",
        "--breakpoint",
        "0",
        "--hits",
        "12",
    ])
    .unwrap();
    assert_eq!(
        command,
        Command::Boundaries(LiveArgs {
            ninge: "t".into(),
            port: 9,
            cpu: TraceCpu::Arm7,
            breakpoint: Some(0),
            hits: 12,
            ..live_args(TraceCpu::Arm7)
        })
    );
}

/// The breakpoint a `live` line with `--breakpoint text` parses to, `None` if it does not parse.
fn breakpoint_of(text: &str) -> Option<u32> {
    match parsed(&[
        "live",
        "t",
        "--port",
        "1",
        "--cpu",
        "9",
        "--breakpoint",
        text,
    ]) {
        Ok(Command::Live(args)) => args.breakpoint,
        _ => None,
    }
}

#[test]
fn given_addresses_with_and_without_a_0x_prefix_when_parsed_then_both_are_hex() {
    assert_eq!(
        [
            breakpoint_of("0x2000004"),
            breakpoint_of("2000004"),
            breakpoint_of("FFFF0000")
        ],
        [Some(0x0200_0004), Some(0x0200_0004), Some(0xFFFF_0000)]
    );
}

#[test]
fn given_a_from_hit_of_0_without_a_breakpoint_when_parsed_then_it_is_accepted() {
    assert!(matches!(
        parsed(&["live", "t", "--port", "1", "--cpu", "9", "--from-hit", "0"]),
        Ok(Command::Live(_))
    ));
}

// ---- malformed lines ----

#[test]
fn given_malformed_lines_when_parsed_then_each_is_a_usage_error_with_the_usage_text() {
    let lines: &[(&'static str, &[&str])] = &[
        ("no arguments", &[]),
        ("a command and nothing else", &["files"]),
        ("files with one path", &["files", "a"]),
        ("an unknown command", &["compare", "a", "b"]),
        ("a flag where the trace goes", &["live", "--port", "1"]),
        ("cpu without a value", &["files", "a", "b", "--cpu"]),
        ("cpu 8", &["files", "a", "b", "--cpu", "8"]),
        ("cpu in words", &["files", "a", "b", "--cpu", "arm9"]),
        (
            "cpu given twice",
            &["files", "a", "b", "--cpu", "9", "--cpu", "7"],
        ),
        ("an extra argument on files", &["files", "a", "b", "c"]),
        (
            "an unknown flag on files",
            &["files", "a", "b", "--port", "1"],
        ),
        ("live without a port", &["live", "t", "--cpu", "9"]),
        ("live without a cpu", &["live", "t", "--port", "1"]),
        (
            "a port out of range",
            &["live", "t", "--port", "65536", "--cpu", "9"],
        ),
        (
            "a port that is not a number",
            &["live", "t", "--port", "http", "--cpu", "9"],
        ),
        (
            "a negative port",
            &["live", "t", "--port", "-1", "--cpu", "9"],
        ),
        (
            "a port given twice",
            &["live", "t", "--port", "1", "--port", "2", "--cpu", "9"],
        ),
        (
            "a port flag without a value",
            &["live", "t", "--cpu", "9", "--port"],
        ),
        (
            "a breakpoint that is not hex",
            &[
                "live",
                "t",
                "--port",
                "1",
                "--cpu",
                "9",
                "--breakpoint",
                "main",
            ],
        ),
        (
            "a breakpoint wider than 32 bits",
            &[
                "live",
                "t",
                "--port",
                "1",
                "--cpu",
                "9",
                "--breakpoint",
                "100000000",
            ],
        ),
        (
            "from-hit without a breakpoint",
            &["live", "t", "--port", "1", "--cpu", "9", "--from-hit", "2"],
        ),
        (
            "from-hit that is not a number",
            &[
                "live",
                "t",
                "--port",
                "1",
                "--cpu",
                "9",
                "--breakpoint",
                "0",
                "--from-hit",
                "x",
            ],
        ),
        (
            "max that is negative",
            &["live", "t", "--port", "1", "--cpu", "9", "--max", "-5"],
        ),
        (
            "an unknown flag on live",
            &["live", "t", "--port", "1", "--cpu", "9", "--verbose", "1"],
        ),
        (
            "boundaries without a breakpoint",
            &["boundaries", "t", "--port", "1", "--cpu", "9"],
        ),
        (
            "hits that is not a number",
            &[
                "boundaries",
                "t",
                "--port",
                "1",
                "--cpu",
                "9",
                "--breakpoint",
                "0",
                "--hits",
                "many",
            ],
        ),
    ];
    assert_eq!(not_usage_errors(lines), Vec::<&str>::new());
}

#[test]
fn given_a_path_that_is_not_utf8_when_parsed_then_the_path_is_kept_as_is() {
    // Paths are not required to be text; only flags and values are.
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![b'a', 0xFF])
    };
    #[cfg(not(unix))]
    let path = std::ffi::OsString::from("a");
    let command = parse([
        std::ffi::OsString::from("files"),
        path.clone(),
        std::ffi::OsString::from("b"),
    ])
    .unwrap();
    assert_eq!(
        command,
        Command::Files {
            ninge: path.into(),
            oracle: "b".into(),
            cpu: None,
        }
    );
}

// ---- the usage text ----

/// The usage text with its line breaks and runs of spaces as single spaces, so a phrase is found
/// wherever the text wraps.
fn usage_flat() -> String {
    USAGE.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn given_the_usage_text_when_read_then_it_warns_about_a_rom_that_is_not_yours_to_share() {
    assert!(usage_flat().contains("not yours to share"));
}

#[test]
fn given_the_usage_text_when_read_then_it_says_never_to_commit_such_a_trace() {
    assert!(usage_flat().contains("never commit it"));
}

#[test]
fn given_the_usage_text_when_read_then_it_lists_the_three_commands_and_the_local_oracle() {
    let present = ["files", "live", "boundaries", "127.0.0.1"]
        .iter()
        .map(|word| USAGE.contains(word))
        .collect::<Vec<bool>>();
    assert_eq!(present, vec![true; 4]);
}

#[test]
fn given_a_usage_error_when_displayed_then_the_reason_comes_before_the_usage() {
    let text = CliError::Usage("missing command".to_string()).to_string();
    assert_eq!(text, format!("missing command\n{USAGE}"));
}
