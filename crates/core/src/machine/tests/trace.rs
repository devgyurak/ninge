// SPDX-License-Identifier: Apache-2.0
//! The traced run through `Machine`: `docs/contributing/trace-oracle.md` ("The trace format",
//! "BIOS collapsing") says a record is the state before an executed instruction, r15 is the
//! instruction's own address, `step` counts one CPU's records from 0, and BIOS execution leaves no
//! line. `docs/hw/arm-branch-pipeline.md` gives the +8 / +4 pipeline offsets the record undoes,
//! and `docs/hw/hle-swi.md` the BIOS areas. Programs are our own, hand-assembled.

use super::bios::{
    BX_LR, DTCM_BASE, DTCM_ENABLE, DTCM_REGISTER, ENTRY7, ENTRY9, HANDLER, HIGH_VECTORS, IE, IME,
    PROGRAM, SP_IRQ9, SYSTEM_I_CLEAR, TIMER0, arm_swi, machine_with,
};
use crate::cpu::{Bank, Bus};
use crate::io::irq::Requests;
use crate::machine::*;
use crate::trace::{NullSink, Opcode, TraceCpu, TraceCpus, TraceRecord, TraceSink, Tracer};

/// `mov r0, #0x55`, `mov r1, #0x66`, `add r2, r0, r1`, `b .`.
const ARM_SEQUENCE: [u32; 4] = [0xE3A0_0055, 0xE3A0_1066, 0xE080_2001, 0xEAFF_FFFE];
/// `add r0, pc, #1`, `bx r0` (to the Thumb code at +8), then Thumb `movs r0, #0x55`,
/// `movs r1, #0x66`, `b .`.
const THUMB_SEQUENCE: [u32; 4] = [0xE28F_0001, 0xE12F_FF10, 0x2166_2055, 0x0000_E7FE];
/// `add r0, r0, #1`, `b .-4`: a loop that changes state every instruction.
const COUNTING_LOOP: [u32; 2] = [0xE280_0001, 0xEAFF_FFFD];
/// `mov r1, #8`, `str r1, [r0, #0x214]` (acknowledge timer 0 in IF; the dispatcher left r0 at the
/// I/O base), `bx lr`.
const ACK_HANDLER: [u32; 3] = [0xE3A0_1008, 0xE580_1214, BX_LR];
/// The BIOS areas, `[base, size]` per CPU (`hle-swi.md`, "The BIOS area").
const BIOS9: (u32, u32) = (0xFFFF_0000, 32 << 10);
const BIOS7: (u32, u32) = (0, 16 << 10);

/// A sink that keeps the first `cap` records of each CPU, so a frame of a spinning CPU does not
/// store a million lines.
#[derive(Clone, Debug)]
struct Capped {
    records: Vec<TraceRecord>,
    cap: usize,
}

impl TraceSink for Capped {
    fn record(&mut self, record: &TraceRecord) {
        let same_cpu = self.records.iter().filter(|r| r.cpu == record.cpu).count();
        if same_cpu < self.cap {
            self.records.push(*record);
        }
    }
}

/// Runs `frames` frames traced and returns the kept records of each CPU, in order, and the step
/// counters of the tracer (ARM9, ARM7).
fn trace_frames(
    m: &mut Machine,
    cpus: TraceCpus,
    frames: u32,
) -> (Vec<TraceRecord>, Vec<TraceRecord>, [u32; 2]) {
    let mut tracer = Tracer::new(
        Capped {
            records: Vec::new(),
            cap: 64,
        },
        cpus,
    );
    (0..frames).for_each(|_| m.run_frame_traced(crate::Input::new(), &mut tracer));
    let steps = [tracer.steps(TraceCpu::Arm9), tracer.steps(TraceCpu::Arm7)];
    let all = tracer.into_sink().records;
    let of = |cpu| {
        all.iter()
            .copied()
            .filter(|r| r.cpu == cpu)
            .collect::<Vec<_>>()
    };
    (of(TraceCpu::Arm9), of(TraceCpu::Arm7), steps)
}

/// Both screens, the audio and the savestate bytes: everything a traced run must leave as an
/// untraced one does.
fn observable(m: &Machine) -> (Vec<u32>, Vec<u32>, Vec<i16>, Vec<u8>) {
    (
        m.frame(Screen::Top).to_vec(),
        m.frame(Screen::Bottom).to_vec(),
        m.audio().to_vec(),
        m.save_state(),
    )
}

