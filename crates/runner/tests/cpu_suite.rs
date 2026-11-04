// SPDX-License-Identifier: Apache-2.0
//! The fetched ARM7TDMI JSON suite, checked against the reader and pinning what the harness
//! assumes about its data. Needs `./scripts/fetch_cpu_suite.sh` (verify.sh runs it first).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ninge_runner::cpu_suite::{Case, State, Transaction, decode, run_json_suite};

/// Modes that bank registers, with the first banked register and the offset of the bank in
/// the suite's `R_*` order: FIQ banks R8–R14, the others R13–R14.
const EXCEPTION_MODES: [(u32, usize); 5] =
    [(0x11, 8), (0x12, 13), (0x13, 13), (0x17, 13), (0x1B, 13)];

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/arm7tdmi/v1")
}

/// The files `tests/cpu_suite/SHA256SUMS` lists, in order.
fn listed_files() -> Vec<String> {
    let sums = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cpu_suite/SHA256SUMS");
    std::fs::read_to_string(sums)
        .expect("tests/cpu_suite/SHA256SUMS is tracked")
        .lines()
        .map(|line| {
            line.split_whitespace()
                .nth(1)
                .expect("a name after the hash")
                .to_string()
        })
        .collect()
}

/// Every case of every listed file, decoded one file at a time and handed to `each`.
fn for_each_case(mut each: impl FnMut(&Case)) {
    for name in listed_files() {
        let bytes = std::fs::read(suite_dir().join(&name)).unwrap_or_else(|e| {
            panic!("{name} is missing ({e}) — run ./scripts/fetch_cpu_suite.sh")
        });
        decode(&bytes)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .iter()
            .for_each(&mut each);
    }
}

/// The registers of `state` that belong to the bank of `mode`, from the `R` slots and from
/// the bank's own array.
fn slots(state: &State, mode: u32) -> (Vec<u32>, Vec<u32>) {
    let (first, bank): (usize, &[u32]) = match mode {
        0x11 => (8, &state.r_fiq),
        0x12 => (13, &state.r_irq),
        0x13 => (13, &state.r_svc),
        0x17 => (13, &state.r_abt),
        _ => (13, &state.r_und),
    };
    (state.r[first..first + bank.len()].to_vec(), bank.to_vec())
}

/// What the suite's data says about `R` versus the bank arrays, per exception mode.
#[derive(Default)]
struct ModeStats {
    /// States (initial and final) whose CPSR names this mode.
    states: usize,
    /// Of those, states whose `R` slots equal the bank array.
    r_equals_bank: usize,
    /// Cases that stay in this mode: `R` slots that changed / bank slots that changed.
    r_slot_changes: usize,
    bank_slot_changes: usize,
}

fn mode_stats() -> BTreeMap<u32, ModeStats> {
    let mut stats: BTreeMap<u32, ModeStats> = EXCEPTION_MODES
        .iter()
        .map(|&(m, _)| (m, ModeStats::default()))
        .collect();
    for_each_case(|case| {
        for state in [&case.initial, &case.expected] {
            let mode = state.cpsr & 0x1F;
            if let Some(entry) = stats.get_mut(&mode) {
                let (r, bank) = slots(state, mode);
                entry.states += 1;
                entry.r_equals_bank += usize::from(r == bank);
            }
        }
        let mode = case.initial.cpsr & 0x1F;
        if mode == case.expected.cpsr & 0x1F
            && let Some(entry) = stats.get_mut(&mode)
        {
            let (r0, b0) = slots(&case.initial, mode);
            let (r1, b1) = slots(&case.expected, mode);
            entry.r_slot_changes += r0.iter().zip(&r1).filter(|(a, b)| a != b).count();
            entry.bank_slot_changes += b0.iter().zip(&b1).filter(|(a, b)| a != b).count();
        }
    });
    stats
}

/// Problems with the exact-block decoding of the listed files, as text.
fn decoding_problems() -> Vec<String> {
    let mut problems = Vec::new();
    let mut on_disk: Vec<String> = std::fs::read_dir(suite_dir())
        .expect("the suite is missing — run ./scripts/fetch_cpu_suite.sh")
        .map(|entry| {
            entry
                .expect("directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json.bin"))
        .collect();
    on_disk.sort();
    let mut listed = listed_files();
    listed.sort();
    if on_disk != listed {
        problems.push(format!(
            "{} files on disk, {} listed in SHA256SUMS",
            on_disk.len(),
            listed.len()
        ));
    }
    for name in &listed {
        let bytes = std::fs::read(suite_dir().join(name)).expect("suite file is readable");
        let header = u32::from_le_bytes(bytes[4..8].try_into().expect("header holds a count"));
        match decode(&bytes) {
            Ok(cases) if cases.len() == header as usize && !cases.is_empty() => {}
            Ok(cases) => problems.push(format!(
                "{name}: header says {header}, decoded {}",
                cases.len()
            )),
            Err(error) => problems.push(format!("{name}: {error}")),
        }
    }
    problems
}

/// Problems with the claim that `R` is not the active mode's view of the bank.
fn r_equals_bank_problems() -> Vec<String> {
    mode_stats()
        .iter()
        .flat_map(|(mode, s)| {
            let mut out = Vec::new();
            if s.states == 0 {
                out.push(format!("mode {mode:#x}: no states in the suite"));
            }
            if s.r_equals_bank != 0 {
                out.push(format!(
                    "mode {mode:#x}: {} of {} states have R equal to the bank",
                    s.r_equals_bank, s.states
                ));
            }
            out
        })
        .collect()
}

/// Problems with the claim that instructions change the bank slots, not the `R` slots.
fn bank_changes_problems() -> Vec<String> {
    mode_stats()
        .iter()
        .filter(|(_, s)| s.bank_slot_changes < 20 * s.r_slot_changes || s.bank_slot_changes == 0)
        .map(|(mode, s)| {
            format!(
                "mode {mode:#x}: bank slots changed {} times, R slots {}",
                s.bank_slot_changes, s.r_slot_changes
            )
        })
        .collect()
}

/// Problems with the structure of a harness run over one real file.
fn run_problems() -> Vec<String> {
    let name = "arm_bx.json.bin";
    let cases = decode(&std::fs::read(suite_dir().join(name)).expect("suite file is readable"))
        .expect("the file decodes")
        .len();
    let diffs = run_json_suite::<false>(&suite_dir().join(name)).expect("the file runs");
    let mut problems: Vec<String> = diffs
        .iter()
        .filter(|d| d.file != name || d.index >= cases || d.fields.is_empty())
        .map(|d| format!("bad diff for case {}", d.index))
        .collect();
    if diffs.windows(2).any(|w| w[0].index >= w[1].index) {
        problems.push("diffs are not in increasing case order".to_string());
    }
    problems
}

#[test]
fn given_fetched_suite_when_decoded_then_every_byte_is_consumed_by_exact_blocks() {
    assert_eq!(decoding_problems(), Vec::<String>::new());
}

#[test]
fn given_fetched_suite_states_when_grouped_by_mode_then_no_exception_mode_state_has_r_equal_to_its_bank_array()
 {
    assert_eq!(r_equals_bank_problems(), Vec::<String>::new());
}

#[test]
fn given_fetched_suite_cases_when_comparing_slot_changes_then_bank_slots_change_far_more_often_than_r_slots()
 {
    assert_eq!(bank_changes_problems(), Vec::<String>::new());
}

#[test]
fn given_a_fetched_suite_file_when_run_then_every_diff_names_the_file_and_a_case_index_in_range() {
    assert_eq!(run_problems(), Vec::<String>::new());
}

/// Runs one suite file on the ARM7 and returns the cases that differ; a file that cannot be
/// read is a panic, not an empty list.
fn arm7_diffs(file: &str) -> Vec<ninge_runner::cpu_suite::CaseDiff> {
    run_json_suite::<false>(&suite_dir().join(file))
        .unwrap_or_else(|e| panic!("{file}: {e} — run ./scripts/fetch_cpu_suite.sh"))
}

#[test]
fn given_arm_data_proc_immediate_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_data_proc_immediate.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_data_proc_immediate_shift_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_data_proc_immediate_shift.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_data_proc_register_shift_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_data_proc_register_shift.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_mrs_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_mrs.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_msr_imm_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_msr_imm.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_msr_reg_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_msr_reg.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

/// Decoded cases of one suite file.
fn cases_of(file: &str) -> Vec<Case> {
    decode(&std::fs::read(suite_dir().join(file)).unwrap_or_else(|e| panic!("{file}: {e}")))
        .unwrap_or_else(|e| panic!("{file}: {e}"))
}

/// Whether the instruction's condition holds for the case's starting flags.
fn condition_passes(case: &Case) -> bool {
    let cpsr = case.initial.cpsr;
    let (n, z, c, v) = (
        cpsr >> 31 & 1 == 1,
        cpsr >> 30 & 1 == 1,
        cpsr >> 29 & 1 == 1,
        cpsr >> 28 & 1 == 1,
    );
    match case.opcode >> 28 {
        0 => z,
        1 => !z,
        2 => c,
        3 => !c,
        4 => n,
        5 => !n,
        6 => v,
        7 => !v,
        8 => c && !z,
        9 => !c || z,
        10 => n == v,
        11 => n != v,
        12 => !z && n == v,
        13 => z || n != v,
        _ => true,
    }
}

fn is_user_or_system(case: &Case) -> bool {
    matches!(case.initial.cpsr & 0x1F, 0x10 | 0x1F)
}

/// Problems with "every case starts with R15 = fetch address + 8" for the six #8 files.
fn start_r15_problems() -> Vec<String> {
    SUITE_FILES_8
        .iter()
        .flat_map(|file| {
            cases_of(file)
                .into_iter()
                .enumerate()
                .filter(|(_, c)| c.initial.r[15] != c.base_addr.wrapping_add(8))
                .map(move |(i, _)| format!("{file}[{i}]: R15 is not base + 8"))
        })
        .collect()
}

const SUITE_FILES_8: [&str; 6] = [
    "arm_data_proc_immediate.json.bin",
    "arm_data_proc_immediate_shift.json.bin",
    "arm_data_proc_register_shift.json.bin",
    "arm_mrs.json.bin",
    "arm_msr_imm.json.bin",
    "arm_msr_reg.json.bin",
];

/// MRS CPSR cases that pass their condition with Rd = R15 end with R15 = CPSR + 4, an
/// ordinary register write plus the normal advance.
fn mrs_rd_pc_problems() -> Vec<String> {
    let cases: Vec<Case> = cases_of("arm_mrs.json.bin")
        .into_iter()
        .filter(|c| condition_passes(c) && c.opcode >> 12 & 0xF == 15 && c.opcode >> 22 & 1 == 0)
        .collect();
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| c.expected.r[15] != c.initial.cpsr.wrapping_add(4))
        .map(|c| format!("{:#010x}: R15 is not the CPSR + 4", c.opcode))
        .collect();
    problems.extend((cases.len() < 500).then(|| format!("only {} cases", cases.len())));
    problems
}

