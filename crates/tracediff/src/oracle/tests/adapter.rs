// SPDX-License-Identifier: Apache-2.0
//! `OracleCpu` against the fake stub. Expected records are written out by hand from the trace
//! format (`docs/contributing/trace-oracle.md`, "The trace format": registers before the
//! instruction, a 4-digit opcode in Thumb state, step counted per CPU) and the BIOS rule
//! ("BIOS collapsing": no line for execution in the area, no read of it, ARM9 at `FFFF0000h`,
//! 32 KiB, ARM7 at 0, 16 KiB).

use ninge_core::trace::{Opcode, TraceCpu, TraceRecord, parse_line};

use super::fake::{Fake, Mode, SharedLog, State, read_addresses, requests};
use crate::oracle::{AdapterError, OracleCpu, RegisterLayout};
use crate::rsp::RspError;

const ARM_STATE: u32 = 0x0000_001F;
const THUMB_STATE: u32 = 0x0000_003F;

fn connect(cpu: TraceCpu, fake: Fake) -> OracleCpu<Fake> {
    OracleCpu::connect(fake, cpu, RegisterLayout::default())
}

/// An adapter over `script` for `cpu`, and the stub's log.
fn adapter(cpu: TraceCpu, script: Vec<State>) -> (OracleCpu<Fake>, SharedLog) {
    let (fake, log) = Fake::new(script);
    (connect(cpu, fake), log)
}

/// Every record until the target exits, or the first error.
fn drain(cpu: &mut OracleCpu<Fake>) -> Result<Vec<TraceRecord>, AdapterError> {
    std::iter::from_fn(|| cpu.next_record().transpose()).collect()
}

/// The pcs and step numbers of `records`.
fn pcs_and_steps(records: &[TraceRecord]) -> Vec<(u32, u32)> {
    records.iter().map(|r| (r.pc(), r.step)).collect()
}

/// The addresses of `m` requests that fall inside `area`.
fn reads_inside(log: &SharedLog, area: std::ops::Range<u32>) -> Vec<u32> {
    read_addresses(log)
        .into_iter()
        .filter(|addr| area.contains(addr))
        .collect()
}

fn arm_at(pc: u32) -> State {
    State::at(pc).arm(0xE1A0_0000)
}

// ---- the record ----

#[test]
fn given_a_script_when_a_record_is_taken_then_it_holds_the_registers_before_the_instruction() {
    let first = State::at(0x0200_0000)
        .reg(0, 1)
        .reg(13, 0x0300_7F00)
        .arm(0xE3A0_0055);
    let second = State::at(0x0200_0004)
        .reg(0, 0x55)
        .reg(13, 0x0300_7F00)
        .arm(0xE1A0_0000);
    let (mut cpu, _) = adapter(TraceCpu::Arm9, vec![first, second]);
    let mut regs = [0; 16];
    regs[0] = 1;
    regs[13] = 0x0300_7F00;
    regs[15] = 0x0200_0000;
    assert_eq!(
        cpu.next_record().unwrap(),
        Some(TraceRecord {
            cpu: TraceCpu::Arm9,
            step: 0,
            opcode: Opcode::Arm(0xE3A0_0055),
            regs,
            cpsr: ARM_STATE,
        })
    );
}

#[test]
fn given_the_arm7_when_a_record_is_taken_then_it_is_tagged_arm7() {
    let (mut cpu, _) = adapter(TraceCpu::Arm7, vec![arm_at(0x0200_0000)]);
    assert_eq!(
        cpu.next_record().unwrap().map(|r| r.cpu),
        Some(TraceCpu::Arm7)
    );
}

#[test]
fn given_an_arm_state_cpsr_when_a_record_is_taken_then_four_bytes_are_read_and_the_opcode_is_arm() {
    let (mut cpu, log) = adapter(
        TraceCpu::Arm9,
        vec![State::at(0x0200_0000).arm(0xE3A0_0055)],
    );
    let record = cpu.next_record().unwrap().unwrap();
    assert_eq!(
        (record.opcode, requests(&log, 'm')),
        (Opcode::Arm(0xE3A0_0055), vec!["m2000000,4".to_string()])
    );
}