/// The observable state after each of `frames` frames, run untraced or traced into a null sink.
fn observed_after_each_frame(program9: &[u32], traced: bool, frames: u32) -> Vec<Observed> {
    let mut m = machine_with(program9, &PROGRAM);
    let mut tracer = Tracer::new(NullSink, TraceCpus::Both);
    (0..frames)
        .map(|_| {
            match traced {
                true => m.run_frame_traced(crate::Input::new(), &mut tracer),
                false => m.run_frame(crate::Input::new()),
            }
            observable(&m)
        })
        .collect()
}

fn run_untraced(m: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| m.run_frame(crate::Input::new()));
}

fn run_null_traced(m: &mut Machine, frames: u32) {
    let mut tracer = Tracer::new(NullSink, TraceCpus::Both);
    (0..frames).for_each(|_| m.run_frame_traced(crate::Input::new(), &mut tracer));
}

type Observed = (Vec<u32>, Vec<u32>, Vec<i16>, Vec<u8>);

/// How many records have a pc in `area` of the CPU's BIOS.
fn in_area(records: &[TraceRecord], (base, size): (u32, u32)) -> usize {
    records
        .iter()
        .filter(|r| r.pc().wrapping_sub(base) < size)
        .count()
}

fn pcs(records: &[TraceRecord]) -> Vec<u32> {
    records.iter().map(TraceRecord::pc).collect()
}

fn steps_of(records: &[TraceRecord]) -> Vec<u32> {
    records.iter().map(|r| r.step).collect()
}

/// An ARM9 taking an IRQ at once, with a handler that acknowledges it, then running an SWI at
/// `ENTRY9`. The vectors are the BIOS's (high), the user handler is pointed to from the DTCM.
fn arm9_with_irq_then_swi() -> Machine {
    let program = [arm_swi(0x1E_0000), 0xE3A0_1066, 0xEAFF_FFFE];
    let mut m = machine_with(&program, &PROGRAM);
    m.arm9.write_cp15(9, 1, 0, DTCM_REGISTER);
    m.arm9.write_cp15(1, 0, 0, HIGH_VECTORS | DTCM_ENABLE);
    m.bus9().write32(DTCM_BASE + 0x3FFC, HANDLER);
    ACK_HANDLER
        .iter()
        .enumerate()
        .for_each(|(i, &word)| m.bus9().write32(HANDLER + 4 * i as u32, word));
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m.arm9.regs_mut().set_banked(Bank::Irq, 13, SP_IRQ9);
    m.bus9().write32(IME, 1);
    m.bus9().write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
    m
}

// ------------------------------------------------------------------ traced equals untraced

#[test]
fn given_a_counting_loop_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run() {
    let untraced = observed_after_each_frame(&COUNTING_LOOP, false, 3);
    let traced = observed_after_each_frame(&COUNTING_LOOP, true, 3);
    assert!(
        traced == untraced,
        "a traced run differs from an untraced one"
    );
}

#[test]
fn given_an_irq_and_swi_program_when_run_traced_then_the_savestate_equals_an_untraced_run() {
    let mut untraced = arm9_with_irq_then_swi();
    let mut traced = arm9_with_irq_then_swi();
    run_untraced(&mut untraced, 2);
    run_null_traced(&mut traced, 2);
    assert!(
        observable(&traced) == observable(&untraced),
        "states differ"
    );
}

#[test]
fn given_a_traced_run_when_saved_and_loaded_then_the_state_continues_as_an_untraced_one() {
    let mut a = machine_with(&COUNTING_LOOP, &PROGRAM);
    let mut tracer = Tracer::new(NullSink, TraceCpus::Both);
    a.run_frame_traced(crate::Input::new(), &mut tracer);
    let mut b = Machine::new();
    b.load_rom(a.rom().to_vec()).expect("same image");
    b.load_state(&a.save_state()).expect("own savestate");
    a.run_frame(crate::Input::new());
    b.run_frame(crate::Input::new());
    assert!(observable(&a) == observable(&b), "continuations differ");
}