/// MRS SPSR in User/System with Rd < 8 (no banking in the way) reads the CPSR.
fn mrs_spsr_user_problems() -> Vec<String> {
    let cases: Vec<Case> = cases_of("arm_mrs.json.bin")
        .into_iter()
        .filter(|c| {
            condition_passes(c)
                && is_user_or_system(c)
                && c.opcode >> 22 & 1 == 1
                && c.opcode >> 12 & 0xF < 8
        })
        .collect();
    let wrong: Vec<&Case> = cases
        .iter()
        .filter(|c| c.expected.r[(c.opcode >> 12 & 0xF) as usize] != c.initial.cpsr)
        .collect();
    let mut problems: Vec<String> = (!wrong.is_empty())
        .then(|| {
            format!(
                "{} of {} cases: Rd is not the CPSR",
                wrong.len(),
                cases.len()
            )
        })
        .into_iter()
        .collect();
    problems.extend(wrong.iter().take(5).map(|c| format!("{:#010x}", c.opcode)));
    problems.extend((cases.len() < 1000).then(|| format!("only {} cases", cases.len())));
    problems
}

/// MSR CPSR cases that end with mode bits naming no mode must leave every register slot but R15
/// and every SPSR as it was, which is what keeping the bank means (#76).
fn invalid_mode_msr_problems() -> Vec<String> {
    let valid = [0x10, 0x11, 0x12, 0x13, 0x17, 0x1B, 0x1F];
    let cases: Vec<Case> = cases_of("arm_msr_reg.json.bin")
        .into_iter()
        .filter(|c| !valid.contains(&(c.expected.cpsr & 0x1F)))
        .collect();
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| {
            let (a, b) = (&c.initial, &c.expected);
            a.r[..15] != b.r[..15]
                || a.r_fiq != b.r_fiq
                || a.r_svc != b.r_svc
                || a.r_abt != b.r_abt
                || a.r_irq != b.r_irq
                || a.r_und != b.r_und
                || a.spsr != b.spsr
        })
        .map(|c| format!("{:#010x}: a register changed", c.opcode))
        .collect();
    problems.extend((cases.len() < 1000).then(|| format!("only {} cases", cases.len())));
    problems
}

/// S with Rd = R15 in User/System keeps the control byte of the CPSR.
fn user_s_pc_problems() -> Vec<String> {
    let cases: Vec<(&str, Case)> = SUITE_FILES_8[..3]
        .iter()
        .flat_map(|file| cases_of(file).into_iter().map(move |c| (*file, c)))
        .filter(|(_, c)| {
            condition_passes(c)
                && is_user_or_system(c)
                && c.opcode >> 20 & 1 == 1
                && c.opcode >> 12 & 0xF == 15
        })
        .collect();
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|(_, c)| c.expected.cpsr & 0xFF != c.initial.cpsr & 0xFF)
        .map(|(file, c)| format!("{file} {:#010x}: control byte changed", c.opcode))
        .collect();
    problems.extend((cases.len() < 3000).then(|| format!("only {} cases", cases.len())));
    problems
}

/// After MSR to the CPSR from User mode, the control byte is unchanged.
fn user_msr_problems() -> Vec<String> {
    let cases: Vec<(&str, Case)> = ["arm_msr_imm.json.bin", "arm_msr_reg.json.bin"]
        .iter()
        .flat_map(|file| cases_of(file).into_iter().map(move |c| (*file, c)))
        .filter(|(_, c)| {
            condition_passes(c) && c.initial.cpsr & 0x1F == 0x10 && c.opcode >> 22 & 1 == 0
        })
        .collect();
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|(_, c)| c.expected.cpsr & 0xFF != c.initial.cpsr & 0xFF)
        .map(|(file, c)| format!("{file} {:#010x}: control byte changed", c.opcode))
        .collect();
    problems.extend((cases.len() < 3000).then(|| format!("only {} cases", cases.len())));
    problems
}

/// MSR to the CPSR control byte from a privileged mode, Rm < 8, with a value whose bit 4 is
/// clear: the result must still have M4 set.
fn m4_forced_problems() -> Vec<String> {
    let cases: Vec<Case> = cases_of("arm_msr_reg.json.bin")
        .into_iter()
        .filter(|c| {
            condition_passes(c)
                && !is_user_mode(c)
                && c.opcode >> 22 & 1 == 0
                && c.opcode >> 16 & 1 == 1
                && c.opcode & 0xF < 8
                && c.initial.r[(c.opcode & 0xF) as usize] & 0x10 == 0
        })
        .collect();
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| c.expected.cpsr & 0x10 == 0)
        .map(|c| format!("{:#010x}: M4 is clear after the write", c.opcode))
        .collect();
    problems.extend((cases.len() < 500).then(|| format!("only {} cases", cases.len())));
    problems
}

/// Whether the case starts in User mode proper (System is privileged).
fn is_user_mode(case: &Case) -> bool {
    case.initial.cpsr & 0x1F == 0x10
}

/// The barrel shifter on its own, as the notes describe it.
fn reference_shift(kind: u32, value: u32, amount: u32) -> u32 {
    match kind {
        0 => value.checked_shl(amount).unwrap_or(0),
        1 => value.checked_shr(amount).unwrap_or(0),
        2 => ((value as i32) >> amount.min(31)) as u32,
        _ => value.rotate_right(amount % 32),
    }
}

/// MOV/MVN with a register shift whose amount register is R15, Rm and Rd below 8: the amount is
/// the low byte of the +8 view (the start R15), and the +12 view would give a different result in
/// some of the cases.
fn rs_pc_problems() -> Vec<String> {
    let cases: Vec<Case> = cases_of("arm_data_proc_register_shift.json.bin")
        .into_iter()
        .filter(|c| {
            condition_passes(c)
                && c.opcode >> 8 & 0xF == 15
                && c.opcode & 0xF < 8
                && c.opcode >> 12 & 0xF < 8
                && matches!(c.opcode >> 21 & 0xF, 13 | 15)
        })
        .collect();
    let result = |c: &Case, r15: u32| {
        let shifted = reference_shift(
            c.opcode >> 5 & 3,
            c.initial.r[(c.opcode & 0xF) as usize],
            r15 & 0xFF,
        );
        shifted
            ^ match c.opcode >> 21 & 0xF {
                15 => u32::MAX,
                _ => 0,
            }
    };
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| c.expected.r[(c.opcode >> 12 & 0xF) as usize] != result(c, c.initial.r[15]))
        .map(|c| format!("{:#010x}: Rd differs from the +8 view", c.opcode))
        .collect();
    let discriminating = cases
        .iter()
        .filter(|c| result(c, c.initial.r[15]) != result(c, c.initial.r[15].wrapping_add(4)))
        .count();
    problems.extend((cases.len() < 20).then(|| format!("only {} cases", cases.len())));
    problems
        .extend((discriminating < 5).then(|| format!("only {discriminating} tell +8 from +12")));
    problems
}

#[test]
fn given_the_six_arm_8_files_when_reading_start_states_then_r15_is_the_fetch_address_plus_8() {
    assert_eq!(start_r15_problems(), Vec::<String>::new());
}

#[test]
fn given_mrs_cases_with_rd_pc_when_comparing_then_r15_is_the_value_plus_4() {
    assert_eq!(mrs_rd_pc_problems(), Vec::<String>::new());
}

#[test]
fn given_mrs_spsr_in_user_or_system_when_comparing_then_rd_holds_the_cpsr() {
    assert_eq!(mrs_spsr_user_problems(), Vec::<String>::new());
}

#[test]
fn given_msr_cases_ending_in_an_invalid_mode_when_comparing_then_no_register_or_spsr_changed() {
    assert_eq!(invalid_mode_msr_problems(), Vec::<String>::new());
}

#[test]
fn given_s_with_rd_pc_in_user_or_system_when_comparing_then_the_control_byte_is_kept() {
    assert_eq!(user_s_pc_problems(), Vec::<String>::new());
}

#[test]
fn given_msr_cpsr_in_user_mode_when_comparing_then_the_control_byte_is_kept() {
    assert_eq!(user_msr_problems(), Vec::<String>::new());
}

#[test]
fn given_msr_cpsr_control_writes_with_bit_4_clear_when_comparing_then_m4_is_set_in_the_result() {
    assert_eq!(m4_forced_problems(), Vec::<String>::new());
}

#[test]
fn given_register_shift_with_rs_r15_when_comparing_then_the_amount_is_the_plus_8_view() {
    assert_eq!(rs_pc_problems(), Vec::<String>::new());
}

/// Whether `diff` is exactly the known ARM7 multiply-carry gap: the only difference is the CPSR,
/// it is in bit 29 alone, and the case sets flags with a passing condition.
fn is_c_only_gap(diff: &ninge_runner::cpu_suite::CaseDiff, cases: &[Case]) -> bool {
    let case = &cases[diff.index];
    let cpsr_bits = |text: &str| u32::from_str_radix(text.trim_start_matches("0x"), 16);
    match diff.fields.as_slice() {
        [field] => {
            field.name == "CPSR"
                && cpsr_bits(&field.expected)
                    .ok()
                    .zip(cpsr_bits(&field.actual).ok())
                    .is_some_and(|(want, got)| want ^ got == 1 << 29)
                && condition_passes(case)
                && case.opcode >> 20 & 1 == 1
        }
        _ => false,
    }
}

