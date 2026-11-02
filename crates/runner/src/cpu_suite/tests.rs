// SPDX-License-Identifier: Apache-2.0
use super::*;

use std::error::Error;
use std::ops::Deref;

// --- encoding helpers: build suite bytes the way the converter's format describes them ---

/// Marker word after the size of an initial state block, of a final state block, and of an
/// opcode block, as the suite writes them.
const INITIAL_MARKER: u32 = 1;
const FINAL_MARKER: u32 = 2;
const OPCODE_MARKER: u32 = 4;

/// Where the pieces of a one-transaction file sit: the file header is 8 bytes, then the case
/// size, the initial state block, the final state block, the transactions block, and the
/// opcode block.
const CASE_AT: usize = 8;
const INITIAL_AT: usize = CASE_AT + 4;
const FINAL_AT: usize = INITIAL_AT + 168;
const TRANSACTIONS_AT: usize = FINAL_AT + 168;
const OPCODE_AT: usize = TRANSACTIONS_AT + 12 + 24;

fn push(out: &mut Vec<u8>, words: &[u32]) {
    words
        .iter()
        .for_each(|w| out.extend_from_slice(&w.to_le_bytes()));
}

fn encode_state(s: &State, marker: u32) -> Vec<u8> {
    let mut out = Vec::new();
    push(&mut out, &[168, marker]);
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
    let size = 12 + 24 * list.len();
    let mut out = Vec::new();
    push(
        &mut out,
        &[
            u32::try_from(size).unwrap(),
            3,
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
    body.extend(encode_state(&c.initial, INITIAL_MARKER));
    body.extend(encode_state(&c.expected, FINAL_MARKER));
    body.extend(encode_transactions(&c.transactions));
    push(&mut body, &[16, OPCODE_MARKER, c.opcode, c.base_addr]);
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

/// `bytes` with the little-endian word at `at` replaced.
fn with_word(mut bytes: Vec<u8>, at: usize, value: u32) -> Vec<u8> {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    bytes
}

/// The message of the error decoding `bytes` gives, or `"accepted"` if it gives none.
fn decode_error(bytes: &[u8]) -> String {
    decode(bytes).map_or_else(|e| e.to_string(), |_| "accepted".to_string())
}

// --- fixtures: every register and bank holds its own value ---

fn state() -> State {
    State {
        r: [
            0x1000, 0x1001, 0x1002, 0x1003, 0x1004, 0x1005, 0x1006, 0x1007, // R0–R7
            0x2008, 0x2009, 0x200A, 0x200B, 0x200C, 0x200D, 0x200E, // user R8–R14
            0x108,  // R15: base_addr + 8
        ],
        r_fiq: [0x3008, 0x3009, 0x300A, 0x300B, 0x300C, 0x300D, 0x300E],
        r_svc: [0x400D, 0x400E],
        r_abt: [0x500D, 0x500E],
        r_irq: [0x600D, 0x600E],
        r_und: [0x700D, 0x700E],
        cpsr: 0xD3,
        spsr: [0x8000, 0x8001, 0x8002, 0x8003, 0x8004],
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
        transactions: vec![txn(0, 4, 0x100, 0xE1A0_1234)],
        opcode: 0xE1A0_1234,
        base_addr: 0x100,
    }
}

/// A directory under the system temp dir that is removed when it goes out of scope.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ninge-cpu-suite-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The step of a CPU that executes nothing, so harness tests do not depend on `Cpu::step`.
fn noop<const ARM9: bool>(_cpu: &mut Cpu<ARM9>, _bus: &mut ReplayBus) {}

fn run_noop(case: &Case) -> Vec<FieldDiff> {
    run_case_with::<false>(case, noop::<false>)
}

/// The file name an IO error names, or `None` for any other error.
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

/// A CPU loaded with `state`.
fn loaded(state: &State) -> Cpu<false> {
    let mut cpu = Cpu::<false>::new();
    load(&mut cpu, state);
    cpu
}

fn visible(cpu: &Cpu<false>, range: std::ops::RangeInclusive<usize>) -> Vec<u32> {
    range.map(|i| cpu.regs().get(i)).collect()
}

fn in_mode(mode: u32) -> State {
    let mut s = state();
    s.cpsr = 0xC0 | mode;
    s
}

const MODES: [u32; 7] = [0x10, 0x11, 0x12, 0x13, 0x17, 0x1B, 0x1F];

/// Every register the suite gives for `s`, as (bank, register index, value).
fn suite_registers(s: &State) -> Vec<(Bank, usize, u32)> {
    let banks: [(Bank, &[u32], usize); 5] = [
        (Bank::Fiq, &s.r_fiq, 8),
        (Bank::Supervisor, &s.r_svc, 13),
        (Bank::Abort, &s.r_abt, 13),
        (Bank::Irq, &s.r_irq, 13),
        (Bank::Undefined, &s.r_und, 13),
    ];
    let mut out: Vec<(Bank, usize, u32)> =
        (8..=14).map(|i| (Bank::UserSystem, i, s.r[i])).collect();
    banks.iter().for_each(|(bank, values, first)| {
        values
            .iter()
            .enumerate()
            .for_each(|(k, &v)| out.push((*bank, first + k, v)));
    });
    out
}

/// Modes after loading which some bank does not read back as the suite gave it.
fn modes_with_unread_banks() -> Vec<u32> {
    MODES
        .into_iter()
        .filter(|&mode| {
            let s = in_mode(mode);
            let cpu = loaded(&s);
            suite_registers(&s)
                .iter()
                .any(|&(bank, i, v)| cpu.regs().banked(bank, i) != v)
        })
        .collect()
}

/// Modes whose loaded state leaves R0–R7, R15, CPSR or an SPSR different from the suite's.
fn modes_with_unread_shared_state() -> Vec<u32> {
    MODES
        .into_iter()
        .filter(|&mode| {
            let s = in_mode(mode);
            let cpu = loaded(&s);
            let regs = cpu.regs();
            let spsr = [
                Bank::Fiq,
                Bank::Supervisor,
                Bank::Abort,
                Bank::Irq,
                Bank::Undefined,
            ]
            .map(|b| regs.spsr(b));
            let shared: Vec<u32> = (0..=7).chain([15]).map(|i| regs.get(i)).collect();
            let want: Vec<u32> = (0..=7).chain([15]).map(|i| s.r[i]).collect();
            shared != want || regs.cpsr() != s.cpsr || spsr != s.spsr.map(Some)
        })
        .collect()
}

/// Modes whose state does not survive a load followed by a step that does nothing.
fn modes_that_do_not_round_trip() -> Vec<u32> {
    MODES
        .into_iter()
        .filter(|&mode| {
            let mut c = case();
            c.initial = in_mode(mode);
            c.expected = in_mode(mode);
            !run_noop(&c).is_empty()
        })
        .collect()
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
fn given_a_case_with_three_transactions_when_decoding_then_all_come_back_in_order() {
    let mut c = case();
    c.transactions = vec![
        txn(0, 4, 0x100, 1),
        txn(1, 2, 0x200, 2),
        txn(2, 1, 0x300, 3),
    ];
    assert_eq!(decode(&encode_file(&[c.clone()])).unwrap(), vec![c]);
}

#[test]
fn given_a_wrong_magic_when_decoding_then_the_error_names_byte_zero() {
    let bytes = with_word(encode_file(&[case()]), 0, 0);
    let error = decode(&bytes).unwrap_err();
    assert_eq!(
        error.to_string(),
        "bad suite data at byte 0: the file does not start with the suite magic"
    );
}

#[test]
fn given_every_truncation_of_a_file_when_decoding_then_none_is_accepted() {
    let bytes = encode_file(&[case(), case()]);
    assert_eq!(prefixes_accepted(&bytes), Vec::<usize>::new());
}

#[test]
fn given_a_wrong_transactions_magic_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), TRANSACTIONS_AT + 4, 9);
    assert!(
        decode_error(&bytes).contains("wrong magic"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_state_block_of_the_wrong_size_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), INITIAL_AT, 172);
    assert!(
        decode_error(&bytes).contains("not 168 bytes"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_state_block_smaller_than_its_fixed_part_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), FINAL_AT, 4);
    assert!(
        decode_error(&bytes).contains("not 168 bytes"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_wrong_marker_in_the_initial_state_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), INITIAL_AT + 4, FINAL_MARKER);
    assert!(
        decode_error(&bytes).contains("wrong marker"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_wrong_marker_in_the_final_state_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), FINAL_AT + 4, INITIAL_MARKER);
    assert!(
        decode_error(&bytes).contains("wrong marker"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_transactions_size_that_disagrees_with_its_count_when_decoding_then_the_file_is_refused()
{
    let bytes = with_word(encode_file(&[case()]), TRANSACTIONS_AT, 12 + 24 * 2);
    assert!(
        decode_error(&bytes).contains("does not match its count"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_transaction_count_beyond_the_data_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), TRANSACTIONS_AT + 8, 1000);
    assert!(
        decode_error(&bytes).contains("does not match its count"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_transaction_count_that_overflows_its_size_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), TRANSACTIONS_AT + 8, u32::MAX);
    assert!(
        decode_error(&bytes).contains("does not match its count"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_an_opcode_block_of_the_wrong_size_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), OPCODE_AT, 20);
    assert!(
        decode_error(&bytes).contains("not 16 bytes"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_wrong_opcode_marker_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), OPCODE_AT + 4, 0);
    assert!(
        decode_error(&bytes).contains("wrong marker"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_case_size_larger_than_its_blocks_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), CASE_AT, 392 + 4);
    assert!(
        decode_error(&bytes).contains("does not match its blocks"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_case_size_smaller_than_its_blocks_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case()]), CASE_AT, 392 - 4);
    assert!(
        decode_error(&bytes).contains("does not match its blocks"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_byte_after_the_last_case_when_decoding_then_the_file_is_refused() {
    let mut bytes = encode_file(&[case()]);
    bytes.push(0);
    assert_eq!(
        decode_error(&bytes),
        format!(
            "bad suite data at byte {}: bytes follow the last case",
            bytes.len() - 1
        )
    );
}

#[test]
fn given_a_header_count_smaller_than_the_cases_present_when_decoding_then_the_file_is_refused() {
    let bytes = with_word(encode_file(&[case(), case()]), 4, 1);
    assert!(
        decode_error(&bytes).contains("bytes follow the last case"),
        "{}",
        decode_error(&bytes)
    );
}

#[test]
fn given_a_huge_case_count_and_no_data_when_decoding_then_the_error_is_a_read_past_byte_eight() {
    let mut bytes = Vec::new();
    push(&mut bytes, &[MAGIC, u32::MAX]);
    assert_eq!(
        decode_error(&bytes),
        "bad suite data at byte 8: the data ends inside a word"
    );
}

// --- replay bus ---

#[test]
fn given_the_base_address_when_fetching_then_the_opcode_is_returned() {
    let mut bus = ReplayBus::new(&case());
    assert_eq!(
        (bus.fetch32(0x100), bus.fetch16(0x100)),
        (0xE1A0_1234, 0x1234)
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

// --- loading a state: `R` holds the User/System registers, `R_<bank>` each bank's own ---

#[test]
fn given_a_user_mode_state_when_the_cpu_switches_to_fiq_then_r8_to_r14_are_the_fiq_registers() {
    let mut cpu = loaded(&in_mode(0x10));
    cpu.regs_mut().set_cpsr(0xC0 | 0x11);
    assert_eq!(visible(&cpu, 8..=14), state().r_fiq.to_vec());
}

#[test]
fn given_a_fiq_mode_state_when_loaded_then_the_visible_r8_to_r14_are_the_fiq_registers() {
    let cpu = loaded(&in_mode(0x11));
    assert_eq!(visible(&cpu, 8..=14), state().r_fiq.to_vec());
}

#[test]
fn given_a_fiq_mode_state_when_loaded_then_the_user_bank_holds_the_r_values() {
    let cpu = loaded(&in_mode(0x11));
    let user: Vec<u32> = (8..=14)
        .map(|i| cpu.regs().banked(Bank::UserSystem, i))
        .collect();
    assert_eq!(user, state().r[8..=14].to_vec());
}

#[test]
fn given_a_fiq_mode_state_when_the_cpu_switches_to_irq_then_r13_r14_are_irq_and_r8_to_r12_the_user_values()
 {
    let mut cpu = loaded(&in_mode(0x11));
    cpu.regs_mut().set_cpsr(0xC0 | 0x12);
    let seen = (visible(&cpu, 8..=12), visible(&cpu, 13..=14));
    assert_eq!(seen, (state().r[8..=12].to_vec(), state().r_irq.to_vec()));
}

#[test]
fn given_a_state_in_each_mode_when_loaded_then_every_bank_reads_back_as_the_suite_gave_it() {
    assert_eq!(modes_with_unread_banks(), Vec::<u32>::new());
}

#[test]
fn given_a_state_in_each_mode_when_loaded_then_r0_to_r7_pc_cpsr_and_spsrs_are_the_suite_values() {
    assert_eq!(modes_with_unread_shared_state(), Vec::<u32>::new());
}

#[test]
fn given_a_state_in_each_mode_when_running_a_step_that_does_nothing_then_no_diff_appears() {
    assert_eq!(modes_that_do_not_round_trip(), Vec::<u32>::new());
}

// --- harness, with steps that do not depend on Cpu::step ---

#[test]
fn given_a_case_that_expects_no_change_when_running_a_noop_step_then_there_are_no_diffs() {
    assert_eq!(run_noop(&case()), vec![]);
}

#[test]
fn given_the_arm9_flavor_when_running_a_noop_step_then_there_are_no_diffs() {
    assert_eq!(run_case_with::<true>(&case(), noop::<true>), vec![]);
}

#[test]
fn given_a_different_final_r0_when_running_a_noop_step_then_one_diff_names_r0_with_both_values() {
    let mut c = case();
    c.expected.r[0] = 0xFFFF_0000;
    assert_eq!(
        run_noop(&c),
        vec![FieldDiff {
            name: "R0".to_string(),
            expected: "0xffff0000".to_string(),
            actual: "0x00001000".to_string(),
        }]
    );
}

#[test]
fn given_a_step_that_writes_the_expected_r0_when_running_then_there_are_no_diffs() {
    let mut c = case();
    c.expected.r[0] = 0xBEEF;
    let diffs = run_case_with::<false>(&c, |cpu, _| cpu.regs_mut().set(0, 0xBEEF));
    assert_eq!(diffs, vec![]);
}

#[test]
fn given_a_step_that_switches_mode_and_writes_a_banked_register_when_running_then_it_matches() {
    let mut c = case();
    c.expected.cpsr = 0xD1;
    c.expected.r_fiq[0] = 0x77;
    let diffs = run_case_with::<false>(&c, |cpu, _| {
        cpu.regs_mut().set_cpsr(0xD1);
        cpu.regs_mut().set(8, 0x77);
    });
    assert_eq!(diffs, vec![]);
}

fn names(diffs: &[FieldDiff]) -> Vec<&str> {
    diffs.iter().map(|d| d.name.as_str()).collect()
}

#[test]
fn given_changes_to_banked_registers_cpsr_and_spsr_when_running_a_noop_step_then_each_is_named() {
    let mut c = case();
    c.expected.r_fiq[2] ^= 1;
    c.expected.r_svc[0] ^= 1;
    c.expected.r_abt[1] ^= 1;
    c.expected.r_irq[0] ^= 1;
    c.expected.r_und[1] ^= 1;
    c.expected.cpsr ^= 0x8000_0000;
    c.expected.spsr[4] ^= 1;
    assert_eq!(
        names(&run_noop(&c)),
        vec![
            "R_fiq[2]", "R_svc[0]", "R_abt[1]", "R_irq[0]", "R_und[1]", "CPSR", "SPSR[4]"
        ]
    );
}

#[test]
fn given_a_difference_in_the_user_bank_registers_when_running_a_noop_step_then_the_visible_slot_is_named()
 {
    let mut c = case();
    c.expected.r[12] ^= 1;
    c.expected.r[14] ^= 1;
    assert_eq!(names(&run_noop(&c)), vec!["R12", "R14"]);
}

#[test]
fn given_pipeline_and_access_values_that_differ_when_running_then_they_are_not_compared() {
    let mut c = case();
    c.expected.pipeline = [1, 2];
    c.expected.access = 99;
    assert_eq!(run_noop(&c), vec![]);
}

#[test]
fn given_a_step_that_writes_the_expected_data_when_running_then_the_transactions_match() {
    let mut c = case();
    c.transactions.push(txn(2, 4, 0x3000, 5));
    let diffs = run_case_with::<false>(&c, |_, bus| bus.write32(0x3000, 5));
    assert_eq!(diffs, vec![]);
}

#[test]
fn given_a_step_that_writes_other_data_when_running_then_the_transaction_diff_shows_both_sides() {
    let mut c = case();
    c.transactions.push(txn(2, 4, 0x3000, 5));
    let diffs = run_case_with::<false>(&c, |_, bus| bus.write32(0x3000, 6));
    assert_eq!(
        diffs,
        vec![FieldDiff {
            name: "transaction[0]".to_string(),
            expected: "write size=4 addr=0x00003000 data=0x00000005".to_string(),
            actual: "write size=4 addr=0x00003000 data=0x00000006".to_string(),
        }]
    );
}

#[test]
fn given_a_step_that_reads_a_recorded_value_when_running_then_the_read_matches() {
    let mut c = case();
    c.transactions.push(txn(1, 2, 0x10, 0xABCD));
    let diffs = run_case_with::<false>(&c, |cpu, bus| {
        let value = bus.read16(0x10);
        cpu.regs_mut().set(1, u32::from(value));
    });
    c.expected.r[1] = 0xABCD;
    let again = run_case_with::<false>(&c, |cpu, bus| {
        let value = bus.read16(0x10);
        cpu.regs_mut().set(1, u32::from(value));
    });
    assert_eq!((names(&diffs), again), (vec!["R1"], vec![]));
}

#[test]
fn given_a_step_that_makes_an_extra_access_when_running_then_the_extra_one_is_reported_against_none()
 {
    let diffs = run_case_with::<false>(&case(), |_, bus| bus.write8(0x40, 1));
    assert_eq!(
        diffs,
        vec![FieldDiff {
            name: "transaction[0]".to_string(),
            expected: "none".to_string(),
            actual: "write size=1 addr=0x00000040 data=0x00000001".to_string(),
        }]
    );
}

#[test]
fn given_a_data_write_the_step_never_made_when_running_then_the_transaction_is_reported_missing() {
    let mut c = case();
    c.transactions.push(txn(2, 4, 0x3000, 5));
    let diffs = run_noop(&c);
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
fn given_a_data_read_the_step_never_made_when_running_then_the_read_is_described() {
    let mut c = case();
    c.transactions.push(txn(1, 2, 0x10, 5));
    assert_eq!(
        run_noop(&c)[0].expected,
        "read size=2 addr=0x00000010 data=0x00000005"
    );
}

#[test]
fn given_a_fetch_transaction_only_when_running_then_it_is_not_a_data_diff() {
    let mut c = case();
    c.transactions = vec![txn(0, 4, 0x100, 1), txn(0, 4, 0x104, 2)];
    assert_eq!(run_noop(&c), vec![]);
}

#[test]
fn given_cases_when_running_them_then_only_failing_ones_are_returned_with_index_file_and_opcode() {
    let mut bad = case();
    bad.opcode = 0xDEAD_BEEF;
    bad.expected.r[1] = 0;
    let diffs = run_cases_with::<false>("x.json.bin", &[case(), bad, case()], noop::<false>);
    let summary: Vec<(String, usize, u32, usize)> = diffs
        .iter()
        .map(|d| (d.file.clone(), d.index, d.opcode, d.fields.len()))
        .collect();
    assert_eq!(summary, vec![("x.json.bin".to_string(), 1, 0xDEAD_BEEF, 1)]);
}

#[test]
fn given_a_suite_file_on_disk_when_running_it_then_the_failures_carry_its_file_name() {
    let dir = TempDir::new("one-file");
    let mut bad = case();
    bad.expected.cpsr = 0;
    std::fs::write(dir.join("a.json.bin"), encode_file(&[case(), bad])).unwrap();
    let diffs = run_json_suite_with::<false>(&dir.join("a.json.bin"), noop::<false>).unwrap();
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
    let dir = TempDir::new("summary");
    let mut bad = case();
    bad.expected.r[2] = 0;
    std::fs::write(dir.join("b.json.bin"), encode_file(&[case()])).unwrap();
    std::fs::write(dir.join("a.json.bin"), encode_file(&[case(), bad, case()])).unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    let lines: Vec<String> = summarize_dir_with(&dir, noop::<false>)
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
    assert_eq!(io_path(&error), Some("suite".to_string()));
}

#[test]
fn given_a_malformed_suite_file_in_the_directory_when_summarizing_then_the_error_is_a_format_error()
{
    let dir = TempDir::new("malformed");
    std::fs::write(dir.join("a.json.bin"), b"not a suite file").unwrap();
    let error = summarize_dir(&dir).unwrap_err();
    assert_eq!(io_path(&error), None);
}

#[test]
fn given_a_directory_entry_that_cannot_be_read_as_a_file_when_summarizing_then_the_error_names_it()
{
    let dir = TempDir::new("unreadable");
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
fn given_a_directory_of_suite_files_when_reporting_then_a_total_line_follows_the_files() {
    let dir = TempDir::new("report");
    let mut bad = case();
    bad.expected.r[2] = 0;
    std::fs::write(dir.join("a.json.bin"), encode_file(&[case(), bad])).unwrap();
    std::fs::write(dir.join("b.json.bin"), encode_file(&[case()])).unwrap();
    assert_eq!(
        report_dir_with(&dir, noop::<false>).unwrap(),
        "a.json.bin: 2 cases, 1 failed\nb.json.bin: 1 cases, 0 failed\ntotal: 3 cases, 1 failed"
    );
}

#[test]
fn given_an_empty_directory_when_reporting_then_only_the_zero_total_is_printed() {
    assert_eq!(
        report_dir(&TempDir::new("empty")).unwrap(),
        "total: 0 cases, 0 failed"
    );
}