#[test]
fn given_a_traced_run_when_it_ends_then_the_frame_count_advanced_by_one_per_frame() {
    let mut m = machine_with(&COUNTING_LOOP, &PROGRAM);
    run_null_traced(&mut m, 3);
    assert_eq!(m.frame_count(), 3);
}

#[test]
fn given_a_machine_without_a_rom_when_run_traced_then_nothing_is_recorded() {
    let mut m = Machine::new();
    let (nine, seven, steps) = trace_frames(&mut m, TraceCpus::Both, 1);
    assert_eq!((nine.len(), seven.len(), steps), (0, 0, [0, 0]));
}

// -------------------------------------------------------------------------- ARM records

#[test]
fn given_an_arm_sequence_when_traced_then_pc_and_opcode_are_the_instruction_at_each_step() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    let got: Vec<(u32, u32, Opcode)> = nine
        .iter()
        .take(5)
        .map(|r| (r.step, r.pc(), r.opcode))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, ENTRY9, Opcode::Arm(0xE3A0_0055)),
            (1, ENTRY9 + 4, Opcode::Arm(0xE3A0_1066)),
            (2, ENTRY9 + 8, Opcode::Arm(0xE080_2001)),
            (3, ENTRY9 + 12, Opcode::Arm(0xEAFF_FFFE)),
            (4, ENTRY9 + 12, Opcode::Arm(0xEAFF_FFFE)),
        ]
    );
}

#[test]
fn given_an_arm_sequence_when_traced_then_registers_are_the_state_before_each_instruction() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    // r0, r1, r2 before steps 0..=3: mov r0 sets r0, mov r1 sets r1, add sets r2 = r0 + r1.
    let got: Vec<[u32; 3]> = nine
        .iter()
        .take(4)
        .map(|r| [r.regs[0], r.regs[1], r.regs[2]])
        .collect();
    assert_eq!(
        got,
        vec![[0, 0, 0], [0x55, 0, 0], [0x55, 0x66, 0], [0x55, 0x66, 0xBB]]
    );
}

#[test]
fn given_an_arm_sequence_when_traced_then_r15_is_the_instruction_address_not_pc_plus_8() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    let r15: Vec<u32> = nine.iter().take(3).map(|r| r.regs[15]).collect();
    assert_eq!(r15, vec![ENTRY9, ENTRY9 + 4, ENTRY9 + 8]);
}

#[test]
fn given_an_arm_sequence_when_traced_then_the_cpsr_is_the_direct_boot_value_with_t_clear() {
    // System mode with I and F set (0xDF) after direct boot (`direct-boot.md`).
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    assert_eq!(nine[0].cpsr, 0xDF);
}

#[test]
fn given_a_traced_arm_sequence_when_each_record_is_printed_and_parsed_then_it_round_trips() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    let back: Vec<_> = nine
        .iter()
        .map(|r| crate::trace::parse_line(&r.to_string()))
        .collect();
    let want: Vec<_> = nine.iter().map(|r| Ok(*r)).collect();
    assert_eq!(back, want);
}

// ------------------------------------------------------------------------- Thumb records

