// SPDX-License-Identifier: Apache-2.0
//! The fetched ARM7TDMI JSON suite, checked against the reader and pinning what the harness
//! assumes about its data. Needs `./scripts/fetch_cpu_suite.sh` (verify.sh runs it first).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ninge_runner::cpu_suite::{Case, State, decode, run_json_suite};

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
    let mut problems: Vec<String> = cases_of("arm_mrs.json.bin")
        .iter()
        .filter(|c| {
            condition_passes(c)
                && is_user_or_system(c)
                && c.opcode >> 22 & 1 == 1
                && c.opcode >> 12 & 0xF < 8
        })
        .filter(|c| c.expected.r[(c.opcode >> 12 & 0xF) as usize] != c.initial.cpsr)
        .map(|c| format!("{:#010x}: Rd is not the CPSR", c.opcode))
        .collect();
    problems.truncate(5);
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
    let mut problems = Vec::new();
    for file in &SUITE_FILES_8[..3] {
        problems.extend(
            cases_of(file)
                .iter()
                .filter(|c| {
                    condition_passes(c)
                        && is_user_or_system(c)
                        && c.opcode >> 20 & 1 == 1
                        && c.opcode >> 12 & 0xF == 15
                })
                .filter(|c| c.expected.cpsr & 0xFF != c.initial.cpsr & 0xFF)
                .map(|c| format!("{file} {:#010x}: control byte changed", c.opcode)),
        );
    }
    problems
}

/// After MSR to the CPSR from User mode, the control byte is unchanged.
fn user_msr_problems() -> Vec<String> {
    ["arm_msr_imm.json.bin", "arm_msr_reg.json.bin"]
        .iter()
        .flat_map(|file| {
            cases_of(file)
                .into_iter()
                .filter(|c| {
                    condition_passes(c) && c.initial.cpsr & 0x1F == 0x10 && c.opcode >> 22 & 1 == 0
                })
                .filter(|c| c.expected.cpsr & 0xFF != c.initial.cpsr & 0xFF)
                .map(move |c| format!("{file} {:#010x}: control byte changed", c.opcode))
        })
        .collect()
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
