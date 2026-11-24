// SPDX-License-Identifier: Apache-2.0
//! The done-when check of issue #60: a trace of our own test ROM (`testroms/smoke`, built by
//! `verify.sh`, never committed), compared with a stand-in oracle that follows the same
//! instructions but has one register different from instruction N on, must report instruction N
//! as the first divergence, with class mismatch and the differing register named.
//!
//! The stand-in is the fake GDB stub scripted with the same records, so the whole path runs: the
//! trace text, `run_live`, the adapter reading registers and opcode bytes over the wire, `compare`
//! and the report. The expected instruction is N itself, known by construction; the class and the
//! field follow `trace-oracle.md` ("Divergence classes": a plain register difference, not after an
//! SWI, is a mismatch). The ROM has no SWI in the window, so no case sits right after one.

use std::sync::OnceLock;

use ninge_core::trace::{Opcode, TraceCpu, TraceCpus, TraceRecord, TraceSink, Tracer};
use ninge_core::{Input, Machine};

use super::trace_text;
use crate::cli::{CliError, LiveArgs, Report, run_live};
use crate::diff::{Class, Field, Outcome, compare};
use crate::oracle::tests::fake::{Fake, script_of};
use crate::oracle::{OracleCpu, RegisterLayout};

/// Frames the ROM runs for.
const FRAMES: u32 = 3;
/// How many of the first records are kept: the fake stub is driven record by record, and a frame
/// holds over a million instructions.
const WINDOW: usize = 3_000;
/// The register that differs on the oracle side from instruction N on.
const CHANGED: usize = 3;

/// A sink that keeps the first [`WINDOW`] records and drops the rest.
struct Head(Vec<TraceRecord>);

impl TraceSink for Head {
    fn record(&mut self, record: &TraceRecord) {
        if self.0.len() < WINDOW {
            self.0.push(*record);
        }
    }
}

/// The first records of the ARM9 running the smoke ROM for [`FRAMES`] frames.
fn smoke_records() -> &'static [TraceRecord] {
    static RECORDS: OnceLock<Vec<TraceRecord>> = OnceLock::new();
    RECORDS.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testroms/build/smoke.nds"
        );
        let rom = std::fs::read(path).expect("the smoke ROM is built by verify.sh");
        let mut machine = Machine::new();
        machine.load_rom(rom).expect("the smoke ROM loads");
        let mut tracer = Tracer::new(Head(Vec::new()), TraceCpus::Arm9);
        (0..FRAMES).for_each(|_| machine.run_frame_traced(Input::default(), &mut tracer));
        tracer.into_sink().0
    })
}

/// The records with [`CHANGED`] different from instruction `from` on, as the oracle would have it.
fn altered(records: &[TraceRecord], from: usize) -> Vec<TraceRecord> {
    records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let mut record = *record;
            if index >= from {
                record.regs[CHANGED] = record.regs[CHANGED].wrapping_add(1);
            }
            record
        })
        .collect()
}

fn args() -> LiveArgs {
    LiveArgs {
        ninge: "smoke.trace".into(),
        port: 1,
        cpu: TraceCpu::Arm9,
        breakpoint: None,
        from_hit: 0,
        hits: u32::MAX,
        max: None,
    }
}

/// `run_live` of ninge's `records` against a stub whose states are `oracle`.
fn live(records: &[TraceRecord], oracle: &[TraceRecord]) -> Result<Report, CliError> {
    let (fake, _) = Fake::new(script_of(oracle));
    run_live(fake, &trace_text(records), &args())
}

/// `OracleCpu` and `compare` directly, without the report.
fn direct(records: &[TraceRecord], oracle: &[TraceRecord]) -> Outcome {
    let (fake, _) = Fake::new(script_of(oracle));
    let mut cpu = OracleCpu::connect(fake, TraceCpu::Arm9, RegisterLayout::default());
    compare(
        records.iter().copied(),
        std::iter::from_fn(|| cpu.next_record().transpose()),
    )
    .expect("the stub answers")
}