#[test]
fn given_a_thumb_sequence_when_traced_then_thumb_opcodes_are_halfwords_at_their_own_address() {
    let mut m = machine_with(&THUMB_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    let got: Vec<(u32, Opcode)> = nine.iter().take(6).map(|r| (r.pc(), r.opcode)).collect();
    assert_eq!(
        got,
        vec![
            (ENTRY9, Opcode::Arm(0xE28F_0001)),
            (ENTRY9 + 4, Opcode::Arm(0xE12F_FF10)),
            (ENTRY9 + 8, Opcode::Thumb(0x2055)),
            (ENTRY9 + 10, Opcode::Thumb(0x2166)),
            (ENTRY9 + 12, Opcode::Thumb(0xE7FE)),
            (ENTRY9 + 12, Opcode::Thumb(0xE7FE)),
        ]
    );
}

#[test]
fn given_a_thumb_sequence_when_traced_then_the_thumb_records_have_the_t_bit_and_arm_ones_do_not() {
    let mut m = machine_with(&THUMB_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    let t: Vec<u32> = nine.iter().take(4).map(|r| r.cpsr & 0x20).collect();
    assert_eq!(t, vec![0, 0, 0x20, 0x20]);
}

#[test]
fn given_a_thumb_sequence_when_traced_then_r15_is_the_address_not_pc_plus_4_and_registers_precede()
{
    let mut m = machine_with(&THUMB_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    // `add r0, pc, #1` makes r0 = ENTRY9 + 8 + 1 (the Thumb entry); `movs r0, #0x55` then `movs r1`.
    let got: Vec<(u32, u32, u32)> = nine
        .iter()
        .skip(1)
        .take(4)
        .map(|r| (r.regs[15], r.regs[0], r.regs[1]))
        .collect();
    assert_eq!(
        got,
        vec![
            (ENTRY9 + 4, ENTRY9 + 9, 0),
            (ENTRY9 + 8, ENTRY9 + 9, 0),
            (ENTRY9 + 10, 0x55, 0),
            (ENTRY9 + 12, 0x55, 0x66),
        ]
    );
}

#[test]
fn given_a_thumb_sequence_when_each_record_is_printed_then_the_opcode_column_matches_the_width() {
    let mut m = machine_with(&THUMB_SEQUENCE, &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    let widths: Vec<usize> = nine
        .iter()
        .take(4)
        .map(|r| r.to_string().split(' ').nth(3).map_or(0, str::len))
        .collect();
    assert_eq!(widths, vec![8, 8, 4, 4]);
}

// --------------------------------------------------------------------------- BIOS collapsing

#[test]
fn given_an_swi_program_when_traced_then_no_record_has_a_pc_in_either_bios_area() {
    let mut m = machine_with(&[arm_swi(0x1E_0000), 0xE3A0_1066, 0xEAFF_FFFE], &PROGRAM);
    m.arm9.write_cp15(1, 0, 0, HIGH_VECTORS);
    let (nine, seven, _) = trace_frames(&mut m, TraceCpus::Both, 1);
    assert_eq!(
        (
            m.inspect().swi_record9().last(),
            in_area(&nine, BIOS9),
            in_area(&seven, BIOS7)
        ),
        (Some(0x1E), 0, 0)
    );
}

#[test]
fn given_an_swi_program_when_traced_then_the_swi_is_recorded_and_the_next_record_is_the_return() {
    let mut m = machine_with(&[arm_swi(0x1E_0000), 0xE3A0_1066, 0xEAFF_FFFE], &PROGRAM);
    m.arm9.write_cp15(1, 0, 0, HIGH_VECTORS);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    assert_eq!(pcs(&nine[..3]), vec![ENTRY9, ENTRY9 + 4, ENTRY9 + 8]);
}

#[test]
fn given_an_swi_program_when_traced_then_steps_count_only_recorded_instructions() {
    let mut m = machine_with(&[arm_swi(0x1E_0000), 0xE3A0_1066, 0xEAFF_FFFE], &PROGRAM);
    m.arm9.write_cp15(1, 0, 0, HIGH_VECTORS);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    assert_eq!(steps_of(&nine[..4]), vec![0, 1, 2, 3]);
}

#[test]
fn given_an_irq_taken_then_an_swi_when_traced_then_handler_then_program_run_with_no_bios_pc() {
    let mut m = arm9_with_irq_then_swi();
    let (nine, seven, _) = trace_frames(&mut m, TraceCpus::Both, 1);
    // The IRQ entry and the dispatcher leave no line; the handler's three instructions come
    // first, then the interrupted instruction (the SWI) at ENTRY9, whose BIOS trap leaves none.
    assert_eq!(
        (
            pcs(&nine[..6]),
            in_area(&nine, BIOS9),
            in_area(&seven, BIOS7),
            m.inspect().swi_record9().last()
        ),
        (
            vec![
                HANDLER,
                HANDLER + 4,
                HANDLER + 8,
                ENTRY9,
                ENTRY9 + 4,
                ENTRY9 + 8
            ],
            0,
            0,
            Some(0x1E)
        )
    );
}

// ----------------------------------------------------------------------------- CPU filter

#[test]
fn given_the_arm9_filter_when_both_cpus_run_then_there_are_no_arm7_records_and_no_arm7_steps() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, seven, steps) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    assert_eq!((nine.is_empty(), seven.len(), steps[1]), (false, 0, 0));
}

#[test]
fn given_the_arm7_filter_when_both_cpus_run_then_there_are_no_arm9_records_and_no_arm9_steps() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, seven, steps) = trace_frames(&mut m, TraceCpus::Arm7, 1);
    assert_eq!((nine.len(), seven.is_empty(), steps[0]), (0, false, 0));
}