/// Condition-passing S cases whose expected C differs from their initial C: the most a CPU that
/// leaves C unchanged can get wrong.
fn expected_c_changes(cases: &[Case]) -> usize {
    cases
        .iter()
        .filter(|c| condition_passes(c) && c.opcode >> 20 & 1 == 1)
        .filter(|c| (c.initial.cpsr ^ c.expected.cpsr) >> 29 & 1 == 1)
        .count()
}

/// Problems with a multiply file run on the ARM7: every diff that is not the C-only gap, and a
/// C-only count that does not equal the number of S cases whose C the suite changes.
fn multiply_gap_problems(file: &str) -> Vec<String> {
    let cases = cases_of(file);
    let diffs = arm7_diffs(file);
    let others: Vec<String> = diffs
        .iter()
        .filter(|d| !is_c_only_gap(d, &cases))
        .map(|d| format!("{file}[{}] {:#010x}: {:?}", d.index, d.opcode, d.fields))
        .collect();
    let c_only = diffs.len() - others.len();
    let want = expected_c_changes(&cases);
    others
        .into_iter()
        .chain((c_only != want).then(|| format!("{file}: {c_only} C-only diffs, {want} expected")))
        .collect()
}

/// The ARM7 C after an S multiply is the gap of #81: the CPU leaves C unchanged, the suite
/// changes it in about half of the cases. When #81 closes, this becomes
/// `given_arm_mul_mla_when_run_on_arm7_then_no_case_differs`.
#[test]
fn given_arm_mul_mla_when_run_on_arm7_then_only_the_c_bit_of_s_cases_differs() {
    assert_eq!(
        multiply_gap_problems("arm_mul_mla.json.bin"),
        Vec::<String>::new()
    );
}

/// The ARM7 C after an S long multiply is the gap of #81 (see above). When it closes, this
/// becomes `given_arm_mull_mlal_when_run_on_arm7_then_no_case_differs`.
#[test]
fn given_arm_mull_mlal_when_run_on_arm7_then_only_the_c_bit_of_s_cases_differs() {
    assert_eq!(
        multiply_gap_problems("arm_mull_mlal.json.bin"),
        Vec::<String>::new()
    );
}

/// Problems with "an R15 operand reads base + 12": the multiply cases whose Rm is R15 with a
/// passing condition and Rs, Rd below 8 hold product = (base + 12) * Rs.
fn multiply_r15_operand_problems() -> Vec<String> {
    let cases: Vec<Case> = cases_of("arm_mul_mla.json.bin")
        .into_iter()
        .filter(|c| {
            condition_passes(c)
                && c.opcode & 0xF == 15
                && c.opcode >> 21 & 1 == 0
                && c.opcode >> 8 & 0xF < 8
                && c.opcode >> 16 & 0xF < 8
        })
        .collect();
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| {
            let rs = c.initial.r[(c.opcode >> 8 & 0xF) as usize];
            c.expected.r[(c.opcode >> 16 & 0xF) as usize]
                != c.base_addr.wrapping_add(12).wrapping_mul(rs)
        })
        .map(|c| format!("{:#010x}: not (base + 12) * Rs", c.opcode))
        .collect();
    problems.extend((cases.len() < 20).then(|| format!("only {} cases", cases.len())));
    problems
}

/// Problems with "RdHi = RdLo keeps Hi": passing UMULL/SMULL cases with RdHi = RdLo end with
/// that register holding the high half of the 64-bit product.
fn long_multiply_equal_halves_problems() -> Vec<String> {
    let cases: Vec<Case> = cases_of("arm_mull_mlal.json.bin")
        .into_iter()
        .filter(|c| {
            condition_passes(c)
                && c.opcode >> 21 & 1 == 0
                && c.opcode >> 16 & 0xF == c.opcode >> 12 & 0xF
                && c.opcode >> 16 & 0xF < 8
                && c.opcode & 0xF < 8
                && c.opcode >> 8 & 0xF < 8
        })
        .collect();
    let hi = |c: &Case| {
        let (a, b) = (
            c.initial.r[(c.opcode & 0xF) as usize],
            c.initial.r[(c.opcode >> 8 & 0xF) as usize],
        );
        match c.opcode >> 22 & 1 {
            1 => ((i64::from(a as i32) * i64::from(b as i32)) >> 32) as u32,
            _ => ((u64::from(a) * u64::from(b)) >> 32) as u32,
        }
    };
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| c.expected.r[(c.opcode >> 16 & 0xF) as usize] != hi(c))
        .map(|c| format!("{:#010x}: the register is not the high half", c.opcode))
        .collect();
    problems.extend((cases.len() < 20).then(|| format!("only {} cases", cases.len())));
    problems
}

#[test]
fn given_mul_cases_with_rm_r15_when_comparing_then_the_operand_is_the_base_plus_12() {
    assert_eq!(multiply_r15_operand_problems(), Vec::<String>::new());
}

#[test]
fn given_long_multiplies_with_rdhi_equal_rdlo_when_comparing_then_the_register_holds_the_high_half()
{
    assert_eq!(long_multiply_equal_halves_problems(), Vec::<String>::new());
}

// ---- #10 loads and stores: facts measured on the pinned suite (MIT) ------------------------
//
// Each pins one fact `docs/hw/arm-load-store.md` cites as "measured". The cases are the ones
// whose condition passes and that run in User/System mode, so `r` is the visible register file.

/// Cases of `file` that execute, run in User/System mode, and satisfy `keep`.
fn executed_cases(file: &str, keep: impl Fn(&Case) -> bool) -> Vec<Case> {
    cases_of(file)
        .into_iter()
        .filter(|c| condition_passes(c) && is_user_or_system(c) && keep(c))
        .collect()
}

/// The data transactions of a case as (kind, size, address, data): kind 1 read, 2 write.
fn data_transactions(case: &Case) -> Vec<(u32, u32, u32, u32)> {
    case.transactions
        .iter()
        .filter(|t| t.kind != 0)
        .map(|t| (t.kind, t.size, t.addr, t.data))
        .collect()
}

/// A guard so a fact cannot pass by matching no case: at least `least` cases were checked.
fn too_few(label: &str, checked: usize, least: usize) -> Option<String> {
    (checked < least).then(|| format!("{label}: only {checked} cases, want {least}"))
}

/// The effective address of a single or halfword transfer with an immediate or register
/// offset (Rm < 15); `None` for forms this helper does not model.
fn transfer_address(case: &Case, halfword: bool) -> Option<u32> {
    let o = case.opcode;
    let base = case.initial.r[(o >> 16 & 0xF) as usize];
    let offset = match (halfword, o >> 22 & 1 == 1, o >> 25 & 1 == 1) {
        (false, _, false) => o & 0xFFF,
        (true, true, _) => (o >> 4 & 0xF0) | (o & 0xF),
        (true, false, _) if o & 0xF < 15 => case.initial.r[(o & 0xF) as usize],
        _ => return None,
    };
    let moved = match o >> 23 & 1 {
        1 => base.wrapping_add(offset),
        _ => base.wrapping_sub(offset),
    };
    Some(match o >> 24 & 1 {
        1 => moved,
        _ => base, // post-indexed: the access uses the unmodified base
    })
}

/// Problems with "the transaction address is the raw effective address, low bits included".
fn raw_address_problems(file: &str, halfword: bool, unaligned_size: u32) -> Vec<String> {
    let cases = executed_cases(file, |c| c.opcode >> 16 & 0xF != 15);
    let mut problems = Vec::new();
    let mut unaligned = 0;
    for case in &cases {
        let (Some(ea), Some(&(_, size, addr, _))) = (
            transfer_address(case, halfword),
            data_transactions(case).first(),
        ) else {
            continue;
        };
        unaligned += usize::from(size == unaligned_size && ea % size != 0);
        if addr != ea {
            problems.push(format!(
                "{file} {:#010x}: {addr:#x} is not {ea:#x}",
                case.opcode
            ));
        }
    }
    problems.extend(too_few(file, unaligned, 20));
    problems
}

#[test]
fn given_unaligned_ldr_and_str_when_comparing_then_the_transaction_address_is_the_raw_address() {
    assert_eq!(
        raw_address_problems("arm_ldr_str_immediate_offset.json.bin", false, 4),
        Vec::<String>::new()
    );
}

#[test]
fn given_unaligned_ldrh_and_strh_when_comparing_then_the_transaction_address_is_the_raw_address() {
    assert_eq!(
        raw_address_problems("arm_ldrh_strh.json.bin", true, 2),
        Vec::<String>::new()
    );
}

#[test]
fn given_unaligned_ldrsh_when_comparing_then_the_transaction_address_is_the_raw_address() {
    assert_eq!(
        raw_address_problems("arm_ldrsb_ldrsh.json.bin", true, 2),
        Vec::<String>::new()
    );
}

/// Rd after a load, for a case that is not a load into its own written-back base.
fn loaded_register(case: &Case) -> u32 {
    case.expected.r[(case.opcode >> 12 & 0xF) as usize]
}

/// A load whose Rd is neither R15 nor a base that the same instruction writes back.
fn plain_load(c: &Case) -> bool {
    let o = c.opcode;
    let (rn, rd) = (o >> 16 & 0xF, o >> 12 & 0xF);
    o >> 20 & 1 == 1 && rn != 15 && rd < 15 && !(rn == rd && (o >> 21 & 1 == 1 || o >> 24 & 1 == 0))
}

/// Problems with "Rd = the read data rotated right by 8 × (address & 3)" for LDR (and zero
/// extended for LDRB), where the recorded read is at the raw address.
fn ldr_rotation_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldr_str_immediate_offset.json.bin", |c| {
        plain_load(c) && c.opcode >> 25 & 1 == 0
    });
    let mut problems = Vec::new();
    let mut rotated = 0;
    for case in &cases {
        let (Some(ea), Some(&(_, size, _, data))) = (
            transfer_address(case, false),
            data_transactions(case).first(),
        ) else {
            continue;
        };
        let want = match size {
            4 => data.rotate_right(8 * (ea & 3)),
            _ => data,
        };
        rotated += usize::from(size == 4 && ea & 3 != 0);
        if loaded_register(case) != want {
            problems.push(format!("{:#010x}: Rd is not {want:#x}", case.opcode));
        }
    }
    problems.extend(too_few("LDR rotation", rotated, 20));
    problems
}