/// The instruction numbers `first` names as first divergence that are not `n`, or whose class,
/// field or report text is wrong, as one line per wrong case.
fn wrong_reports(records: &[TraceRecord], cases: &[usize]) -> Vec<String> {
    cases
        .iter()
        .flat_map(|&n| {
            let oracle = altered(records, n);
            let mut wrong = Vec::new();
            let report = live(records, &oracle).expect("the live run finishes");
            let header = format!("first divergence at instruction {n} (ARM9): mismatch\n");
            if !report.diverged || !report.text.starts_with(&header) {
                wrong.push(format!("N={n}: report was {:?}", report.text));
            }
            if !report.text.contains("differs: r3\n") {
                wrong.push(format!("N={n}: r3 is not the differing field"));
            }
            let found = match direct(records, &oracle) {
                Outcome::Diverged(divergence) => Some(*divergence),
                _ => None,
            };
            let expected = Some((
                n as u32,
                Class::Mismatch,
                vec![Field::Register(CHANGED as u8)],
            ));
            let got = found.map(|d| (d.ninge.step, d.class, d.differing().collect::<Vec<_>>()));
            if got != expected {
                wrong.push(format!("N={n}: direct comparison found {got:?}"));
            }
            wrong
        })
        .collect()
}

#[test]
fn given_the_smoke_rom_when_traced_then_the_window_is_full_and_steps_count_from_0() {
    let records = smoke_records();
    let steps_ok = records
        .iter()
        .enumerate()
        .all(|(index, record)| record.step as usize == index);
    assert_eq!((records.len(), steps_ok), (WINDOW, true));
}

/// Whether `record` is a software interrupt: ARM `EF......` (any condition), Thumb `DFxx`.
fn is_swi(record: &TraceRecord) -> bool {
    match record.opcode {
        Opcode::Arm(word) => word & 0x0F00_0000 == 0x0F00_0000,
        Opcode::Thumb(half) => half >> 8 == 0xDF,
    }
}

#[test]
fn given_the_smoke_rom_when_traced_then_no_record_in_the_window_is_an_swi() {
    // The cases below are mismatches because nothing precedes them that is a BIOS call; if the
    // ROM grows an SWI this test says so, instead of the class assertions failing obscurely.
    let swis = smoke_records().iter().filter(|record| is_swi(record));
    assert_eq!(swis.count(), 0);
}

#[test]
fn given_a_register_changed_from_instruction_n_when_run_live_then_n_is_the_first_divergence() {
    let records = smoke_records();
    let last = records.len() - 1;
    let wrong = wrong_reports(records, &[0, 1, 57, 100, last]);
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn given_the_unchanged_stub_when_run_live_then_all_records_match() {
    let records = smoke_records();
    let report = live(records, records).expect("the live run finishes");
    assert_eq!(
        report,
        Report {
            text: format!("ARM9: {} instructions match", records.len()),
            diverged: false,
        }
    );
}

#[test]
fn given_the_unchanged_stub_when_compared_directly_then_all_records_matched() {
    let records = smoke_records();
    assert_eq!(
        direct(records, records),
        Outcome::Match {
            compared: records.len() as u32
        }
    );
}

#[test]
fn given_a_change_from_instruction_57_when_compared_twice_then_the_two_reports_are_equal() {
    let records = smoke_records();
    let oracle = altered(records, 57);
    let first = live(records, &oracle).expect("runs");
    let second = live(records, &oracle).expect("runs");
    assert_eq!(first, second);
}

#[test]
fn given_a_stub_that_ends_after_n_records_when_run_live_then_the_oracle_is_reported_ended() {
    let records = smoke_records();
    let report = live(records, &records[..100]).expect("runs");
    assert_eq!(
        report,
        Report {
            text: "ARM9: the oracle ended after 100 matching instructions".to_string(),
            diverged: true,
        }
    );
}