#[test]
fn given_the_both_filter_when_a_frame_runs_then_each_cpu_has_records() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, seven, _) = trace_frames(&mut m, TraceCpus::Both, 1);
    assert_eq!((nine.is_empty(), seven.is_empty()), (false, false));
}

#[test]
fn given_the_both_filter_when_a_frame_runs_then_each_cpu_counts_its_own_steps_from_zero() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (nine, seven, _) = trace_frames(&mut m, TraceCpus::Both, 1);
    let want: Vec<u32> = (0..64).collect();
    assert_eq!((steps_of(&nine), steps_of(&seven)), (want.clone(), want));
}

#[test]
fn given_the_both_filter_when_a_frame_runs_then_the_arm7_records_start_at_its_entry() {
    let mut m = machine_with(&ARM_SEQUENCE, &PROGRAM);
    let (_, seven, _) = trace_frames(&mut m, TraceCpus::Both, 1);
    assert_eq!(pcs(&seven[..3]), vec![ENTRY7, ENTRY7 + 4, ENTRY7 + 8]);
}

#[test]
fn given_a_trace_over_two_frames_when_it_ends_then_the_step_counter_continues_across_frames() {
    let mut m = machine_with(&COUNTING_LOOP, &PROGRAM);
    let mut tracer = Tracer::new(NullSink, TraceCpus::Arm9);
    m.run_frame_traced(crate::Input::new(), &mut tracer);
    let after_one = tracer.steps(TraceCpu::Arm9);
    m.run_frame_traced(crate::Input::new(), &mut tracer);
    assert_eq!(
        (
            after_one > 0,
            tracer.steps(TraceCpu::Arm9) > after_one,
            tracer.steps(TraceCpu::Arm7)
        ),
        (true, true, 0)
    );
}

#[test]
fn given_an_arm9_that_jumps_into_its_bios_area_when_traced_then_no_record_has_a_bios_pc() {
    // `mvn r0, #0xFF00` (0xFFFF00FF), `sub r0, r0, #1`, `mov pc, r0`: the ARM9 lands in the BIOS
    // area at an address the HLE does not trap, so it executes there.
    let mut m = machine_with(&[0xE3E0_0CFF, 0xE240_0001, 0xE1A0_F000], &PROGRAM);
    let (nine, _, _) = trace_frames(&mut m, TraceCpus::Arm9, 1);
    // The instructions run in the area leave no record, so the record after the jump is the
    // first one past the area's end (or anywhere out of it) and the steps stay consecutive.
    let after_jump = nine[3].pc();
    assert_eq!(
        (
            in_area(&nine, BIOS9),
            after_jump.wrapping_sub(BIOS9.0) >= BIOS9.1,
            nine[3].step
        ),
        (0, true, 3)
    );
}

// ------------------------------------------------------- traced equals untraced: halted CPUs, TCM
//
// `traced.rs` repeats the untraced loops of `events.rs`, so the paths a plain counting loop never
// takes (a halted core, a TCM mapping the guest moves mid-run) need their own equality tests.
// Expected values: a halted core runs nothing and wakes on `IE & IF != 0` whatever IME is
// (`hle-swi.md`, "Halt, and what wakes a CPU"; `cp15.md`, "C7"; `interrupts.md`, "HALTCNT"); a CP15
// change takes effect for the next instruction and a disabled or moved TCM lets the access reach the
// memory underneath (`arm9-memory-map.md`, "TCM").