/// Problems with "LDRH from an odd address gives the halfword rotated right by 8; LDRSH from an
/// odd address gives the sign-extended byte at that address (the high byte of the halfword)".
fn halfword_load_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldrsb_ldrsh.json.bin", plain_load)
        .into_iter()
        .chain(executed_cases("arm_ldrh_strh.json.bin", plain_load));
    let mut problems = Vec::new();
    let mut odd = [0usize; 2];
    for case in cases {
        let (Some(ea), Some(&(_, size, _, data))) = (
            transfer_address(&case, true),
            data_transactions(&case).first(),
        ) else {
            continue;
        };
        let (kind, odd_address) = (case.opcode >> 5 & 3, ea & 1 == 1);
        let want = match (kind, size, odd_address) {
            (1, _, false) => data,
            (1, _, true) => data.rotate_right(8),
            (2, _, _) => i32::from(data as u8 as i8) as u32,
            (3, _, false) => i32::from(data as u16 as i16) as u32,
            _ => i32::from((data >> 8) as u8 as i8) as u32,
        };
        odd[0] += usize::from(kind == 1 && odd_address);
        odd[1] += usize::from(kind == 3 && odd_address);
        if loaded_register(&case) != want {
            problems.push(format!("{:#010x}: Rd is not {want:#x}", case.opcode));
        }
    }
    problems.extend(too_few("LDRH odd", odd[0], 20));
    problems.extend(too_few("LDRSH odd", odd[1], 20));
    problems
}

#[test]
fn given_unaligned_ldr_when_comparing_then_rd_is_the_read_data_rotated_right_by_8_times_the_low_bits()
 {
    assert_eq!(ldr_rotation_problems(), Vec::<String>::new());
}

#[test]
fn given_odd_address_ldrh_and_ldrsh_when_comparing_then_rd_is_the_rotated_halfword_and_the_sign_extended_byte()
 {
    assert_eq!(halfword_load_problems(), Vec::<String>::new());
}

/// Problems with "a store writes Rd unrotated": the word, the low halfword, or the low byte.
fn store_data_problems() -> Vec<String> {
    let mut problems = Vec::new();
    let mut unaligned = 0;
    for (file, halfword) in [
        ("arm_ldr_str_immediate_offset.json.bin", false),
        ("arm_ldrh_strh.json.bin", true),
    ] {
        let cases = executed_cases(file, |c| {
            let o = c.opcode;
            o >> 20 & 1 == 0 && o >> 16 & 0xF != 15 && o >> 12 & 0xF != 15
        });
        for case in &cases {
            let (Some(ea), Some(&(_, size, _, data))) = (
                transfer_address(case, halfword),
                data_transactions(case).first(),
            ) else {
                continue;
            };
            let rd = case.initial.r[(case.opcode >> 12 & 0xF) as usize];
            let want = match size {
                1 => rd & 0xFF,
                2 => rd & 0xFFFF,
                _ => rd,
            };
            unaligned += usize::from(size > 1 && ea % size != 0);
            if data != want {
                problems.push(format!(
                    "{file} {:#010x}: wrote {data:#x}, not {want:#x}",
                    case.opcode
                ));
            }
        }
    }
    problems.extend(too_few("unaligned stores", unaligned, 40));
    problems
}

#[test]
fn given_unaligned_str_and_strh_when_comparing_then_the_written_data_is_rd_unrotated() {
    assert_eq!(store_data_problems(), Vec::<String>::new());
}

/// The first address of an LDM/STM run for base `base` and `n` registers (P and U from the opcode).
fn block_start(opcode: u32, base: u32, n: u32) -> u32 {
    match (opcode >> 24 & 1, opcode >> 23 & 1) {
        (0, 1) => base,
        (1, 1) => base.wrapping_add(4),
        (0, _) => base.wrapping_sub(4 * n).wrapping_add(4),
        _ => base.wrapping_sub(4 * n),
    }
}

/// Problems with "block transfers use raw addresses: register i is at start + 4i with the low
/// bits of the base kept, and writeback adds or subtracts 4n to the raw base".
fn block_address_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldm_stm.json.bin", |c| {
        let rn = (c.opcode >> 16 & 0xF) as usize;
        c.opcode & 0xFFFF != 0 && rn != 15 && c.opcode >> 15 & 1 == 0
    });
    let mut problems = Vec::new();
    let (mut unaligned, mut written_back) = (0, 0);
    for case in &cases {
        let o = case.opcode;
        let (rn, n) = ((o >> 16 & 0xF) as usize, (o & 0xFFFF).count_ones());
        let base = case.initial.r[rn];
        let start = block_start(o, base, n);
        let addresses: Vec<u32> = data_transactions(case).iter().map(|t| t.2).collect();
        let want: Vec<u32> = (0..n).map(|i| start.wrapping_add(4 * i)).collect();
        unaligned += usize::from(base & 3 != 0);
        if addresses != want {
            problems.push(format!(
                "{o:#010x}: addresses {addresses:x?}, not {want:x?}"
            ));
        }
        let moved = match o >> 23 & 1 {
            1 => base.wrapping_add(4 * n),
            _ => base.wrapping_sub(4 * n),
        };
        let in_list = o >> rn & 1 == 1;
        written_back += usize::from(o >> 21 & 1 == 1 && !in_list);
        if o >> 21 & 1 == 1 && !in_list && case.expected.r[rn] != moved {
            problems.push(format!("{o:#010x}: base is not {moved:#x}"));
        }
    }
    problems.extend(too_few("block unaligned base", unaligned, 100));
    problems.extend(too_few("block writeback", written_back, 100));
    problems
}

#[test]
fn given_ldm_and_stm_with_an_unaligned_base_when_comparing_then_every_address_and_the_writeback_are_raw()
 {
    assert_eq!(block_address_problems(), Vec::<String>::new());
}

/// Problems with "SWP reads the raw address, rotates the word, and writes Rm unrotated (the
/// low byte for SWPB) to the same raw address".
fn swap_problems() -> Vec<String> {
    let cases = executed_cases("arm_swp.json.bin", |c| {
        let o = c.opcode;
        o >> 16 & 0xF != 15 && o >> 12 & 0xF != 15 && o & 0xF != 15
    });
    let mut problems = Vec::new();
    let mut unaligned = 0;
    for case in &cases {
        let o = case.opcode;
        let ea = case.initial.r[(o >> 16 & 0xF) as usize];
        let transactions = data_transactions(case);
        let ((_, _, read_at, read), (_, _, write_at, written)) = (transactions[0], transactions[1]);
        let byte = o >> 22 & 1 == 1;
        let rm = case.initial.r[(o & 0xF) as usize];
        let (want_rd, want_write) = match byte {
            true => (read, rm & 0xFF),
            false => (read.rotate_right(8 * (ea & 3)), rm),
        };
        unaligned += usize::from(!byte && ea & 3 != 0);
        let ok = transactions.len() == 2
            && (read_at, write_at) == (ea, ea)
            && written == want_write
            && loaded_register(case) == want_rd;
        if !ok {
            problems.push(format!(
                "{o:#010x}: not (raw address, rotated read, unrotated write)"
            ));
        }
    }
    problems.extend(too_few("SWP unaligned", unaligned, 100));
    problems
}

#[test]
fn given_swp_with_an_unaligned_address_when_comparing_then_both_accesses_are_raw_and_only_the_read_is_rotated()
 {
    assert_eq!(swap_problems(), Vec::<String>::new());
}

/// Problems with "R15 as a base reads address + 8, as a stored value reads address + 12, and a
/// load into R15 ends with R15 = value + 8 (the low bits kept)".
fn single_r15_problems() -> Vec<String> {
    let file = "arm_ldr_str_immediate_offset.json.bin";
    let mut problems = Vec::new();
    let as_base = executed_cases(file, |c| {
        c.opcode >> 16 & 0xF == 15 && c.opcode >> 25 & 1 == 0
    });
    let stored = executed_cases(file, |c| {
        c.opcode >> 20 & 1 == 0 && c.opcode >> 12 & 0xF == 15 && c.opcode >> 16 & 0xF != 15
    });
    let loaded = executed_cases(file, |c| {
        c.opcode >> 20 & 1 == 1
            && c.opcode >> 22 & 1 == 0
            && c.opcode >> 12 & 0xF == 15
            && c.opcode >> 16 & 0xF != 15
            && c.opcode >> 25 & 1 == 0
            && c.opcode >> 24 & 1 == 1
            && c.opcode >> 21 & 1 == 0
    });
    for case in &as_base {
        let o = case.opcode;
        let base = case.base_addr.wrapping_add(8);
        let off = o & 0xFFF;
        let moved = match o >> 23 & 1 {
            1 => base.wrapping_add(off),
            _ => base.wrapping_sub(off),
        };
        let want = match o >> 24 & 1 {
            1 => moved,
            _ => base,
        };
        let got = data_transactions(case)[0].2;
        if got != want {
            problems.push(format!("{o:#010x}: base access at {got:#x}, not {want:#x}"));
        }
    }
    for case in &stored {
        let got = data_transactions(case)[0].3;
        let want = case.base_addr.wrapping_add(12);
        let want = match case.opcode >> 22 & 1 {
            1 => want & 0xFF,
            _ => want,
        };
        if got != want {
            problems.push(format!(
                "{:#010x}: stored {got:#x}, not {want:#x}",
                case.opcode
            ));
        }
    }
    for case in &loaded {
        let o = case.opcode;
        let ea = transfer_address(case, false).expect("immediate form");
        let value = data_transactions(case)[0].3.rotate_right(8 * (ea & 3));
        if case.expected.r[15] != value.wrapping_add(8) {
            problems.push(format!("{o:#010x}: R15 is not {value:#x} + 8"));
        }
    }
    problems.extend(too_few("R15 base", as_base.len(), 100));
    problems.extend(too_few("R15 stored", stored.len(), 100));
    problems.extend(too_few("R15 loaded", loaded.len(), 20));
    problems
}

#[test]
fn given_single_transfers_with_r15_when_comparing_then_base_reads_plus_8_store_reads_plus_12_and_a_load_ends_at_value_plus_8()
 {
    assert_eq!(single_r15_problems(), Vec::<String>::new());
}

