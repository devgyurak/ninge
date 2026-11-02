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
