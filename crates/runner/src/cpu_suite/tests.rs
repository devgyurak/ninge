// SPDX-License-Identifier: Apache-2.0
use super::*;

use std::error::Error;

// --- encoding helpers: build suite bytes the way the converter's format describes them ---

fn push(out: &mut Vec<u8>, words: &[u32]) {
    words
        .iter()
        .for_each(|w| out.extend_from_slice(&w.to_le_bytes()));
}

fn encode_state(s: &State) -> Vec<u8> {
    let mut out = Vec::new();
    push(&mut out, &[u32::try_from(STATE_BLOCK_MIN).unwrap(), 0]);
    push(&mut out, &s.r);
    push(&mut out, &s.r_fiq);
    push(&mut out, &s.r_svc);
    push(&mut out, &s.r_abt);
    push(&mut out, &s.r_irq);
    push(&mut out, &s.r_und);
    push(&mut out, &[s.cpsr]);
    push(&mut out, &s.spsr);
    push(&mut out, &s.pipeline);
    push(&mut out, &[s.access]);
    out
}

fn encode_transactions(list: &[Transaction]) -> Vec<u8> {
    let size = TRANSACTIONS_BLOCK_MIN + TRANSACTION_BYTES * list.len();
    let mut out = Vec::new();
    push(
        &mut out,
        &[
            u32::try_from(size).unwrap(),
            TRANSACTIONS_MAGIC,
            u32::try_from(list.len()).unwrap(),
        ],
    );
    list.iter().for_each(|t| {
        push(
            &mut out,
            &[t.kind, t.size, t.addr, t.data, t.cycle, t.access],
        )
    });
    out
}

fn encode_case(c: &Case) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(encode_state(&c.initial));
    body.extend(encode_state(&c.expected));
    body.extend(encode_transactions(&c.transactions));
    push(
        &mut body,
        &[
            u32::try_from(OPCODE_BLOCK_MIN).unwrap(),
            0,
            c.opcode,
            c.base_addr,
        ],
    );
    let mut out = Vec::new();
    push(&mut out, &[u32::try_from(body.len() + 4).unwrap()]);
    out.extend(body);
    out
}

fn encode_file(cases: &[Case]) -> Vec<u8> {
    let mut out = Vec::new();
    push(&mut out, &[MAGIC, u32::try_from(cases.len()).unwrap()]);
    cases.iter().for_each(|c| out.extend(encode_case(c)));
    out
}

// --- fixtures ---

fn state() -> State {
    State {
        r: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 213, 214, 15],
        r_fiq: [18, 19, 110, 111, 112, 113, 114],
        r_svc: [213, 214],
        r_abt: [313, 314],
        r_irq: [413, 414],
        r_und: [513, 514],
        cpsr: 0xD3,
        spsr: [10, 11, 12, 13, 14],
        pipeline: [0xAAAA, 0xBBBB],
        access: 7,
    }
}

fn txn(kind: u32, size: u32, addr: u32, data: u32) -> Transaction {
    Transaction {
        kind,
        size,
        addr,
        data,
        cycle: 1,
        access: 2,
    }
}