/// Problems with "a load into its own written-back base keeps the loaded value".
fn load_into_base_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldr_str_immediate_offset.json.bin", |c| {
        let o = c.opcode;
        let rn = o >> 16 & 0xF;
        o >> 20 & 1 == 1
            && o >> 22 & 1 == 0
            && o >> 25 & 1 == 0
            && rn == o >> 12 & 0xF
            && rn < 15
            && (o >> 21 & 1 == 1 || o >> 24 & 1 == 0)
    });
    let problems: Vec<String> = cases
        .iter()
        .filter(|c| {
            let ea = transfer_address(c, false).expect("immediate form");
            let value = data_transactions(c)[0].3.rotate_right(8 * (ea & 3));
            loaded_register(c) != value
        })
        .map(|c| format!("{:#010x}: Rd = Rn did not keep the loaded value", c.opcode))
        .collect();
    problems
        .into_iter()
        .chain(too_few("load into base", cases.len(), 50))
        .collect()
}

#[test]
fn given_ldr_with_rd_equal_to_a_written_back_rn_when_comparing_then_the_loaded_value_wins() {
    assert_eq!(load_into_base_problems(), Vec::<String>::new());
}

/// Problems with the base register in the list: LDM writes back nothing (the loaded value
/// stays); STM stores the old base when it is the first register and the new base otherwise.
fn base_in_list_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldm_stm.json.bin", |c| {
        let o = c.opcode;
        let rn = (o >> 16 & 0xF) as usize;
        rn < 15 && o >> 21 & 1 == 1 && o >> 22 & 1 == 0 && o >> rn & 1 == 1 && o >> 15 & 1 == 0
    });
    let mut problems = Vec::new();
    let (mut loads, mut first, mut later) = (0, 0, 0);
    for case in &cases {
        let o = case.opcode;
        let rn = (o >> 16 & 0xF) as usize;
        let n = (o & 0xFFFF).count_ones();
        let base = case.initial.r[rn];
        let moved = match o >> 23 & 1 {
            1 => base.wrapping_add(4 * n),
            _ => base.wrapping_sub(4 * n),
        };
        let index = (o & ((1 << rn) - 1)).count_ones() as usize;
        let transaction = data_transactions(case)[index];
        match o >> 20 & 1 {
            1 => {
                loads += 1;
                if case.expected.r[rn] != transaction.3 {
                    problems.push(format!("{o:#010x}: LDM base is not the loaded value"));
                }
            }
            _ => {
                let is_first = index == 0;
                first += usize::from(is_first);
                later += usize::from(!is_first);
                let want = if is_first { base } else { moved };
                if transaction.3 != want || case.expected.r[rn] != moved {
                    problems.push(format!(
                        "{o:#010x}: STM stored {:#x}, not {want:#x}",
                        transaction.3
                    ));
                }
            }
        }
    }
    problems.extend(too_few("LDM base in list", loads, 100));
    problems.extend(too_few("STM base first", first, 20));
    problems.extend(too_few("STM base later", later, 100));
    problems
}

#[test]
fn given_ldm_and_stm_with_the_base_in_the_list_when_comparing_then_ldm_keeps_the_loaded_value_and_stm_stores_old_only_when_first()
 {
    assert_eq!(base_in_list_problems(), Vec::<String>::new());
}

/// Problems with "STM and LDM with R15 in the list: R15 is stored as address + 12, and an LDM
/// that loads it ends with R15 = value + 8 (the low bits kept)".
fn block_r15_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldm_stm.json.bin", |c| {
        let o = c.opcode;
        o >> 15 & 1 == 1 && o >> 16 & 0xF != 15
    });
    let mut problems = Vec::new();
    let (mut stores, mut loads) = (0, 0);
    for case in &cases {
        let o = case.opcode;
        let n = (o & 0xFFFF).count_ones() as usize;
        let last = data_transactions(case)[n - 1];
        match o >> 20 & 1 {
            0 => {
                stores += 1;
                if last.3 != case.base_addr.wrapping_add(12) {
                    problems.push(format!("{o:#010x}: stored R15 {:#x}", last.3));
                }
            }
            _ => {
                loads += 1;
                if case.expected.r[15] != last.3.wrapping_add(8) {
                    problems.push(format!("{o:#010x}: R15 is not the loaded value + 8"));
                }
            }
        }
    }
    problems.extend(too_few("STM R15", stores, 100));
    problems.extend(too_few("LDM R15", loads, 100));
    problems
}

#[test]
fn given_block_transfers_with_r15_in_the_list_when_comparing_then_stm_stores_plus_12_and_ldm_ends_at_value_plus_8()
 {
    assert_eq!(block_r15_problems(), Vec::<String>::new());
}

/// Problems with "STRH with Rd = R15 stores address + 12 (the low halfword)".
fn strh_r15_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldrh_strh.json.bin", |c| {
        let o = c.opcode;
        o >> 20 & 1 == 0 && o >> 5 & 3 == 1 && o >> 12 & 0xF == 15 && o >> 16 & 0xF != 15
    });
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| data_transactions(c)[0].3 != c.base_addr.wrapping_add(12) & 0xFFFF)
        .map(|c| format!("{:#010x}: STRH R15 did not store address + 12", c.opcode))
        .collect();
    problems.extend(too_few("STRH R15", cases.len(), 100));
    problems
}

#[test]
fn given_strh_with_rd_r15_when_comparing_then_the_stored_halfword_is_the_address_plus_12() {
    assert_eq!(strh_r15_problems(), Vec::<String>::new());
}

/// Problems with "a post-indexed halfword transfer with bit 21 set (W) writes back like any
/// post-indexed form": Rn ends as the base plus or minus the immediate.
fn halfword_post_index_w_problems() -> Vec<String> {
    let mut problems = Vec::new();
    for file in ["arm_ldrh_strh.json.bin", "arm_ldrsb_ldrsh.json.bin"] {
        let cases = executed_cases(file, |c| {
            let o = c.opcode;
            let (rn, rd) = (o >> 16 & 0xF, o >> 12 & 0xF);
            o >> 24 & 1 == 0 && o >> 21 & 1 == 1 && o >> 22 & 1 == 1 && rn < 15 && rd != rn
        });
        problems.extend(
            cases
                .iter()
                .filter(|c| {
                    let o = c.opcode;
                    let rn = (o >> 16 & 0xF) as usize;
                    let offset = (o >> 4 & 0xF0) | (o & 0xF);
                    let moved = match o >> 23 & 1 {
                        1 => c.initial.r[rn].wrapping_add(offset),
                        _ => c.initial.r[rn].wrapping_sub(offset),
                    };
                    c.expected.r[rn] != moved
                })
                .map(|c| format!("{file} {:#010x}: P = 0, W = 1 did not write back", c.opcode)),
        );
        problems.extend(too_few(file, cases.len(), 100));
    }
    problems
}

#[test]
fn given_halfword_transfers_with_p_0_and_w_1_when_comparing_then_rn_is_written_back_as_post_indexing_does()
 {
    assert_eq!(halfword_post_index_w_problems(), Vec::<String>::new());
}

/// Problems with "SWP reads an R15 Rn as address + 12 and stores an R15 Rm as address + 12".
fn swap_r15_problems() -> Vec<String> {
    let file = "arm_swp.json.bin";
    let as_base = executed_cases(file, |c| {
        c.opcode >> 16 & 0xF == 15 && c.opcode >> 12 & 0xF != 15 && c.opcode & 0xF != 15
    });
    let as_source = executed_cases(file, |c| {
        c.opcode & 0xF == 15 && c.opcode >> 16 & 0xF != 15 && c.opcode >> 12 & 0xF != 15
    });
    let mut problems: Vec<String> = as_base
        .iter()
        .filter(|c| data_transactions(c)[0].2 != c.base_addr.wrapping_add(12))
        .map(|c| format!("{:#010x}: Rn = R15 did not read address + 12", c.opcode))
        .collect();
    problems.extend(
        as_source
            .iter()
            .filter(|c| {
                let want = c.base_addr.wrapping_add(12);
                let want = match c.opcode >> 22 & 1 {
                    1 => want & 0xFF,
                    _ => want,
                };
                data_transactions(c)[1].3 != want
            })
            .map(|c| format!("{:#010x}: Rm = R15 did not store address + 12", c.opcode)),
    );
    problems.extend(too_few("SWP Rn = R15", as_base.len(), 100));
    problems.extend(too_few("SWP Rm = R15", as_source.len(), 100));
    problems
}

#[test]
fn given_swp_with_r15_operands_when_comparing_then_both_read_as_the_address_plus_12() {
    assert_eq!(swap_r15_problems(), Vec::<String>::new());
}

/// Problems with "writeback to R15: a single transfer ends at the moved address + 12, a block
/// transfer at the moved address + 8 (the base reads address + 8 in both)".
fn r15_writeback_problems() -> Vec<String> {
    let single = executed_cases("arm_ldr_str_immediate_offset.json.bin", |c| {
        let o = c.opcode;
        o >> 25 & 1 == 0
            && o >> 16 & 0xF == 15
            && o >> 12 & 0xF != 15
            && (o >> 21 & 1 == 1 || o >> 24 & 1 == 0)
    });
    let block = executed_cases("arm_ldm_stm.json.bin", |c| {
        let o = c.opcode;
        o >> 16 & 0xF == 15 && o >> 21 & 1 == 1 && o & 0xFFFF != 0 && o >> 15 & 1 == 0
    });
    let moved = |base: u32, up: bool, bytes: u32| match up {
        true => base.wrapping_add(bytes),
        false => base.wrapping_sub(bytes),
    };
    let mut problems: Vec<String> = single
        .iter()
        .filter(|c| {
            let base = c.base_addr.wrapping_add(8);
            let target = moved(base, c.opcode >> 23 & 1 == 1, c.opcode & 0xFFF);
            c.expected.r[15] != target.wrapping_add(12)
        })
        .map(|c| format!("{:#010x}: single writeback to R15", c.opcode))
        .collect();
    problems.extend(
        block
            .iter()
            .filter(|c| {
                let base = c.base_addr.wrapping_add(8);
                let bytes = 4 * (c.opcode & 0xFFFF).count_ones();
                let target = moved(base, c.opcode >> 23 & 1 == 1, bytes);
                c.expected.r[15] != target.wrapping_add(8)
            })
            .map(|c| format!("{:#010x}: block writeback to R15", c.opcode)),
    );
    problems.extend(too_few("single writeback to R15", single.len(), 100));
    problems.extend(too_few("block writeback to R15", block.len(), 100));
    problems
}

