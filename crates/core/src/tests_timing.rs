// SPDX-License-Identifier: Apache-2.0
//! Display timing driving both CPUs through `Machine`. Expected values come from
//! `docs/hw/display-timing.md` ("Frame layout", "VCOUNT", "Flags", "Interrupt requests",
//! "Edge cases") and `docs/hw/scheduler.md` ("Frame timing"), never from running the code.
//!
//! Numbers used, from the note: a line is 4,260 master cycles; there are 263 lines; the VBlank
//! flag is set from the start of line 192 until the start of line 262; the HBlank flag sets
//! 3,212 master cycles into a line and clears at the next line start; DISPSTAT's writable bits
//! are 3, 4, 5, 7 and 8-15 (`FFB8h`).
//!
//! The programs are `b .` on both CPUs. `run_to(t)` runs rounds until the master clock is `t`;
//! the events due at `t` have fired when it returns (`scheduler.md`, "Interleaving").

use super::*;
use crate::cart::test_image::{Image, words};
use crate::scheduler::{EventKind, FRAME_CYCLES, SLICE, run_round};

const LINE: u64 = 4_260;
const HBLANK_AT: u64 = 3_212;
const DISPSTAT: u32 = 0x0400_0004;
const VCOUNT: u32 = 0x0400_0006;
const IME: u32 = 0x0400_0208;
const IE: u32 = 0x0400_0210;
const VBLANK_IRQ: u32 = 1 << 0;
const HBLANK_IRQ: u32 = 1 << 1;
const VCOUNT_IRQ: u32 = 1 << 2;
const VBLANK_FLAG: u16 = 1 << 0;
const HBLANK_FLAG: u16 = 1 << 1;
const VCOUNT_FLAG: u16 = 1 << 2;
/// DISPSTAT enable bits 3-5.
const ENABLE_VBLANK: u16 = 1 << 3;
const ENABLE_HBLANK: u16 = 1 << 4;
const ENABLE_VCOUNT: u16 = 1 << 5;
const IRQ_MODE: u32 = 0x12;

fn plain() -> TcmConfig {
    TcmConfig::from_cp15(Machine::new().arm9.cp15())
}

fn machine() -> Machine {
    let mut image = Image::valid();
    image.arm9_code = words(&[0xEAFF_FFFE]);
    image.arm7_code = words(&[0xEAFF_FFFE]);
    let mut machine = Machine::new();
    machine.load_rom(image.build()).expect("synthetic image");
    machine
}

/// Runs until the master clock is exactly `time`; events due at `time` have fired. Whole rounds
/// while a full slice fits, then single steps, so the clock never passes `time`.
fn run_to(m: &mut Machine, time: u64) {
    while m.scheduler.now() + SLICE <= time {
        run_round(m);
    }
    while m.scheduler.now() < time {
        m.step_arm9();
    }
}

fn write9(m: &mut Machine, addr: u32, value: u16) {
    m.memory.write16(&plain(), addr, value);
}

fn read9(m: &Machine, addr: u32) -> u16 {
    m.memory.read16(&plain(), addr)
}

/// A line number's start time.
fn line_start(line: u64) -> u64 {
    line * LINE
}

/// One machine run forward through `times` (ascending); what `read` shows at each.
fn observe(times: &[u64], read: impl Fn(&Machine) -> u16) -> Vec<u16> {
    let mut m = machine();
    times
        .iter()
        .map(|&time| {
            run_to(&mut m, time);
            read(&m)
        })
        .collect()
}

// ------------------------------------------------------------------ events and VCOUNT