/// `mcr p15, 0, r2, c7, c0, 4` (wait-for-interrupt; r2 is never written by these programs).
const WFI: u32 = 0xEE07_2F90;
/// `mov r1, #0x66`, `mov r2, #0x66` and `b .`.
const MARK_R1: u32 = 0xE3A0_1066;
const MARK_R2: u32 = 0xE3A0_2066;
const SPIN_WORD: u32 = 0xEAFF_FFFE;
/// `mov r0, #0x04000000`, `mov r1, #1`, `str r1, [r0, #0x210]` (IE: V-blank), `mov r1, #8`,
/// `strh r1, [r0, #4]` (DISPSTAT: V-blank IRQ enable): the V-blank of the frame sets IF, which wakes
/// a halted core with IME still 0. The program then halts, so the wake comes ~192 lines in.
const ENABLE_VBLANK_IRQ: [u32; 5] = [
    0xE3A0_0301,
    0xE3A0_1001,
    0xE580_1210,
    0xE3A0_1008,
    0xE1C0_10B4,
];
/// `mov r0, #0x04000000`, `mov r1, #0x80`, `strb r1, [r0, #0x301]`: HALTCNT mode 2 (halt).
const WRITE_HALTCNT: [u32; 3] = [0xE3A0_0301, 0xE3A0_1080, 0xE5C0_1301];
/// SWI 06h, Halt.
const SWI_HALT: u32 = 0xEF06_0000;

fn concat(parts: &[&[u32]]) -> Vec<u32> {
    parts.iter().flat_map(|part| part.iter().copied()).collect()
}

/// The first 64 records of each CPU: enough to see a program's start and what follows a wake.
#[derive(Default)]
struct Heads {
    nine: Vec<TraceRecord>,
    seven: Vec<TraceRecord>,
}

impl TraceSink for Heads {
    fn record(&mut self, record: &TraceRecord) {
        let kept = match record.cpu {
            TraceCpu::Arm9 => &mut self.nine,
            TraceCpu::Arm7 => &mut self.seven,
        };
        if kept.len() < 64 {
            kept.push(*record);
        }
    }
}

/// What running two identical machines, one traced and one not, frame by frame, shows.
struct Comparison {
    /// The frames after which the two machines' screens, audio or savestate bytes differ.
    differing: Vec<usize>,
    nine: Vec<TraceRecord>,
    seven: Vec<TraceRecord>,
    traced: Machine,
}

fn compare_traced_with_untraced(arm9: &[u32], arm7: &[u32], frames: usize) -> Comparison {
    let mut untraced = machine_with(arm9, arm7);
    let mut traced = machine_with(arm9, arm7);
    let mut tracer = Tracer::new(Heads::default(), TraceCpus::Both);
    let differing = (0..frames)
        .filter(|_| {
            untraced.run_frame(crate::Input::new());
            traced.run_frame_traced(crate::Input::new(), &mut tracer);
            observable(&untraced) != observable(&traced)
        })
        .collect();
    let heads = tracer.into_sink();
    Comparison {
        differing,
        nine: heads.nine,
        seven: heads.seven,
        traced,
    }
}

/// `base + 4 * i` for each index.
fn word_addresses(base: u32, indices: &[u32]) -> Vec<u32> {
    indices.iter().map(|i| base + 4 * i).collect()
}

fn arm9_wfi_without_wake() -> Vec<u32> {
    vec![WFI, MARK_R1, SPIN_WORD]
}

fn arm9_wfi_with_wake() -> Vec<u32> {
    concat(&[&ENABLE_VBLANK_IRQ, &[WFI, MARK_R1, SPIN_WORD]])
}

fn arm7_haltcnt_without_wake() -> Vec<u32> {
    concat(&[&WRITE_HALTCNT, &[MARK_R2, SPIN_WORD]])
}

fn arm7_haltcnt_with_wake() -> Vec<u32> {
    concat(&[&ENABLE_VBLANK_IRQ, &WRITE_HALTCNT, &[MARK_R2, SPIN_WORD]])
}

fn arm7_swi_halt_without_wake() -> Vec<u32> {
    vec![SWI_HALT, MARK_R2, SPIN_WORD]
}

fn arm7_swi_halt_with_wake() -> Vec<u32> {
    concat(&[&ENABLE_VBLANK_IRQ, &[SWI_HALT, MARK_R2, SPIN_WORD]])
}

// ------------------------------------------------------------------------------------ ARM9 WFI

#[test]
fn given_an_arm9_in_wfi_with_nothing_pending_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&arm9_wfi_without_wake(), &PROGRAM, 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_an_arm9_in_wfi_with_nothing_pending_when_run_traced_then_only_the_wfi_is_recorded_and_the_core_stays_halted()
 {
    let c = compare_traced_with_untraced(&arm9_wfi_without_wake(), &PROGRAM, 3);
    assert_eq!(
        (pcs(&c.nine), c.traced.inspect().arm9().halted()),
        (vec![ENTRY9], true)
    );
}