#[test]
fn given_transfers_whose_written_back_rn_is_r15_when_comparing_then_single_ends_at_plus_12_and_block_at_plus_8()
 {
    assert_eq!(r15_writeback_problems(), Vec::<String>::new());
}

/// The base register `rn` (13 or 14, banked in every exception mode) as `state` sees it in the
/// exception mode its CPSR names.
fn exception_bank_register(state: &State, rn: usize) -> u32 {
    match state.cpsr & 0x1F {
        0x11 => state.r_fiq[rn - 8],
        0x12 => state.r_irq[rn - 13],
        0x13 => state.r_svc[rn - 13],
        0x17 => state.r_abt[rn - 13],
        _ => state.r_und[rn - 13],
    }
}

/// Problems with "STM with S and W in an exception mode reads the base from the current mode's
/// register and writes the moved base to the User-bank register".
fn user_bank_writeback_problems() -> Vec<String> {
    let cases: Vec<Case> = cases_of("arm_ldm_stm.json.bin")
        .into_iter()
        .filter(|c| {
            let o = c.opcode;
            let rn = o >> 16 & 0xF;
            condition_passes(c)
                && matches!(c.initial.cpsr & 0x1F, 0x11 | 0x12 | 0x13 | 0x17 | 0x1B)
                && o >> 20 & 1 == 0
                && o >> 22 & 1 == 1
                && o >> 21 & 1 == 1
                && (13..15).contains(&rn)
                && o >> 15 & 1 == 0
                && o & 0xFFFF != 0
        })
        .collect();
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| {
            let o = c.opcode;
            let rn = (o >> 16 & 0xF) as usize;
            let bytes = 4 * (o & 0xFFFF).count_ones();
            let base = exception_bank_register(&c.initial, rn);
            let moved = match o >> 23 & 1 {
                1 => base.wrapping_add(bytes),
                _ => base.wrapping_sub(bytes),
            };
            c.expected.r[rn] != moved || exception_bank_register(&c.expected, rn) != base
        })
        .map(|c| format!("{:#010x}: S + W did not write the User-bank Rn", c.opcode))
        .collect();
    problems.extend(too_few("STM S + W", cases.len(), 100));
    problems
}

#[test]
fn given_stm_with_s_and_w_in_an_exception_mode_when_comparing_then_the_user_bank_rn_gets_the_moved_base()
 {
    assert_eq!(user_bank_writeback_problems(), Vec::<String>::new());
}

/// Problems with "LDM with R15 and S in User/System (no SPSR) loads R15 and leaves the CPSR".
fn ldm_pc_without_spsr_problems() -> Vec<String> {
    let cases = executed_cases("arm_ldm_stm.json.bin", |c| {
        let o = c.opcode;
        o >> 20 & 1 == 1 && o >> 22 & 1 == 1 && o >> 15 & 1 == 1 && o >> 16 & 0xF != 15
    });
    let mut problems: Vec<String> = cases
        .iter()
        .filter(|c| c.expected.cpsr != c.initial.cpsr)
        .map(|c| format!("{:#010x}: the CPSR changed", c.opcode))
        .collect();
    problems.extend(too_few("LDM^ {PC} in User/System", cases.len(), 100));
    problems
}

#[test]
fn given_ldm_with_r15_and_s_in_user_or_system_when_comparing_then_the_cpsr_is_unchanged() {
    assert_eq!(ldm_pc_without_spsr_problems(), Vec::<String>::new());
}

#[test]
fn given_arm_ldr_str_immediate_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_ldr_str_immediate_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_ldr_str_register_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_ldr_str_register_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_ldrh_strh_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_ldrh_strh.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_ldrsb_ldrsh_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_ldrsb_ldrsh.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_ldm_stm_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_ldm_stm.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_swp_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_swp.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

// ---- The prefetch pipeline (#11, `docs/hw/arm-branch-pipeline.md`) -------------------------
//
// Oracle evidence, not hardware evidence: each fact below is what the suite shows and is listed
// under "Unverified" in the note where GBATEK is silent.

/// The ARM files the CPU executes: the data-processing, PSR, multiply, transfer, swap, branch
/// and BX groups.
const PIPELINE_FILES: [&str; 16] = [
    "arm_data_proc_immediate.json.bin",
    "arm_data_proc_immediate_shift.json.bin",
    "arm_data_proc_register_shift.json.bin",
    "arm_mrs.json.bin",
    "arm_msr_imm.json.bin",
    "arm_msr_reg.json.bin",
    "arm_mul_mla.json.bin",
    "arm_mull_mlal.json.bin",
    "arm_ldr_str_immediate_offset.json.bin",
    "arm_ldr_str_register_offset.json.bin",
    "arm_ldrh_strh.json.bin",
    "arm_ldrsb_ldrsh.json.bin",
    "arm_ldm_stm.json.bin",
    "arm_swp.json.bin",
    "arm_b_bl.json.bin",
    "arm_bx.json.bin",
];

/// The code fetches of a case, in order.
fn fetches(case: &Case) -> Vec<Transaction> {
    case.transactions
        .iter()
        .copied()
        .filter(|t| t.kind == 0)
        .collect()
}

/// Every case of `PIPELINE_FILES`, labeled `file[index]`.
fn pipeline_cases() -> Vec<(String, Case)> {
    PIPELINE_FILES
        .iter()
        .flat_map(|file| {
            cases_of(file)
                .into_iter()
                .enumerate()
                .map(move |(i, c)| (format!("{file}[{i}]"), c))
        })
        .collect()
}

/// Labels of the cases that `keep` selects and `holds` rejects, plus a guard that enough cases
/// were checked.
fn fact_problems(
    label: &str,
    cases: &[(String, Case)],
    keep: impl Fn(&Case) -> bool,
    holds: impl Fn(&Case) -> bool,
    least: usize,
) -> Vec<String> {
    let kept: Vec<&(String, Case)> = cases.iter().filter(|(_, c)| keep(c)).collect();
    let mut problems: Vec<String> = kept
        .iter()
        .filter(|(_, c)| !holds(c))
        .map(|(name, _)| format!("{label}: {name}"))
        .collect();
    problems.extend(too_few(label, kept.len(), least));
    problems
}

/// Whether the fetch pair after the first is `t & !3` and `(t & !3) + 4` and the final state is
/// the pipeline those two fetches fill, with R15 = t + 8 for the `t` whose low bits R15 keeps.
fn refilled_as_arm(c: &Case) -> bool {
    let f = fetches(c);
    let r15 = c.expected.r[15];
    f.len() == 3
        && f[1].size == 4
        && f[2].size == 4
        && f[1].addr & 3 == 0
        && f[2].addr == f[1].addr.wrapping_add(4)
        && c.expected.pipeline == [f[1].data, f[2].data]
        && r15.wrapping_sub(f[1].addr) >> 2 == 2
}

fn plain_step(c: &Case) -> bool {
    let f = fetches(c);
    f.len() == 1
        && c.expected.pipeline == [c.initial.pipeline[1], f[0].data]
        && c.expected.r[15] == c.initial.r[15].wrapping_add(4)
}

/// Fact 1: `pipeline[0]` is the opcode and R15 is the base + 8, in every case.
fn start_state_problems() -> Vec<String> {
    fact_problems(
        "start",
        &pipeline_cases(),
        |_| true,
        |c| c.initial.pipeline[0] == c.opcode && c.initial.r[15] == c.base_addr.wrapping_add(8),
        800_000,
    )
}

#[test]
fn given_every_executed_arm_file_when_reading_start_states_then_pipeline_0_is_the_opcode_and_r15_is_base_plus_8()
 {
    assert_eq!(start_state_problems(), Vec::<String>::new());
}

/// Fact 2: the first transaction is a word fetch at R15 (base + 8); after the data accesses
/// only the two refill fetches can follow.
fn fetch_order_problems() -> Vec<String> {
    let shape = |c: &Case| {
        let kinds: Vec<u32> = c.transactions.iter().map(|t| t.kind).collect();
        let data_end = kinds.iter().rposition(|&k| k != 0).map_or(1, |i| i + 1);
        let tail = &kinds[data_end..];
        let head_is_fetch = kinds[0] == 0 && kinds[1..data_end].iter().all(|&k| k != 0);
        head_is_fetch && (tail.is_empty() || tail == [0, 0])
    };
    fact_problems(
        "fetch order",
        &pipeline_cases(),
        |_| true,
        |c| {
            let first = c.transactions[0];
            shape(c) && first.size == 4 && first.addr == c.base_addr.wrapping_add(8)
        },
        800_000,
    )
}

#[test]
fn given_every_executed_arm_file_when_listing_transactions_then_a_fetch_at_base_plus_8_comes_before_any_data_access()
 {
    assert_eq!(fetch_order_problems(), Vec::<String>::new());
}

/// Fact 3: a failed condition, and an ALU result not written to R15, shift the pipeline by one
/// fetched word and advance R15 by 4.
fn plain_advance_problems() -> Vec<String> {
    let mut problems = fact_problems(
        "failed condition",
        &pipeline_cases(),
        |c| !condition_passes(c),
        |c| plain_step(c) && c.transactions.len() == 1,
        100_000,
    );
    let alu = cases_of_prefixed("arm_data_proc");
    problems.extend(fact_problems(
        "ALU, Rd not R15",
        &alu,
        |c| condition_passes(c) && c.opcode >> 12 & 0xF != 15,
        plain_step,
        50_000,
    ));
    problems
}

#[test]
fn given_failed_conditions_and_alu_results_outside_r15_when_comparing_then_the_pipeline_shifts_and_r15_advances_by_4()
 {
    assert_eq!(plain_advance_problems(), Vec::<String>::new());
}

/// A PC-write group: its label, the file-name prefix, which cases belong, and the fewest cases
/// the suite must supply (about 80% of what was measured).
type PcWriteGroup = (&'static str, &'static str, fn(&Case) -> bool, usize);