fn case() -> Case {
    Case {
        initial: state(),
        expected: state(),
        transactions: vec![txn(0, 4, 0x100, 0xE1A0_0000)],
        opcode: 0xE1A0_0000,
        base_addr: 0x100,
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ninge-cpu-suite-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The path an IO error names, or `None` for any other error.
fn io_path(error: &SuiteError) -> Option<String> {
    match error {
        SuiteError::Io { path, .. } => path.file_name().map(|n| n.to_string_lossy().into_owned()),
        SuiteError::Format { .. } => None,
    }
}

/// Lengths of strict prefixes of `bytes` that decode without an error.
fn prefixes_accepted(bytes: &[u8]) -> Vec<usize> {
    (0..bytes.len())
        .filter(|&n| decode(&bytes[..n]).is_ok())
        .collect()
}

/// Runs a case whose expectation changes one register field, returning the diff names.
fn diff_names_after(change: impl Fn(&mut State)) -> Vec<String> {
    let mut c = case();
    change(&mut c.expected);
    run_case::<false>(&c).into_iter().map(|d| d.name).collect()
}

// --- reader ---

#[test]
fn given_an_encoded_case_when_decoding_then_every_field_round_trips() {
    let bytes = encode_file(&[case()]);
    assert_eq!(decode(&bytes).unwrap(), vec![case()]);
}

#[test]
fn given_a_file_with_no_cases_when_decoding_then_the_list_is_empty() {
    assert_eq!(decode(&encode_file(&[])).unwrap(), vec![]);
}

#[test]
fn given_two_cases_when_decoding_then_both_come_back_in_order() {
    let mut second = case();
    second.opcode = 0x1234_5678;
    let bytes = encode_file(&[case(), second.clone()]);
    assert_eq!(decode(&bytes).unwrap(), vec![case(), second]);
}

#[test]
fn given_a_case_with_no_transactions_when_decoding_then_the_list_is_empty() {
    let mut c = case();
    c.transactions.clear();
    assert_eq!(decode(&encode_file(&[c.clone()])).unwrap(), vec![c]);
}

#[test]
fn given_a_wrong_magic_when_decoding_then_the_error_names_byte_zero() {
    let mut bytes = encode_file(&[case()]);
    bytes[0] ^= 0xFF;
    let error = decode(&bytes).unwrap_err();
    assert!(
        matches!(error, SuiteError::Format { offset: 0, .. }),
        "{error}"
    );
}

#[test]
fn given_every_truncation_of_a_file_when_decoding_then_none_is_accepted() {
    let bytes = encode_file(&[case(), case()]);
    assert_eq!(prefixes_accepted(&bytes), Vec::<usize>::new());
}

#[test]
fn given_a_wrong_transactions_magic_when_decoding_then_the_file_is_refused() {
    let mut bytes = encode_file(&[case()]);
    let at = 8 + 4 + 2 * STATE_BLOCK_MIN + 4;
    bytes[at] = 9;
    let error = decode(&bytes).unwrap_err();
    assert!(error.to_string().contains("wrong magic"), "{error}");
}

#[test]
fn given_a_block_size_below_its_fixed_part_when_decoding_then_the_file_is_refused() {
    let mut bytes = encode_file(&[case()]);
    bytes[12..16].copy_from_slice(&4u32.to_le_bytes());
    let error = decode(&bytes).unwrap_err();
    assert!(error.to_string().contains("smaller"), "{error}");
}

#[test]
fn given_a_huge_case_count_and_no_data_when_decoding_then_it_fails_without_allocating_it() {
    let mut bytes = Vec::new();
    push(&mut bytes, &[MAGIC, u32::MAX]);
    assert!(decode(&bytes).is_err());
}

#[test]
fn given_a_transaction_count_beyond_the_data_when_decoding_then_the_file_is_refused() {
    let mut bytes = encode_file(&[case()]);
    let at = 8 + 4 + 2 * STATE_BLOCK_MIN + 8;
    bytes[at..at + 4].copy_from_slice(&1000u32.to_le_bytes());
    assert!(decode(&bytes).is_err());
}

// --- replay bus ---

#[test]
fn given_the_base_address_when_fetching_then_the_opcode_is_returned() {
    let mut bus = ReplayBus::new(&case());
    assert_eq!(
        (bus.fetch32(0x100), bus.fetch16(0x100)),
        (0xE1A0_0000, 0x0000)
    );
}

#[test]
fn given_another_address_when_fetching_then_the_address_itself_is_returned() {
    let mut bus = ReplayBus::new(&case());
    assert_eq!((bus.fetch32(0x104), bus.fetch16(0x106)), (0x104, 0x106));
}

#[test]
fn given_a_recorded_read_when_reading_then_its_data_is_returned_once() {
    let mut c = case();
    c.transactions = vec![txn(1, 4, 0x2000, 0xCAFE_F00D)];
    let mut bus = ReplayBus::new(&c);
    assert_eq!((bus.read32(0x2000), bus.read32(0x2000)), (0xCAFE_F00D, 0));
}

#[test]
fn given_two_recorded_reads_of_one_address_when_reading_twice_then_they_come_back_in_order() {
    let mut c = case();
    c.transactions = vec![txn(1, 4, 0x2000, 1), txn(1, 4, 0x2000, 2)];
    let mut bus = ReplayBus::new(&c);
    assert_eq!((bus.read32(0x2000), bus.read32(0x2000)), (1, 2));
}

#[test]
fn given_recorded_reads_of_each_size_when_reading_then_the_size_must_match() {
    let mut c = case();
    c.transactions = vec![txn(1, 1, 0x10, 0x1FF), txn(1, 2, 0x10, 0x1_2345)];
    let mut bus = ReplayBus::new(&c);
    assert_eq!(
        (bus.read8(0x10), bus.read16(0x10), bus.read32(0x10)),
        (0xFF, 0x2345, 0)
    );
}

#[test]
fn given_accesses_when_logged_then_the_log_keeps_kind_size_address_data_in_order() {
    let mut c = case();
    c.transactions = vec![txn(1, 4, 0x20, 9)];
    let mut bus = ReplayBus::new(&c);
    bus.read32(0x20);
    bus.write8(0x21, 0xAB);
    bus.write16(0x22, 0xABCD);
    bus.write32(0x24, 0x1122_3344);
    let log: Vec<(u32, u32, u32, u32)> = bus
        .log
        .iter()
        .map(|a| (a.kind, a.size, a.addr, a.data))
        .collect();
    assert_eq!(
        log,
        vec![
            (1, 4, 0x20, 9),
            (2, 1, 0x21, 0xAB),
            (2, 2, 0x22, 0xABCD),
            (2, 4, 0x24, 0x1122_3344)
        ]
    );
}

// --- harness ---

#[test]
fn given_a_case_that_expects_no_change_when_running_then_there_are_no_diffs() {
    assert_eq!(run_case::<false>(&case()), vec![]);
}

#[test]
fn given_the_arm9_flavor_when_running_a_no_change_case_then_there_are_no_diffs() {
    assert_eq!(run_case::<true>(&case()), vec![]);
}

#[test]
fn given_a_different_final_r0_when_running_then_one_diff_names_r0_with_both_values() {
    let mut c = case();
    c.expected.r[0] = 0xFFFF_0000;
    assert_eq!(
        run_case::<false>(&c),
        vec![FieldDiff {
            name: "R0".to_string(),
            expected: "0xffff0000".to_string(),
            actual: "0x00000000".to_string(),
        }]
    );
}

#[test]
fn given_changes_to_banked_registers_cpsr_and_spsr_when_running_then_each_is_named() {
    let names_seen = diff_names_after(|s| {
        s.r_fiq[2] ^= 1;
        s.r_svc[0] ^= 1;
        s.r_abt[1] ^= 1;
        s.r_irq[0] ^= 1;
        s.r_und[1] ^= 1;
        s.cpsr ^= 0x8000_0000;
        s.spsr[4] ^= 1;
    });
    assert_eq!(
        names_seen,
        vec![
            "R_fiq[2]", "R_svc[0]", "R_abt[1]", "R_irq[0]", "R_und[1]", "CPSR", "SPSR[4]"
        ]
    );
}

#[test]
fn given_pipeline_and_access_values_that_differ_when_running_then_they_are_not_compared() {
    let mut c = case();
    c.expected.pipeline = [1, 2];
    c.expected.access = 99;
    assert_eq!(run_case::<false>(&c), vec![]);
}

#[test]
fn given_a_data_write_the_cpu_never_made_when_running_then_the_transaction_is_reported_missing() {
    let mut c = case();
    c.transactions.push(txn(2, 4, 0x3000, 5));
    let diffs = run_case::<false>(&c);
    assert_eq!(
        diffs,
        vec![FieldDiff {
            name: "transaction[0]".to_string(),
            expected: "write size=4 addr=0x00003000 data=0x00000005".to_string(),
            actual: "none".to_string(),
        }]
    );
}

#[test]
fn given_a_data_read_the_cpu_never_made_when_running_then_the_read_is_described() {
    let mut c = case();
    c.transactions.push(txn(1, 2, 0x10, 5));
    assert_eq!(
        run_case::<false>(&c)[0].expected,
        "read size=2 addr=0x00000010 data=0x00000005"
    );
}

#[test]
fn given_a_fetch_transaction_only_when_running_then_it_is_not_a_data_diff() {
    let mut c = case();
    c.transactions = vec![txn(0, 4, 0x100, 1), txn(0, 4, 0x104, 2)];
    assert_eq!(run_case::<false>(&c), vec![]);
}

/// A state whose visible registers agree with the bank `mode` names, as every suite state does.
fn state_in(mode: u32) -> State {
    let mut s = state();
    s.cpsr = 0xC0 | mode;
    match mode {
        0x11 => s.r[8..=14].copy_from_slice(&s.r_fiq),
        0x12 => s.r[13..=14].copy_from_slice(&s.r_irq),
        0x13 => s.r[13..=14].copy_from_slice(&s.r_svc),
        0x17 => s.r[13..=14].copy_from_slice(&s.r_abt),
        0x1B => s.r[13..=14].copy_from_slice(&s.r_und),
        _ => {}
    }
    s
}

/// Modes whose consistent state does not survive a load followed by a no-op step.
fn modes_that_do_not_round_trip() -> Vec<u32> {
    [0x10, 0x11, 0x12, 0x13, 0x17, 0x1B, 0x1F]
        .into_iter()
        .filter(|&mode| {
            let mut c = case();
            c.initial = state_in(mode);
            c.expected = state_in(mode);
            !run_case::<false>(&c).is_empty()
        })
        .collect()
}

#[test]
fn given_a_state_in_each_mode_when_loading_then_a_no_op_step_leaves_no_diff() {
    assert_eq!(modes_that_do_not_round_trip(), Vec::<u32>::new());
}

#[test]
fn given_cases_when_running_them_then_only_failing_ones_are_returned_with_index_file_and_opcode() {
    let mut bad = case();
    bad.opcode = 0xDEAD_BEEF;
    bad.expected.r[1] = 0;
    let diffs = run_cases::<false>("x.json.bin", &[case(), bad, case()]);
    let summary: Vec<(String, usize, u32, usize)> = diffs
        .iter()
        .map(|d| (d.file.clone(), d.index, d.opcode, d.fields.len()))
        .collect();
    assert_eq!(summary, vec![("x.json.bin".to_string(), 1, 0xDEAD_BEEF, 1)]);
}

#[test]
fn given_a_suite_file_on_disk_when_running_it_then_the_failures_carry_its_file_name() {
    let dir = temp_dir("one-file");
    let mut bad = case();
    bad.expected.cpsr = 0;
    std::fs::write(dir.join("a.json.bin"), encode_file(&[case(), bad])).unwrap();
    let diffs = run_json_suite::<false>(&dir.join("a.json.bin")).unwrap();
    assert_eq!(
        diffs
            .iter()
            .map(|d| (d.file.as_str(), d.index))
            .collect::<Vec<_>>(),
        vec![("a.json.bin", 1)]
    );
}

#[test]
fn given_a_missing_suite_file_when_running_it_then_the_error_is_an_io_error_naming_the_path() {
    let error = run_json_suite::<false>(Path::new("/nonexistent/ninge/a.json.bin")).unwrap_err();
    assert_eq!(io_path(&error), Some("a.json.bin".to_string()));
}

#[test]
fn given_a_directory_of_suite_files_when_summarizing_then_counts_follow_file_name_order() {
    let dir = temp_dir("summary");
    let mut bad = case();
    bad.expected.r[2] = 0;
    std::fs::write(dir.join("b.json.bin"), encode_file(&[case()])).unwrap();
    std::fs::write(dir.join("a.json.bin"), encode_file(&[case(), bad, case()])).unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    let lines: Vec<String> = summarize_dir(&dir)
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        lines,
        vec![
            "a.json.bin: 3 cases, 1 failed",
            "b.json.bin: 1 cases, 0 failed"
        ]
    );
}

#[test]
fn given_a_missing_directory_when_summarizing_then_the_error_is_an_io_error() {
    let error = summarize_dir(Path::new("/nonexistent/ninge/suite")).unwrap_err();
    assert!(matches!(error, SuiteError::Io { .. }), "{error}");
}

#[test]
fn given_a_malformed_suite_file_in_the_directory_when_summarizing_then_the_error_is_a_format_error()
{
    let dir = temp_dir("malformed");
    std::fs::write(dir.join("a.json.bin"), b"not a suite file").unwrap();
    let error = summarize_dir(&dir).unwrap_err();
    assert!(matches!(error, SuiteError::Format { .. }), "{error}");
}

#[test]
fn given_a_directory_entry_that_cannot_be_read_as_a_file_when_summarizing_then_the_error_names_it()
{
    let dir = temp_dir("unreadable");
    std::fs::create_dir_all(dir.join("a.json.bin")).unwrap();
    let error = summarize_dir(&dir).unwrap_err();
    assert_eq!(io_path(&error), Some("a.json.bin".to_string()));
}

#[test]
fn given_suite_errors_when_displayed_then_the_message_names_the_cause_and_io_has_a_source() {
    let format = SuiteError::Format {
        offset: 12,
        reason: "nope".to_string(),
    };
    let io = SuiteError::Io {
        path: PathBuf::from("/x"),
        source: std::io::Error::other("boom"),
    };
    assert_eq!(
        (
            format.to_string(),
            io.to_string(),
            format.source().is_none(),
            io.source().is_some()
        ),
        (
            "bad suite data at byte 12: nope".to_string(),
            "cannot read /x: boom".to_string(),
            true,
            true
        )
    );
}

#[test]
fn given_the_manifest_directory_when_asking_for_the_suite_dir_then_it_ends_in_the_data_folder() {
    assert!(suite_dir().ends_with("tests/data/arm7tdmi/v1"));
}

#[test]
fn given_a_directory_of_suite_files_when_reporting_then_a_total_line_follows_the_files() {
    let dir = temp_dir("report");
    let mut bad = case();
    bad.expected.r[2] = 0;
    std::fs::write(dir.join("a.json.bin"), encode_file(&[case(), bad])).unwrap();
    std::fs::write(dir.join("b.json.bin"), encode_file(&[case()])).unwrap();
    assert_eq!(
        report_dir(&dir).unwrap(),
        "a.json.bin: 2 cases, 1 failed\nb.json.bin: 1 cases, 0 failed\ntotal: 3 cases, 1 failed"
    );
}

#[test]
fn given_an_empty_directory_when_reporting_then_only_the_zero_total_is_printed() {
    assert_eq!(
        report_dir(&temp_dir("empty")).unwrap(),
        "total: 0 cases, 0 failed"
    );
}