#[test]
fn given_an_arm9_in_wfi_with_a_vblank_wake_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&arm9_wfi_with_wake(), &PROGRAM, 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_an_arm9_in_wfi_with_a_vblank_wake_when_run_traced_then_no_record_falls_in_the_halt_and_the_next_pc_is_after_the_wfi()
 {
    // Instructions 0-4 set up, 5 is the WFI, 6 the marker and 7 the spin; the spin repeats.
    let c = compare_traced_with_untraced(&arm9_wfi_with_wake(), &PROGRAM, 3);
    let want = word_addresses(ENTRY9, &[0, 1, 2, 3, 4, 5, 6, 7, 7]);
    assert_eq!(
        (
            pcs(&c.nine[..9]),
            steps_of(&c.nine[..9]),
            c.traced.inspect().arm9().regs().get(1),
            c.traced.inspect().arm9().halted()
        ),
        (want, (0..9).collect::<Vec<u32>>(), 0x66, false)
    );
}

// ------------------------------------------------------------------------------------ ARM7 halt

#[test]
fn given_an_arm7_halted_by_haltcnt_with_nothing_pending_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_haltcnt_without_wake(), 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_an_arm7_halted_by_haltcnt_with_nothing_pending_when_run_traced_then_records_stop_after_the_haltcnt_store()
 {
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_haltcnt_without_wake(), 3);
    assert_eq!(
        (pcs(&c.seven), c.traced.inspect().arm7().halted()),
        (word_addresses(ENTRY7, &[0, 1, 2]), true)
    );
}

#[test]
fn given_an_arm7_halted_by_haltcnt_with_a_vblank_wake_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_haltcnt_with_wake(), 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_an_arm7_halted_by_haltcnt_with_a_vblank_wake_when_run_traced_then_no_record_falls_in_the_halt_and_the_next_pc_is_after_the_store()
 {
    // 0-4 set up, 5-7 write HALTCNT, 8 is the marker and 9 the spin; the spin repeats.
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_haltcnt_with_wake(), 3);
    let want = word_addresses(ENTRY7, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 9]);
    assert_eq!(
        (
            pcs(&c.seven[..11]),
            steps_of(&c.seven[..11]),
            c.traced.inspect().arm7().regs().get(2),
            c.traced.inspect().arm7().halted()
        ),
        (want, (0..11).collect::<Vec<u32>>(), 0x66, false)
    );
}

#[test]
fn given_an_arm7_halted_by_swi_halt_with_nothing_pending_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_swi_halt_without_wake(), 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_an_arm7_halted_by_swi_halt_with_nothing_pending_when_run_traced_then_only_the_swi_is_recorded_and_the_core_stays_halted()
 {
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_swi_halt_without_wake(), 3);
    assert_eq!(
        (pcs(&c.seven), c.traced.inspect().arm7().halted()),
        (vec![ENTRY7], true)
    );
}

#[test]
fn given_an_arm7_halted_by_swi_halt_with_a_vblank_wake_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_swi_halt_with_wake(), 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_an_arm7_halted_by_swi_halt_with_a_vblank_wake_when_run_traced_then_no_record_falls_in_the_halt_and_the_next_pc_is_after_the_swi()
 {
    // 0-4 set up, 5 is the SWI, 6 the marker and 7 the spin; the spin repeats.
    let c = compare_traced_with_untraced(&COUNTING_LOOP, &arm7_swi_halt_with_wake(), 3);
    let want = word_addresses(ENTRY7, &[0, 1, 2, 3, 4, 5, 6, 7, 7]);
    assert_eq!(
        (
            pcs(&c.seven[..9]),
            steps_of(&c.seven[..9]),
            c.traced.inspect().arm7().regs().get(2),
            c.traced.inspect().arm7().halted()
        ),
        (want, (0..9).collect::<Vec<u32>>(), 0x66, false)
    );
}

// ------------------------------------------------------------- a TCM the guest moves mid-run