/// Fact 4: a write to the PC fetches twice more, at `target & !3` and `+ 4`; the pipeline holds
/// those two words and R15 is target + 8 with the low bits kept (R15 − first fetch is 8..=11).
/// The cases: ALU Rd = 15 (not a compare), LDR into the PC, LDM with R15 in the list, multiply
/// Rd = 15, long multiply RdHi or RdLo = 15, SWP Rd = 15, B and BL. Cases that end in Thumb
/// state are left to fact 5. Every minimum case count in this section is about 80% of what the
/// pinned suite supplies, so a selector that goes wrong for part of the cases still fails.
fn pc_write_problems() -> Vec<String> {
    let flushes = |c: &Case| condition_passes(c) && c.expected.cpsr >> 5 & 1 == 0;
    let groups: [PcWriteGroup; 7] = [
        (
            "ALU",
            "arm_data_proc",
            |c| c.opcode >> 12 & 0xF == 15 && !(8..=11).contains(&(c.opcode >> 21 & 0xF)),
            2_750,
        ),
        (
            "LDR PC",
            "arm_ldr_str",
            |c| c.opcode >> 20 & 1 == 1 && c.opcode >> 12 & 0xF == 15,
            1_280,
        ),
        (
            "LDM PC",
            "arm_ldm_stm",
            |c| c.opcode >> 20 & 1 == 1 && c.opcode >> 15 & 1 == 1,
            5_000,
        ),
        (
            "MUL PC",
            "arm_mul_mla",
            |c| c.opcode >> 16 & 0xF == 15,
            1_280,
        ),
        (
            "MULL PC",
            "arm_mull_mlal",
            |c| c.opcode >> 16 & 0xF == 15 || c.opcode >> 12 & 0xF == 15,
            2_360,
        ),
        ("SWP PC", "arm_swp", |c| c.opcode >> 12 & 0xF == 15, 1_250),
        ("B, BL", "arm_b_bl", |_| true, 20_000),
    ];
    groups
        .iter()
        .flat_map(|&(label, prefix, select, least)| {
            let cases = cases_of_prefixed(prefix);
            fact_problems(
                label,
                &cases,
                |c| flushes(c) && select(c),
                |c| {
                    let f = fetches(c);
                    refilled_as_arm(c) && f[0].addr == c.base_addr.wrapping_add(8)
                },
                least,
            )
        })
        .collect()
}

#[test]
fn given_pc_writes_when_comparing_then_two_more_fetches_follow_at_the_aligned_target_and_r15_is_target_plus_8()
 {
    assert_eq!(pc_write_problems(), Vec::<String>::new());
}

/// B and BL land exactly on R15 (base + 8) + the sign-extended word offset; BL's LR is base + 4.
fn branch_target_problems() -> Vec<String> {
    let cases = cases_of_prefixed("arm_b_bl");
    let target = |c: &Case| {
        let offset = (((c.opcode & 0x00FF_FFFF) << 8) as i32 >> 6) as u32; // sign-extend, then << 2
        c.base_addr.wrapping_add(8).wrapping_add(offset)
    };
    let mut problems = fact_problems(
        "branch target",
        &cases,
        condition_passes,
        |c| {
            c.expected.r[15] == target(c).wrapping_add(8)
                && fetches(c).get(1).map(|t| t.addr) == Some(target(c))
        },
        20_000,
    );
    problems.extend(fact_problems(
        "BL link",
        &cases,
        |c| condition_passes(c) && c.opcode >> 24 & 1 == 1 && c.initial.cpsr & 0x1F == 0x1F,
        |c| c.expected.r[14] == c.base_addr.wrapping_add(4),
        1_240,
    ));
    problems
}

#[test]
fn given_b_and_bl_when_comparing_then_the_target_is_r15_plus_the_word_offset_and_bl_links_the_next_instruction()
 {
    assert_eq!(branch_target_problems(), Vec::<String>::new());
}

/// Fact 5: BX with bit 0 set fetches a word, then two halfwords at `t & !1` and `+ 2`; T is
/// set and R15 is `(t & !1) + 4`. The halfword is the low half of the address' word.
fn bx_thumb_problems() -> Vec<String> {
    let cases = cases_of_prefixed("arm_bx");
    fact_problems(
        "BX to Thumb",
        &cases,
        |c| condition_passes(c) && c.opcode & 0xF < 8 && c.expected.cpsr >> 5 & 1 == 1,
        |c| {
            let f = fetches(c);
            f.len() == 3
                && f[1].addr == c.initial.r[(c.opcode & 0xF) as usize] & !1
                && (f[1].size, f[2].size) == (2, 2)
                && f[1].addr & 1 == 0
                && f[2].addr == f[1].addr.wrapping_add(2)
                && c.expected.r[15] == f[1].addr.wrapping_add(4)
                && c.expected.pipeline == [f[1].data, f[2].data]
                && f[1].data == f[1].addr & 0xFFFF
        },
        4_900,
    )
}

/// The labeled cases of the `PIPELINE_FILES` whose names start with `prefix`; only those files
/// are decoded.
fn cases_of_prefixed(prefix: &str) -> Vec<(String, Case)> {
    PIPELINE_FILES
        .iter()
        .filter(|file| file.starts_with(prefix))
        .flat_map(|file| {
            cases_of(file)
                .into_iter()
                .enumerate()
                .map(move |(i, c)| (format!("{file}[{i}]"), c))
        })
        .collect()
}

#[test]
fn given_bx_to_thumb_when_comparing_then_it_fetches_32_16_16_sets_t_and_r15_is_the_halfword_target_plus_4()
 {
    assert_eq!(bx_thumb_problems(), Vec::<String>::new());
}

/// Fact 6: BX to ARM with bit 1 set fetches word-aligned but R15 = target + 8 stays unaligned
/// (R15 − first fetch = 10). Rn is restricted to R0–R7, which are the same in every mode.
fn bx_arm_bit_1_problems() -> Vec<String> {
    let target = |c: &Case| c.initial.r[(c.opcode & 0xF) as usize];
    fact_problems(
        "BX to ARM with bit 1",
        &cases_of_prefixed("arm_bx"),
        |c| condition_passes(c) && c.opcode & 0xF < 8 && target(c) & 3 == 2,
        |c| {
            let f = fetches(c);
            refilled_as_arm(c)
                && c.expected.r[15] == target(c).wrapping_add(8)
                && f[1].addr == target(c) & !3
        },
        2_500,
    )
}

#[test]
fn given_bx_to_arm_with_bit_1_set_when_comparing_then_fetches_are_word_aligned_and_r15_keeps_the_target_plus_8()
 {
    assert_eq!(bx_arm_bit_1_problems(), Vec::<String>::new());
}

/// Fact 7: MRS with Rd = R15 is not a flush: one fetch, the pipeline shifts, R15 advances by 4
/// from the value written.
fn mrs_no_flush_problems() -> Vec<String> {
    fact_problems(
        "MRS Rd = R15",
        &cases_of_prefixed("arm_mrs"),
        |c| condition_passes(c) && c.opcode >> 12 & 0xF == 15,
        |c| {
            let f = fetches(c);
            f.len() == 1
                && c.transactions.len() == 1
                && c.expected.pipeline == [c.initial.pipeline[1], f[0].data]
        },
        1_260,
    )
}

#[test]
fn given_mrs_with_rd_r15_when_comparing_then_nothing_flushes_and_one_fetch_is_made() {
    assert_eq!(mrs_no_flush_problems(), Vec::<String>::new());
}

/// Fact 8: a single or halfword transfer (immediate offset) that writes back to Rn = R15 (no PC
/// load) flushes to the moved address + 4 (so R15 = moved + 12); a block transfer flushes to the
/// moved address (R15 = moved + 8).
fn r15_writeback_flush_problems() -> Vec<String> {
    let moved = |base: u32, up: bool, bytes: u32| match up {
        true => base.wrapping_add(bytes),
        false => base.wrapping_sub(bytes),
    };
    let writes_back = |o: u32| o >> 21 & 1 == 1 || o >> 24 & 1 == 0;
    let single = |c: &Case| {
        c.opcode >> 16 & 0xF == 15 && c.opcode >> 12 & 0xF != 15 && writes_back(c.opcode)
    };
    // The moved base of a single (12-bit offset) or halfword (split 8-bit offset) transfer.
    let single_moved = |c: &Case, halfword: bool| {
        let o = c.opcode;
        let offset = match halfword {
            true => (o >> 4 & 0xF0) | (o & 0xF),
            false => o & 0xFFF,
        };
        moved(c.base_addr.wrapping_add(8), o >> 23 & 1 == 1, offset)
    };
    let single_holds = |c: &Case, halfword: bool| {
        let target = single_moved(c, halfword).wrapping_add(4);
        let f = fetches(c);
        refilled_as_arm(c) && f[1].addr == target & !3 && c.expected.r[15] == target.wrapping_add(8)
    };
    let block = |c: &Case| {
        let o = c.opcode;
        o >> 16 & 0xF == 15 && o >> 21 & 1 == 1 && o & 0xFFFF != 0 && o >> 15 & 1 == 0
    };
    let mut problems = fact_problems(
        "single writeback to R15",
        &cases_of_prefixed("arm_ldr_str_immediate_offset"),
        |c| condition_passes(c) && single(c),
        |c| single_holds(c, false),
        920,
    );
    problems.extend(fact_problems(
        "halfword writeback to R15",
        &cases_of_prefixed("arm_ldrh_strh"),
        |c| condition_passes(c) && single(c) && c.opcode >> 22 & 1 == 1,
        |c| single_holds(c, true),
        430,
    ));
    problems.extend(fact_problems(
        "block writeback to R15",
        &cases_of_prefixed("arm_ldm_stm"),
        |c| condition_passes(c) && block(c),
        |c| {
            let bytes = 4 * (c.opcode & 0xFFFF).count_ones();
            let target = moved(c.base_addr.wrapping_add(8), c.opcode >> 23 & 1 == 1, bytes);
            let f = fetches(c);
            refilled_as_arm(c)
                && f[1].addr == target & !3
                && c.expected.r[15] == target.wrapping_add(8)
        },
        305,
    ));
    problems
}

#[test]
fn given_transfers_writing_back_to_r15_when_comparing_then_single_flushes_to_moved_plus_4_and_block_to_moved()
 {
    assert_eq!(r15_writeback_flush_problems(), Vec::<String>::new());
}

#[test]
fn given_arm_b_bl_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_b_bl.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_bx_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_bx.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