#[test]
fn given_a_thumb_state_cpsr_when_a_record_is_taken_then_two_bytes_are_read_and_the_opcode_is_thumb()
{
    let (mut cpu, log) = adapter(TraceCpu::Arm9, vec![State::at(0x0200_0000).thumb(0x2166)]);
    let record = cpu.next_record().unwrap().unwrap();
    assert_eq!(
        (record.opcode, record.cpsr, requests(&log, 'm')),
        (
            Opcode::Thumb(0x2166),
            THUMB_STATE,
            vec!["m2000000,2".to_string()]
        )
    );
}

#[test]
fn given_only_bit_5_of_the_cpsr_differing_when_records_are_taken_then_the_opcode_width_follows_it()
{
    let arm = State::at(0x0200_0000).arm(0xE3A0_0055);
    let thumb = State::at(0x0200_0004).thumb(0x2166);
    let (mut cpu, _) = adapter(TraceCpu::Arm9, vec![arm, thumb]);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        records
            .iter()
            .map(|r| (r.opcode, r.cpsr))
            .collect::<Vec<_>>(),
        vec![
            (Opcode::Arm(0xE3A0_0055), ARM_STATE),
            (Opcode::Thumb(0x2166), THUMB_STATE)
        ]
    );
}

#[test]
fn given_three_instructions_when_all_are_taken_then_the_steps_are_0_1_2_and_steps_says_3() {
    let script = vec![
        arm_at(0x0200_0000),
        arm_at(0x0200_0004),
        arm_at(0x0200_0008),
    ];
    let (mut cpu, _) = adapter(TraceCpu::Arm9, script);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        (pcs_and_steps(&records), cpu.steps()),
        (
            vec![(0x0200_0000, 0), (0x0200_0004, 1), (0x0200_0008, 2)],
            3
        )
    );
}

#[test]
fn given_a_fresh_adapter_when_asked_for_its_steps_then_it_is_zero() {
    let (cpu, _) = adapter(TraceCpu::Arm9, vec![arm_at(0x0200_0000)]);
    assert_eq!(cpu.steps(), 0);
}

#[test]
fn given_a_cpsr_in_another_word_when_the_layout_says_so_then_the_record_has_that_cpsr() {
    let (fake, _) = Fake::new(vec![State::at(0x0200_0000).thumb(0x2166)]);
    let mut cpu = OracleCpu::connect(
        fake.cpsr_word(25),
        TraceCpu::Arm9,
        RegisterLayout { cpsr: 25 },
    );
    assert_eq!(
        cpu.next_record().unwrap().map(|r| r.cpsr),
        Some(THUMB_STATE)
    );
}

#[test]
fn given_the_breakpoint_api_when_used_through_the_adapter_then_the_stub_gets_the_packet() {
    let (mut cpu, log) = adapter(TraceCpu::Arm9, vec![arm_at(0x0200_0000)]);
    cpu.client().set_breakpoint(0x0200_0000, 4).unwrap();
    assert_eq!(requests(&log, 'Z'), vec!["Z0,2000000,4".to_string()]);
}

// ---- BIOS collapsing ----

/// An ARM9 run that enters the BIOS at `FFFF0018h` (the exception vector) for three steps and
/// returns to the program.
fn arm9_bios_visit() -> Vec<State> {
    vec![
        arm_at(0x0200_0000),
        arm_at(0xFFFF_0018),
        arm_at(0xFFFF_001C),
        arm_at(0xFFFF_0020),
        arm_at(0x0200_0004),
        arm_at(0x0200_0008),
    ]
}

/// The same for the ARM7, whose BIOS starts at 0.
fn arm7_bios_visit() -> Vec<State> {
    vec![
        arm_at(0x0238_0000),
        arm_at(0x0000_0018),
        arm_at(0x0000_001C),
        arm_at(0x0000_0020),
        arm_at(0x0238_0004),
        arm_at(0x0238_0008),
    ]
}

#[test]
fn given_an_arm9_run_that_visits_the_bios_when_recorded_then_only_the_outside_steps_remain_in_a_row()
 {
    let (mut cpu, _) = adapter(TraceCpu::Arm9, arm9_bios_visit());
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        pcs_and_steps(&records),
        vec![(0x0200_0000, 0), (0x0200_0004, 1), (0x0200_0008, 2)]
    );
}