#[test]
fn given_a_booted_machine_when_nothing_ran_then_the_pending_events_are_hblank_then_the_next_line() {
    let pending: Vec<(u64, EventKind)> = machine()
        .scheduler()
        .pending()
        .iter()
        .map(|e| (e.time, e.kind))
        .collect();
    assert_eq!(
        pending,
        vec![
            (HBLANK_AT, EventKind::HBlankStart),
            (LINE, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_a_booted_machine_when_reading_vcount_at_each_line_start_of_a_frame_then_it_counts_0_to_262_and_wraps()
 {
    let times: Vec<u64> = (0..=263).map(line_start).collect();
    let expected: Vec<u16> = (0..263).chain(0..1).collect();
    assert_eq!(observe(&times, |m| m.vcount()), expected);
}

#[test]
fn given_a_booted_machine_when_reading_vcount_one_cycle_before_each_line_start_then_it_has_not_advanced()
 {
    let times: Vec<u64> = (1..=263).map(|n| line_start(n) - 1).collect();
    let expected: Vec<u16> = (0..263).collect();
    assert_eq!(observe(&times, |m| m.vcount()), expected);
}

#[test]
fn given_a_booted_machine_when_one_frame_runs_then_it_ends_at_the_frame_length_and_counts_once() {
    let mut m = machine();
    m.run_frame();
    assert_eq!(
        (m.scheduler().now(), m.frame_count(), FRAME_CYCLES),
        (line_start(263), 1, 1_120_380)
    );
}

#[test]
fn given_a_frame_run_when_listing_pending_then_line_1_events_follow_the_wrap_from_their_own_times()
{
    let mut m = machine();
    m.run_frame();
    let pending: Vec<(u64, EventKind)> = m
        .scheduler()
        .pending()
        .iter()
        .map(|e| (e.time, e.kind))
        .collect();
    assert_eq!(
        pending,
        vec![
            (FRAME_CYCLES + HBLANK_AT, EventKind::HBlankStart),
            (FRAME_CYCLES + LINE, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_the_last_line_when_the_frame_wraps_then_vcount_is_zero_and_the_screens_are_drawn_once() {
    let mut m = machine();
    run_to(&mut m, line_start(262));
    let before = (m.vcount(), m.frame_count());
    run_to(&mut m, line_start(263));
    assert_eq!((before, (m.vcount(), m.frame_count())), ((262, 0), (0, 1)));
}

#[test]
fn given_two_frames_when_run_then_the_second_frame_ends_at_twice_the_frame_length() {
    let mut m = machine();
    m.run_frame();
    m.run_frame();
    assert_eq!(
        (m.scheduler().now(), m.frame_count()),
        (2 * FRAME_CYCLES, 2)
    );
}

// ------------------------------------------------------------------ flags

#[test]
fn given_a_booted_machine_when_reading_the_vblank_flag_around_lines_192_and_262_then_it_is_set_for_192_to_261()
 {
    let times = [
        line_start(192) - 1,
        line_start(192),
        line_start(262) - 1,
        line_start(262),
    ];
    let flags = observe(&times, |m| m.dispstat9() & VBLANK_FLAG);
    assert_eq!(flags, vec![0, 1, 1, 0]);
}

#[test]
fn given_a_booted_machine_when_reading_the_hblank_flag_in_line_1_then_it_sets_3212_cycles_in_and_clears_at_the_next_line()
 {
    let times = [
        line_start(1) + HBLANK_AT - 1,
        line_start(1) + HBLANK_AT,
        line_start(2) - 1,
        line_start(2),
    ];
    let flags = observe(&times, |m| (m.dispstat9() & HBLANK_FLAG) >> 1);
    assert_eq!(flags, vec![0, 1, 1, 0]);
}

#[test]
fn given_a_booted_machine_when_reading_the_hblank_flag_in_a_vblank_line_then_it_toggles_there_too()
{
    let times = [
        line_start(200) + HBLANK_AT - 1,
        line_start(200) + HBLANK_AT,
        line_start(201),
    ];
    let flags = observe(&times, |m| (m.dispstat9() & HBLANK_FLAG) >> 1);
    assert_eq!(flags, vec![0, 1, 0]);
}

#[test]
fn given_the_vcount_setting_100_when_reading_the_match_flag_around_line_100_then_it_is_set_only_in_that_line()
 {
    let mut m = machine();
    write9(&mut m, DISPSTAT, 100 << 8);
    let times = [line_start(99), line_start(100), line_start(101)];
    let flags: Vec<u16> = times
        .iter()
        .map(|&t| {
            run_to(&mut m, t);
            (m.dispstat9() & VCOUNT_FLAG) >> 2
        })
        .collect();
    assert_eq!(flags, vec![0, 1, 0]);
}

#[test]
fn given_a_vblank_flag_set_when_the_arm7_reads_dispstat_then_it_sees_the_same_flag_through_its_own_view()
 {
    let mut m = machine();
    run_to(&mut m, line_start(200) + HBLANK_AT);
    assert_eq!(
        (m.memory.read16_arm7(0, DISPSTAT), m.dispstat7()),
        (VBLANK_FLAG | HBLANK_FLAG, VBLANK_FLAG | HBLANK_FLAG)
    );
}

// ------------------------------------------------------------------ DISPSTAT access

#[test]
fn given_the_arm9_when_writing_ffff_to_dispstat_then_only_the_enables_and_the_setting_stay() {
    let mut m = machine();
    write9(&mut m, DISPSTAT, 0xFFFF);
    assert_eq!(read9(&m, DISPSTAT), 0xFFB8);
}

#[test]
fn given_the_arm7_when_writing_ffff_to_dispstat_then_only_the_enables_and_the_setting_stay() {
    let mut m = machine();
    m.memory.write16_arm7(0, DISPSTAT, 0xFFFF);
    assert_eq!(m.memory.read16_arm7(0, DISPSTAT), 0xFFB8);
}

#[test]
fn given_the_arm9_when_writing_ones_to_the_flag_bits_and_bit_6_then_dispstat_stays_zero() {
    let mut m = machine();
    write9(&mut m, DISPSTAT, 0x0047);
    assert_eq!(read9(&m, DISPSTAT), 0);
}

#[test]
fn given_the_arm9_when_writing_dispstat_then_the_arm7_register_is_unchanged() {
    let mut m = machine();
    write9(&mut m, DISPSTAT, 0xFFFF);
    assert_eq!(m.memory.read16_arm7(0, DISPSTAT), 0);
}

#[test]
fn given_the_arm7_when_writing_dispstat_then_the_arm9_register_is_unchanged() {
    let mut m = machine();
    m.memory.write16_arm7(0, DISPSTAT, 0xFFFF);
    assert_eq!(read9(&m, DISPSTAT), 0);
}

#[test]
fn given_a_byte_write_to_the_high_byte_when_reading_dispstat_then_it_holds_the_low_setting_bits() {
    let mut m = machine();
    m.memory.write8(&plain(), DISPSTAT + 1, 0xAB);
    assert_eq!(read9(&m, DISPSTAT), 0xAB00);
}

#[test]
fn given_a_setting_of_262_when_written_as_bit_7_and_the_low_byte_then_dispstat_reads_it_back() {
    let mut m = machine();
    write9(&mut m, DISPSTAT, 0x0680);
    assert_eq!(read9(&m, DISPSTAT), 0x0680);
}

// ------------------------------------------------------------------ VCOUNT access

#[test]
fn given_line_200_when_each_cpu_reads_vcount_then_both_see_200() {
    let mut m = machine();
    run_to(&mut m, line_start(200));
    assert_eq!(
        (read9(&m, VCOUNT), m.memory.read16_arm7(0, VCOUNT)),
        (200, 200)
    );
}

#[test]
fn given_line_262_when_each_cpu_reads_vcount_then_the_ninth_bit_shows_on_both() {
    let mut m = machine();
    run_to(&mut m, line_start(262));
    assert_eq!(
        (read9(&m, VCOUNT), m.memory.read16_arm7(0, VCOUNT)),
        (262, 262)
    );
}

#[test]
fn given_line_3_when_each_cpu_writes_vcount_then_it_is_ignored_as_the_note_stubs_it_unverified() {
    let mut m = machine();
    run_to(&mut m, line_start(3));
    write9(&mut m, VCOUNT, 100);
    m.memory.write16_arm7(0, VCOUNT, 50);
    assert_eq!(m.vcount(), 3);
}

// ------------------------------------------------------------------ IRQ requests

/// (IF of the ARM9, IF of the ARM7) at `time` after `setup` ran on a fresh machine.
fn ifs_at(time: u64, setup: impl Fn(&mut Machine)) -> (u32, u32) {
    let mut m = machine();
    setup(&mut m);
    run_to(&mut m, time);
    (m.irq9().if_(), m.irq7().if_())
}

#[test]
fn given_the_arm9_vblank_enable_when_line_192_starts_then_only_the_arm9_latches_the_vblank_request()
{
    let ifs = ifs_at(line_start(192), |m| write9(m, DISPSTAT, ENABLE_VBLANK));
    assert_eq!(ifs, (VBLANK_IRQ, 0));
}

#[test]
fn given_the_arm7_vblank_enable_when_line_192_starts_then_only_the_arm7_latches_the_vblank_request()
{
    let ifs = ifs_at(line_start(192), |m| {
        m.memory.write16_arm7(0, DISPSTAT, ENABLE_VBLANK);
    });
    assert_eq!(ifs, (0, VBLANK_IRQ));
}

#[test]
fn given_no_vblank_enable_when_line_192_starts_then_neither_cpu_latches_a_request() {
    assert_eq!(ifs_at(line_start(192), |_| {}), (0, 0));
}

#[test]
fn given_the_vblank_enable_when_one_cycle_before_line_192_then_no_request_is_latched() {
    let ifs = ifs_at(line_start(192) - 1, |m| write9(m, DISPSTAT, ENABLE_VBLANK));
    assert_eq!(ifs, (0, 0));
}

#[test]
fn given_the_vblank_enable_when_a_whole_frame_runs_then_the_request_is_latched_once_not_every_line()
{
    let mut m = machine();
    write9(&mut m, DISPSTAT, ENABLE_VBLANK);
    run_to(&mut m, line_start(192));
    m.memory.irq9_mut().write_byte(0x0400_0214, 0x01);
    run_to(&mut m, line_start(263));
    assert_eq!(m.irq9().if_(), 0);
}

#[test]
fn given_the_hblank_enable_when_line_0_reaches_its_hblank_then_both_cpus_with_the_enable_latch_it()
{
    let ifs = ifs_at(HBLANK_AT, |m| {
        write9(m, DISPSTAT, ENABLE_HBLANK);
        m.memory.write16_arm7(0, DISPSTAT, ENABLE_HBLANK);
    });
    assert_eq!(ifs, (HBLANK_IRQ, HBLANK_IRQ));
}

#[test]
fn given_the_hblank_enable_when_one_cycle_before_the_hblank_then_no_request_is_latched() {
    let ifs = ifs_at(HBLANK_AT - 1, |m| write9(m, DISPSTAT, ENABLE_HBLANK));
    assert_eq!(ifs, (0, 0));
}

#[test]
fn given_the_hblank_enable_when_a_vblank_line_reaches_its_hblank_then_the_request_is_latched_unverified()
 {
    let ifs = ifs_at(line_start(200) + HBLANK_AT, |m| {
        write9(m, DISPSTAT, ENABLE_HBLANK);
    });
    assert_eq!(ifs, (HBLANK_IRQ, 0));
}

#[test]
fn given_the_setting_100_and_the_vcount_enable_when_line_100_starts_then_the_request_is_latched() {
    let ifs = ifs_at(line_start(100), |m| {
        write9(m, DISPSTAT, (100 << 8) | ENABLE_VCOUNT);
    });
    assert_eq!(ifs, (VCOUNT_IRQ, 0));
}

#[test]
fn given_the_setting_100_and_the_vcount_enable_when_line_99_starts_then_no_request_is_latched() {
    let ifs = ifs_at(line_start(99), |m| {
        write9(m, DISPSTAT, (100 << 8) | ENABLE_VCOUNT);
    });
    assert_eq!(ifs, (0, 0));
}

#[test]
fn given_the_setting_100_without_the_enable_when_line_100_starts_then_the_flag_is_set_but_no_request_is_latched()
 {
    let mut m = machine();
    write9(&mut m, DISPSTAT, 100 << 8);
    run_to(&mut m, line_start(100));
    assert_eq!(
        (read9(&m, DISPSTAT) & VCOUNT_FLAG, m.irq9().if_()),
        (VCOUNT_FLAG, 0)
    );
}

#[test]
fn given_the_setting_262_and_the_vcount_enable_when_line_262_starts_then_the_request_is_latched() {
    let ifs = ifs_at(line_start(262), |m| {
        write9(m, DISPSTAT, 0x0680 | ENABLE_VCOUNT);
    });
    assert_eq!(ifs, (VCOUNT_IRQ, 0));
}

#[test]
fn given_the_setting_0_and_the_vcount_enable_when_the_frame_wraps_to_line_0_then_the_request_is_latched()
 {
    let ifs = ifs_at(line_start(263), |m| write9(m, DISPSTAT, ENABLE_VCOUNT));
    assert_eq!(ifs, (VCOUNT_IRQ, 0));
}

#[test]
fn given_the_arm7_vblank_request_with_ime_and_ie_when_the_arm7_runs_on_then_it_enters_irq_mode() {
    let mut m = machine();
    m.memory.write16_arm7(0, DISPSTAT, ENABLE_VBLANK);
    m.memory.write32_arm7(0, IE, VBLANK_IRQ);
    m.memory.write32_arm7(0, IME, 1);
    m.arm7.regs_mut().set_cpsr(0x1F);
    run_to(&mut m, line_start(192));
    run_round(&mut m);
    assert_eq!(m.arm7().regs().cpsr() & 0x1F, IRQ_MODE);
}

#[test]
fn given_the_arm9_vblank_request_with_ime_and_ie_when_the_arm9_runs_on_then_it_enters_irq_mode() {
    let mut m = machine();
    write9(&mut m, DISPSTAT, ENABLE_VBLANK);
    m.memory.write32(&plain(), IE, VBLANK_IRQ);
    m.memory.write32(&plain(), IME, 1);
    m.arm9.regs_mut().set_cpsr(0x1F);
    run_to(&mut m, line_start(192));
    run_round(&mut m);
    assert_eq!(m.arm9().regs().cpsr() & 0x1F, IRQ_MODE);
}

// ------------------------------------------------------------------ savestates mid-frame

/// Whether a machine saved at `time` and resumed on a fresh one matches an uninterrupted run
/// once both reach `until`, in the saved bytes.
fn resumed_matches(time: u64, until: u64) -> bool {
    let setup = |m: &mut Machine| {
        write9(
            m,
            DISPSTAT,
            (150 << 8) | ENABLE_VBLANK | ENABLE_HBLANK | ENABLE_VCOUNT,
        );
    };
    let mut uninterrupted = machine();
    setup(&mut uninterrupted);
    run_to(&mut uninterrupted, until);

    let mut saver = machine();
    setup(&mut saver);
    run_to(&mut saver, time);
    let mut resumed = machine();
    let loaded = resumed.load_state(&saver.save_state());
    run_to(&mut resumed, until);
    loaded.is_ok() && resumed.save_state() == uninterrupted.save_state()
}

#[test]
fn given_a_save_inside_hblank_of_line_150_when_resumed_then_it_matches_an_uninterrupted_run() {
    assert!(resumed_matches(
        line_start(150) + HBLANK_AT + 40,
        line_start(152)
    ));
}

#[test]
fn given_a_save_inside_vblank_when_resumed_then_it_matches_an_uninterrupted_run_across_the_wrap() {
    assert!(resumed_matches(line_start(200) + 100, line_start(265)));
}

#[test]
fn given_a_save_mid_frame_when_loaded_then_vcount_and_dispstat_are_restored() {
    let mut saver = machine();
    write9(&mut saver, DISPSTAT, (150 << 8) | ENABLE_VCOUNT);
    run_to(&mut saver, line_start(150) + HBLANK_AT + 40);
    let mut resumed = machine();
    let loaded = resumed.load_state(&saver.save_state());
    assert_eq!(
        (loaded, resumed.vcount(), resumed.dispstat9()),
        (
            Ok(()),
            150,
            (150 << 8) | ENABLE_VCOUNT | VCOUNT_FLAG | HBLANK_FLAG
        )
    );
}

#[test]
fn given_two_runs_of_the_same_frame_when_each_ends_then_the_savestate_bytes_match() {
    let (mut a, mut b) = (machine(), machine());
    a.run_frame();
    b.run_frame();
    assert_eq!(a.save_state(), b.save_state());
}