// ---- Exceptions (#12, `docs/hw/arm-exceptions-modes.md`) -----------------------------------
//
// Oracle evidence for the ARM7, not hardware evidence: SWI enters Supervisor mode through vector
// 08h, and every coprocessor instruction (CDP, MCR/MRC, LDC/STC, any coprocessor number) enters
// Undefined mode through vector 04h. Gap in GBATEK for the coprocessors: it does not say what an
// absent coprocessor does. Minimum case counts are about 80% of what the pinned suite supplies.

/// An exception suite file with the SPSR index and mode bits of the mode it enters, and its
/// vector offset.
type ExceptionFile = (&'static str, usize, u32, u32);

const SWI_FILE: ExceptionFile = ("arm_swi.json.bin", 1, 0x13, 0x08);
const COPROCESSOR_FILES: [ExceptionFile; 3] = [
    ("arm_cdp.json.bin", 4, 0x1B, 0x04),
    ("arm_mcr_mrc.json.bin", 4, 0x1B, 0x04),
    ("arm_stc_ldc.json.bin", 4, 0x1B, 0x04),
];

fn labeled(file: &str) -> Vec<(String, Case)> {
    cases_of(file)
        .into_iter()
        .enumerate()
        .map(|(i, c)| (format!("{file}[{i}]"), c))
        .collect()
}

/// R14 of the entered mode: the suite lists R14 as the second element of a two-register bank.
fn entered_r14(state: &State, mode: u32) -> u32 {
    match mode {
        0x13 => state.r_svc[1],
        _ => state.r_und[1],
    }
}

/// The CPSR an entry into `mode` leaves: bits 31–8 kept, T cleared, I set, F kept.
fn entered_cpsr(initial: u32, mode: u32) -> u32 {
    (initial & !0xFF) | 0x80 | (initial & 0x40) | mode
}

/// Whether the entry left alone every User/System register (R0–R14), every bank but the
/// entered one, the entered bank's R13, and the SPSRs of the other modes. `spsr` is the index of
/// the entered mode's SPSR.
fn rest_untouched(c: &Case, mode: u32, spsr: usize) -> bool {
    let (i, e) = (&c.initial, &c.expected);
    e.r[..15] == i.r[..15]
        && e.r_fiq == i.r_fiq
        && e.r_irq == i.r_irq
        && e.r_abt == i.r_abt
        && match mode {
            0x13 => e.r_und == i.r_und && e.r_svc[0] == i.r_svc[0],
            _ => e.r_svc == i.r_svc && e.r_und[0] == i.r_und[0],
        }
        && (0..5).all(|k| k == spsr || e.spsr[k] == i.spsr[k])
}

/// The facts every entering case of `file` shows: the CPSR, R14 and the SPSR of the entered
/// mode, the three fetches, the refilled pipeline and R15, and the registers left alone.
fn entry_problems(
    label: &str,
    (file, spsr, mode, vector): ExceptionFile,
    least: usize,
) -> Vec<String> {
    fact_problems(
        label,
        &labeled(file),
        condition_passes,
        |c| {
            let f = fetches(c);
            c.expected.cpsr == entered_cpsr(c.initial.cpsr, mode)
                && entered_r14(&c.expected, mode) == c.base_addr.wrapping_add(4)
                && c.expected.spsr[spsr] == c.initial.cpsr
                && f.len() == 3
                && f.iter().all(|t| t.size == 4)
                && f[0].addr == c.base_addr.wrapping_add(8)
                && f[1].addr == vector
                && f[2].addr == vector + 4
                && c.expected.pipeline == [f[1].data, f[2].data]
                && c.expected.r[15] == vector + 8
                && rest_untouched(c, mode, spsr)
        },
        least,
    )
}

fn swi_entry_problems() -> Vec<String> {
    entry_problems("SWI entry", SWI_FILE, 20_000)
}

#[test]
fn given_swi_cases_with_a_true_condition_when_comparing_then_svc_is_entered_through_vector_08() {
    assert_eq!(swi_entry_problems(), Vec::<String>::new());
}

fn coprocessor_entry_problems() -> Vec<String> {
    COPROCESSOR_FILES
        .iter()
        .flat_map(|&file| entry_problems(file.0, file, 20_000))
        .collect()
}

#[test]
fn given_coprocessor_cases_with_a_true_condition_when_comparing_then_undefined_is_entered_through_vector_04()
 {
    assert_eq!(coprocessor_entry_problems(), Vec::<String>::new());
}

/// Every coprocessor number, CP14 and CP15 included, traps: each (file, number) pair has
/// entering cases and none of them is a plain step.
fn coprocessor_number_problems() -> Vec<String> {
    COPROCESSOR_FILES
        .iter()
        .flat_map(|&(file, _, _, _)| {
            let cases = labeled(file);
            (0..16u32).flat_map(move |cp| {
                fact_problems(
                    &format!("{file} cp{cp}"),
                    &cases,
                    |c| condition_passes(c) && c.opcode >> 8 & 0xF == cp,
                    |c| {
                        c.expected.cpsr & 0x1F == 0x1B
                            && fetches(c).get(1).is_some_and(|f| f.addr == 4)
                    },
                    1_000,
                )
            })
        })
        .collect()
}

#[test]
fn given_coprocessor_numbers_0_to_15_when_comparing_then_every_one_enters_undefined() {
    assert_eq!(coprocessor_number_problems(), Vec::<String>::new());
}

/// A failed condition is a plain step: no exception, the CPSR as it was.
fn failed_condition_problems() -> Vec<String> {
    std::iter::once(SWI_FILE)
        .chain(COPROCESSOR_FILES)
        .flat_map(|(file, _, _, _)| {
            fact_problems(
                &format!("{file} failed condition"),
                &labeled(file),
                |c| !condition_passes(c),
                |c| plain_step(c) && c.expected.cpsr == c.initial.cpsr,
                20_000,
            )
        })
        .collect()
}

#[test]
fn given_exception_cases_with_a_false_condition_when_comparing_then_the_step_is_plain() {
    assert_eq!(failed_condition_problems(), Vec::<String>::new());
}

#[test]
fn given_arm_swi_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_swi.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_cdp_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_cdp.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_mcr_mrc_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_mcr_mrc.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_arm_stc_ldc_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("arm_stc_ldc.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

// ---- Thumb (#13, `docs/hw/thumb.md`) ----------------------------------------------------

/// The mask and value of a Thumb MUL (format 4, op 1101): halfword bits 15–6 = 0100001101.
const THUMB_MUL_MASK: u32 = 0xFFC0;
const THUMB_MUL: u32 = 0x4340;

/// Whether `diff` is exactly the known ARM7 multiply-carry gap on a Thumb MUL: the only
/// difference is the CPSR, in bit 29 alone.
fn is_thumb_mul_c_gap(diff: &ninge_runner::cpu_suite::CaseDiff) -> bool {
    let cpsr_bits = |text: &str| u32::from_str_radix(text.trim_start_matches("0x"), 16);
    match diff.fields.as_slice() {
        [field] => {
            field.name == "CPSR"
                && cpsr_bits(&field.expected)
                    .ok()
                    .zip(cpsr_bits(&field.actual).ok())
                    .is_some_and(|(want, got)| want ^ got == 1 << 29)
                && diff.opcode & THUMB_MUL_MASK == THUMB_MUL
        }
        _ => false,
    }
}

/// Problems with `thumb_data_proc` run on the ARM7: every diff that is not the C-only gap of a
/// MUL, and a C-only count that does not equal the number of MUL cases whose C the suite changes
/// (a Thumb instruction has no condition, so every MUL executes).
fn thumb_multiply_gap_problems() -> Vec<String> {
    let file = "thumb_data_proc.json.bin";
    let cases = cases_of(file);
    let diffs = arm7_diffs(file);
    let others: Vec<String> = diffs
        .iter()
        .filter(|d| !is_thumb_mul_c_gap(d))
        .map(|d| format!("{file}[{}] {:#06x}: {:?}", d.index, d.opcode, d.fields))
        .collect();
    let c_only = diffs.len() - others.len();
    let want = cases
        .iter()
        .filter(|c| c.opcode & THUMB_MUL_MASK == THUMB_MUL)
        .filter(|c| (c.initial.cpsr ^ c.expected.cpsr) >> 29 & 1 == 1)
        .count();
    others
        .into_iter()
        .chain((c_only != want).then(|| format!("{file}: {c_only} C-only diffs, {want} expected")))
        .collect()
}

/// The ARM7 C after a Thumb MUL is the gap of #81, as for the ARM multiply: the CPU leaves C
/// unchanged and the suite changes it in about half of the MUL cases. When #81 closes, this
/// becomes `given_thumb_data_proc_when_run_on_arm7_then_no_case_differs`.
#[test]
fn given_thumb_data_proc_when_run_on_arm7_then_only_the_c_bit_of_mul_cases_differs() {
    assert_eq!(thumb_multiply_gap_problems(), Vec::<String>::new());
}

#[test]
fn given_thumb_add_cmp_mov_hi_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_add_cmp_mov_hi.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_add_sp_or_pc_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_add_sp_or_pc.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_add_sub_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_add_sub.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_add_sub_sp_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_add_sub_sp.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_b_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_b.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_bcc_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_bcc.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_bl_blx_prefix_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_bl_blx_prefix.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_bl_suffix_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_bl_suffix.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_bx_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_bx.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldm_stm_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldm_stm.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldr_pc_rel_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldr_pc_rel.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldr_str_imm_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldr_str_imm_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldr_str_reg_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldr_str_reg_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldr_str_sp_rel_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldr_str_sp_rel.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldrb_strb_imm_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldrb_strb_imm_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldrh_strh_imm_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldrh_strh_imm_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldrh_strh_reg_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldrh_strh_reg_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldrsb_strb_reg_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldrsb_strb_reg_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_ldrsh_ldrsb_reg_offset_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_ldrsh_ldrsb_reg_offset.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_lsl_lsr_asr_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_lsl_lsr_asr.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_mov_cmp_add_sub_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_mov_cmp_add_sub.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_push_pop_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_push_pop.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_swi_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_swi.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}

#[test]
fn given_thumb_undefined_bcc_when_run_on_arm7_then_no_case_differs() {
    assert_eq!(
        arm7_diffs("thumb_undefined_bcc.json.bin"),
        Vec::<ninge_runner::cpu_suite::CaseDiff>::new()
    );
}