/// `MCR p15, 0, r0, c9, c1, 0` (DTCM region), `MCR p15, 0, r0, c9, c1, 1` (ITCM region) and
/// `MCR p15, 0, r1, c1, c0, 0` (control register).
const MCR_DTCM_REGION: u32 = 0xEE09_0F11;
const MCR_ITCM_REGION: u32 = 0xEE09_0F31;
const MCR_CONTROL_R1: u32 = 0xEE01_1F10;
/// Where the DTCM first sits, and the 16 KiB region value (size field 5) that puts it there.
const DTCM_FIRST: u32 = 0x0210_0000;

/// The guest enables a DTCM at `DTCM_FIRST`, stores 0x55 through it, moves it 1 MiB up, and stores
/// 0x66 at `DTCM_FIRST + 4`, which the DTCM no longer covers, so it reaches main RAM.
const MOVING_DTCM: [u32; 14] = [
    0xE3A0_0621, // mov r0, #0x02100000
    0xE380_000A, // orr r0, r0, #0xA (size field 5)
    MCR_DTCM_REGION,
    0xE3A0_1801, // mov r1, #1 << 16 (DTCM enable)
    MCR_CONTROL_R1,
    0xE3A0_2621, // mov r2, #0x02100000
    0xE3A0_3055, // mov r3, #0x55
    0xE582_3000, // str r3, [r2]: into the DTCM
    0xE280_0601, // add r0, r0, #0x00100000
    MCR_DTCM_REGION,
    0xE3A0_3066, // mov r3, #0x66
    0xE582_3004, // str r3, [r2, #4]: the DTCM is gone from here
    0xE592_4004, // ldr r4, [r2, #4]
    SPIN_WORD,
];

/// The guest sets the ITCM to 32 KiB (size field 6), enables it, then stores and loads 0x55 at
/// 0x100: only an enabled ITCM holds the word (nothing else is mapped there).
const ENABLING_ITCM: [u32; 9] = [
    0xE3A0_000C, // mov r0, #0xC
    MCR_ITCM_REGION,
    0xE3A0_1701, // mov r1, #1 << 18 (ITCM enable)
    MCR_CONTROL_R1,
    0xE3A0_2C01, // mov r2, #0x100
    0xE3A0_3055, // mov r3, #0x55
    0xE582_3000, // str r3, [r2]
    0xE592_4000, // ldr r4, [r2]
    SPIN_WORD,
];

#[test]
fn given_a_guest_that_moves_the_dtcm_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&MOVING_DTCM, &PROGRAM, 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_a_guest_that_moves_the_dtcm_when_run_traced_then_the_store_after_the_move_reaches_ram_and_the_one_before_stayed_in_the_tcm()
 {
    let c = compare_traced_with_untraced(&MOVING_DTCM, &PROGRAM, 1);
    let view = c.traced.inspect();
    assert_eq!(
        (
            view.arm9_peek32(DTCM_FIRST),
            view.arm9_peek32(DTCM_FIRST + 4),
            view.arm9().regs().get(4)
        ),
        (0, 0x66, 0x66)
    );
}

#[test]
fn given_a_guest_that_moves_the_dtcm_when_run_traced_then_records_follow_the_program_without_a_gap()
{
    let c = compare_traced_with_untraced(&MOVING_DTCM, &PROGRAM, 1);
    let want = word_addresses(ENTRY9, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 13]);
    assert_eq!(
        (pcs(&c.nine[..15]), steps_of(&c.nine[..15])),
        (want, (0..15).collect::<Vec<u32>>())
    );
}

#[test]
fn given_a_guest_that_enables_the_itcm_when_run_traced_then_frames_audio_and_savestate_equal_an_untraced_run()
 {
    let c = compare_traced_with_untraced(&ENABLING_ITCM, &PROGRAM, 3);
    assert_eq!(c.differing, Vec::<usize>::new());
}

#[test]
fn given_a_guest_that_enables_the_itcm_when_run_traced_then_the_next_store_and_load_use_the_itcm() {
    let c = compare_traced_with_untraced(&ENABLING_ITCM, &PROGRAM, 1);
    assert_eq!(
        (
            c.traced.inspect().arm9_peek32(0x100),
            c.traced.inspect().arm9().regs().get(4)
        ),
        (0x55, 0x55)
    );
}