#[test]
fn given_an_arm7_run_that_visits_the_bios_when_recorded_then_only_the_outside_steps_remain_in_a_row()
 {
    let (mut cpu, _) = adapter(TraceCpu::Arm7, arm7_bios_visit());
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        pcs_and_steps(&records),
        vec![(0x0238_0000, 0), (0x0238_0004, 1), (0x0238_0008, 2)]
    );
}

#[test]
fn given_an_arm9_run_that_visits_the_bios_when_recorded_then_no_m_request_lands_in_the_area() {
    let (mut cpu, log) = adapter(TraceCpu::Arm9, arm9_bios_visit());
    drain(&mut cpu).unwrap();
    assert_eq!(
        reads_inside(&log, 0xFFFF_0000..0xFFFF_8000),
        Vec::<u32>::new()
    );
}

#[test]
fn given_an_arm7_run_that_visits_the_bios_when_recorded_then_no_m_request_lands_in_the_area() {
    let (mut cpu, log) = adapter(TraceCpu::Arm7, arm7_bios_visit());
    drain(&mut cpu).unwrap();
    assert_eq!(reads_inside(&log, 0..0x4000), Vec::<u32>::new());
}

#[test]
fn given_an_arm9_run_that_visits_the_bios_when_recorded_then_the_stub_is_asked_for_each_outside_opcode_once()
 {
    let (mut cpu, log) = adapter(TraceCpu::Arm9, arm9_bios_visit());
    drain(&mut cpu).unwrap();
    assert_eq!(
        read_addresses(&log),
        vec![0x0200_0000, 0x0200_0004, 0x0200_0008]
    );
}

#[test]
fn given_a_bios_step_when_collapsed_then_the_stub_is_stepped_once_per_instruction_in_the_script() {
    let (mut cpu, log) = adapter(TraceCpu::Arm9, arm9_bios_visit());
    drain(&mut cpu).unwrap();
    // Six instructions, so six `s` requests (the last one ends the target).
    assert_eq!(requests(&log, 's').len(), 6);
}

/// The ARM9 area's edges: `FFFF7FFCh` is inside, `FFFF8000h` is the first address outside.
#[test]
fn given_pcs_either_side_of_the_arm9_area_end_when_recorded_then_only_the_outside_one_is_read() {
    let script = vec![arm_at(0xFFFF_7FFC), arm_at(0xFFFF_8000)];
    let (mut cpu, log) = adapter(TraceCpu::Arm9, script);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        (pcs_and_steps(&records), read_addresses(&log)),
        (vec![(0xFFFF_8000, 0)], vec![0xFFFF_8000])
    );
}

#[test]
fn given_a_pc_just_below_the_arm9_area_when_recorded_then_it_is_read_and_recorded() {
    let script = vec![arm_at(0xFFFE_FFFC)];
    let (mut cpu, log) = adapter(TraceCpu::Arm9, script);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        (pcs_and_steps(&records), read_addresses(&log)),
        (vec![(0xFFFE_FFFC, 0)], vec![0xFFFE_FFFC])
    );
}

#[test]
fn given_pcs_either_side_of_the_arm7_area_end_when_recorded_then_only_the_outside_one_is_read() {
    let script = vec![arm_at(0x0000_3FFC), arm_at(0x0000_4000)];
    let (mut cpu, log) = adapter(TraceCpu::Arm7, script);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        (pcs_and_steps(&records), read_addresses(&log)),
        (vec![(0x0000_4000, 0)], vec![0x0000_4000])
    );
}

#[test]
fn given_an_arm7_pc_at_zero_when_recorded_then_it_is_collapsed_and_never_read() {
    let script = vec![arm_at(0x0000_0000), arm_at(0x0238_0000)];
    let (mut cpu, log) = adapter(TraceCpu::Arm7, script);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        (pcs_and_steps(&records), read_addresses(&log)),
        (vec![(0x0238_0000, 0)], vec![0x0238_0000])
    );
}

#[test]
fn given_the_arm9_bios_address_on_the_arm7_when_recorded_then_it_is_an_ordinary_read() {
    // `FFFF0018h` is only BIOS for the ARM9: the area is per CPU.
    let (mut cpu, log) = adapter(TraceCpu::Arm7, vec![arm_at(0xFFFF_0018)]);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        (records.len(), read_addresses(&log)),
        (1, vec![0xFFFF_0018])
    );
}

// ---- the end of the target ----

#[test]
fn given_the_last_instruction_when_it_is_taken_then_the_next_call_is_none_and_the_stub_hears_nothing_more()
 {
    let (mut cpu, log) = adapter(TraceCpu::Arm9, vec![arm_at(0x0200_0000)]);
    let first = cpu.next_record().unwrap();
    let heard = log.borrow().packets.len();
    let after = (cpu.next_record().unwrap(), cpu.next_record().unwrap());
    assert_eq!(
        (
            first.map(|r| r.pc()),
            after,
            log.borrow().packets.len() - heard
        ),
        (Some(0x0200_0000), (None, None), 0)
    );
}

#[test]
fn given_an_exit_inside_the_bios_when_recorded_then_the_run_ends_with_none() {
    let script = vec![arm_at(0x0200_0000), arm_at(0xFFFF_0018)];
    let (mut cpu, _) = adapter(TraceCpu::Arm9, script);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(pcs_and_steps(&records), vec![(0x0200_0000, 0)]);
}

#[test]
fn given_a_script_with_no_instruction_outside_the_bios_when_recorded_then_there_are_no_records() {
    let script = vec![arm_at(0xFFFF_0018), arm_at(0xFFFF_001C)];
    let (mut cpu, _) = adapter(TraceCpu::Arm9, script);
    assert_eq!(drain(&mut cpu).unwrap(), Vec::<TraceRecord>::new());
}

// ---- errors ----

#[test]
fn given_a_register_reply_with_16_words_when_recorded_then_the_register_count_is_wrong() {
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    // 16 words of zero: r0..r15 and no CPSR.
    let mut cpu = connect(TraceCpu::Arm9, fake.reply(b'g', "00".repeat(64).as_bytes()));
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::RegisterCount {
            words: 16,
            needed: 17
        })
    ));
}

#[test]
fn given_a_layout_that_puts_the_cpsr_past_the_reply_when_recorded_then_the_register_count_is_wrong()
{
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    let mut cpu = OracleCpu::connect(fake, TraceCpu::Arm9, RegisterLayout { cpsr: 20 });
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::RegisterCount {
            words: 17,
            needed: 21
        })
    ));
}

#[test]
fn given_an_empty_register_reply_when_recorded_then_the_stub_is_unsupported() {
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    let mut cpu = connect(TraceCpu::Arm9, fake.reply(b'g', b""));
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::Rsp(RspError::Unsupported))
    ));
}

#[test]
fn given_a_stub_that_replies_with_a_bad_checksum_forever_when_recorded_then_it_is_an_error_not_a_panic()
 {
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    let mut cpu = connect(TraceCpu::Arm9, fake.corrupt_replies(usize::MAX));
    assert!(matches!(cpu.next_record(), Err(AdapterError::Rsp(_))));
}

#[test]
fn given_a_stub_reply_that_is_malformed_when_recorded_then_it_is_an_error_not_a_panic() {
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    // A lone `}` with a correct checksum: a dangling escape.
    let mut cpu = connect(TraceCpu::Arm9, fake.reply(b'g', b"}"));
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::Rsp(RspError::Malformed(_)))
    ));
}

#[test]
fn given_a_stub_that_goes_silent_when_recorded_then_the_stream_is_closed() {
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    let mut cpu = connect(TraceCpu::Arm9, fake.mode(Mode::Silent));
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::Rsp(RspError::Closed))
    ));
}

#[test]
fn given_an_opcode_read_that_fails_when_recorded_then_the_stub_error_surfaces() {
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    let mut cpu = connect(TraceCpu::Arm9, fake.reply(b'm', b"E14"));
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::Rsp(RspError::Stub(0x14)))
    ));
}

#[test]
fn given_a_step_reply_that_is_garbage_when_recorded_then_it_is_an_error_not_a_panic() {
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    let mut cpu = connect(TraceCpu::Arm9, fake.reply(b's', b"zz"));
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::Rsp(RspError::Unexpected(_)))
    ));
}

// ---- error messages ----

#[test]
fn given_each_adapter_error_when_displayed_then_the_message_names_the_problem() {
    let messages = [
        AdapterError::StuckInBios.to_string(),
        AdapterError::RegisterCount {
            words: 16,
            needed: 17,
        }
        .to_string(),
        AdapterError::Rsp(RspError::Closed).to_string(),
    ];
    assert_eq!(
        messages,
        [
            "the CPU stayed in its BIOS area for 16777216 steps",
            "the stub sent 16 register words, the layout needs 17",
            "the stub closed the connection",
        ]
    );
}

#[test]
fn given_an_rsp_error_when_wrapped_by_the_adapter_then_it_is_its_source() {
    use std::error::Error;
    let error = AdapterError::from(RspError::Closed);
    assert_eq!(
        (
            error.source().map(ToString::to_string),
            AdapterError::StuckInBios.source().is_none()
        ),
        (Some("the stub closed the connection".to_string()), true)
    );
}

// ---- the produced trace ----

/// The records printed one per line and parsed back, with those that did not survive.
fn print_parse_failures(records: &[TraceRecord]) -> Vec<(TraceRecord, String)> {
    records
        .iter()
        .map(|record| (*record, format!("{record}")))
        .filter(|(record, line)| parse_line(line).as_ref() != Ok(record))
        .collect()
}

#[test]
fn given_a_mixed_arm_and_thumb_run_when_the_records_are_printed_then_each_parses_back_unchanged() {
    let script = vec![
        State::at(0x0200_0000).reg(0, 0xDEAD_BEEF).arm(0xE3A0_0055),
        State::at(0x0200_0004).reg(1, 0x8000_0000).thumb(0x2166),
        State::at(0x0200_0006).thumb(0x4770),
    ];
    let (mut cpu, _) = adapter(TraceCpu::Arm9, script);
    let records = drain(&mut cpu).unwrap();
    assert_eq!(
        (records.len(), print_parse_failures(&records)),
        (3, Vec::<(TraceRecord, String)>::new())
    );
}

#[test]
fn given_a_known_script_when_the_first_record_is_printed_then_it_is_the_hand_written_line() {
    let script = vec![State::at(0x0200_0000).arm(0xE3A0_0055)];
    let (mut cpu, _) = adapter(TraceCpu::Arm9, script);
    let line = format!("{}", cpu.next_record().unwrap().unwrap());
    assert_eq!(
        line,
        "9 00000000 02000000 e3a00055 00000000 00000000 00000000 00000000 \
00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
00000000 02000000 0000001f"
    );
}

// ---- a BIOS call that never returns ----

/// `count` consecutive states inside the ARM9 BIOS area.
fn bios_states(count: u32) -> Vec<State> {
    (0..count).map(|i| arm_at(0xFFFF_0018 + 4 * i)).collect()
}

#[test]
fn given_a_cpu_that_stays_in_its_bios_area_when_the_limit_is_passed_then_it_is_stuck_in_bios() {
    let (cpu, _log) = adapter(TraceCpu::Arm9, bios_states(10));
    let mut cpu = cpu.with_bios_limit(3);
    assert!(matches!(cpu.next_record(), Err(AdapterError::StuckInBios)));
}

#[test]
fn given_a_cpu_that_stays_in_its_bios_area_when_the_limit_is_passed_then_no_memory_was_read() {
    let (cpu, log) = adapter(TraceCpu::Arm9, bios_states(10));
    let mut cpu = cpu.with_bios_limit(3);
    let _ = cpu.next_record();
    assert_eq!(read_addresses(&log), Vec::<u32>::new());
}

#[test]
fn given_a_layout_with_the_cpsr_among_the_first_words_when_the_reply_is_short_then_it_is_an_error()
{
    // A layout that says the CPSR is word 3 must not let a 4-word reply reach r15.
    let (fake, _) = Fake::new(vec![arm_at(0x0200_0000)]);
    let mut cpu = OracleCpu::connect(
        fake.reply(b'g', "00".repeat(16).as_bytes()),
        TraceCpu::Arm9,
        RegisterLayout { cpsr: 3 },
    );
    assert!(matches!(
        cpu.next_record(),
        Err(AdapterError::RegisterCount {
            words: 4,
            needed: 16
        })
    ));
}
