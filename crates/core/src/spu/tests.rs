// SPDX-License-Identifier: Apache-2.0
//! The sound registers as a device. Expected values come from `docs/hw/sound-registers.md`
//! ("Registers", "The status bit of a channel", "Model"), worked out by hand, never from running
//! the code.
//!
//! With TMR `FFF0h` a sample takes `4 * 16 = 64` master cycles, so a one-shot of PNT + LEN = 4
//! words lasts 16 PCM8 samples (1,024 cycles), 8 PCM16 samples (512), and, with the header word, 4
//! words of ADPCM are `8 * 3 = 24` samples (1,536).

use super::*;
use crate::io::device::cx_at;

/// TMR `FFF0h`: 16 steps of 4 master cycles.
const TMR: u32 = 0xFFF0;
/// Channel `x`'s register `reg` in the device's offsets.
const fn ch(x: u32, reg: u32) -> u32 {
    (x << 4) + reg
}
/// SOUNDxCNT with the given repeat mode and format and the start bit set.
const fn start(repeat: u32, format: u32) -> u32 {
    CNT_START | repeat << 27 | format << 29
}
const LOOP: u32 = 1;
const ONE_SHOT: u32 = 2;
const PCM8: u32 = 0;
const PCM16: u32 = 1;
const ADPCM: u32 = 2;
const PSG: u32 = 3;

fn write(spu: &mut Spu, offset: u32, width: Width, value: u32, now: u64) {
    spu.write(offset, width, value, cx_at(now));
}

/// `value` written at each of `offsets` as a `width` access at time 0.
fn write_each(spu: &mut Spu, offsets: &[u32], width: Width, value: u32) {
    offsets.iter().for_each(|&o| write(spu, o, width, value, 0));
}

fn read(spu: &Spu, offset: u32, width: Width, now: u64) -> u32 {
    spu.peek(offset, width, cx_at(now))
}

/// A device with channel `x` set to `tmr`, `pnt`, `len` and then started with `cnt` at `now`.
fn started(x: u32, cnt: u32, pnt: u32, len: u32, now: u64) -> Spu {
    let mut spu = Spu::new();
    write(&mut spu, ch(x, TMR_OFFSET), Width::Half, TMR, now);
    write(&mut spu, ch(x, PNT_OFFSET), Width::Half, pnt, now);
    write(&mut spu, ch(x, LEN_OFFSET), Width::Word, len, now);
    write(&mut spu, ch(x, CNT_OFFSET), Width::Word, cnt, now);
    spu
}

/// Whether channel `x`'s busy bit reads set at `now`.
fn busy(spu: &Spu, x: u32, now: u64) -> bool {
    read(spu, ch(x, CNT_OFFSET), Width::Word, now) & CNT_START != 0
}

/// Whether capture `x`'s busy bit reads set at `now`.
fn cap_busy(spu: &Spu, x: u32, now: u64) -> bool {
    let offset = SNDCAP0CNT_OFFSET + x;
    read(spu, offset, Width::Byte, now) & u32::from(CAP_START) != 0
}

/// Whether a started channel reads (busy just before `end`, busy at `end`).
fn busy_around(spu: &Spu, x: u32, end: u64) -> (bool, bool) {
    (busy(spu, x, end - 1), busy(spu, x, end))
}

/// The formats (name, format, PNT + LEN) whose one-shot busy bit does not read (set, clear) around
/// the end the note gives.
fn one_shot_failures() -> Vec<(&'static str, (bool, bool))> {
    [
        ("pcm8", PCM8, 1_024),
        ("pcm16", PCM16, 512),
        ("adpcm", ADPCM, 1_536),
    ]
    .iter()
    .map(|&(name, format, ticks)| {
        let spu = started(0, start(ONE_SHOT, format), 0, 4, 1_000);
        (name, busy_around(&spu, 0, 1_000 + ticks))
    })
    .filter(|&(_, around)| around != (true, false))
    .collect()
}

#[test]
fn given_a_one_shot_of_each_format_when_the_end_time_is_reached_then_the_busy_bit_clears_exactly_there()
 {
    assert_eq!(one_shot_failures(), Vec::new());
}

#[test]
fn given_a_slower_timer_when_a_pcm8_one_shot_runs_then_a_sample_takes_four_cycles_per_step() {
    // TMR 0: 4 * 10000h = 262,144 cycles per sample; 4 words of PCM8 are 16 samples.
    let mut spu = Spu::new();
    write(&mut spu, ch(2, LEN_OFFSET), Width::Word, 4, 0);
    write(
        &mut spu,
        ch(2, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, PCM8),
        0,
    );
    assert_eq!(busy_around(&spu, 2, 16 * 262_144), (true, false));
}

#[test]
fn given_a_one_shot_with_pnt_and_len_when_started_then_the_sum_sets_the_length() {
    // PNT 1 + LEN 3 = 4 words of PCM8: the same 16 samples as LEN 4 alone.
    let spu = started(0, start(ONE_SHOT, PCM8), 1, 3, 0);
    assert_eq!(busy_around(&spu, 0, 1_024), (true, false));
}

#[test]
fn given_a_one_shot_shorter_than_four_words_when_much_time_passes_then_it_stays_busy() {
    let spu = started(0, start(ONE_SHOT, PCM8), 1, 2, 0);
    assert!(busy(&spu, 0, 1 << 60));
}

#[test]
fn given_a_loop_channel_when_much_time_passes_then_it_stays_busy() {
    let spu = started(0, start(LOOP, PCM8), 0, 4, 0);
    assert!(busy(&spu, 0, 1 << 60));
}

#[test]
fn given_a_psg_channel_when_much_time_passes_then_it_stays_busy() {
    let spu = started(8, start(ONE_SHOT, PSG), 0, 4, 0);
    assert!(busy(&spu, 8, 1 << 60));
}

#[test]
fn given_the_manual_and_prohibited_repeat_modes_when_much_time_passes_then_both_stay_busy() {
    let modes = [0, 3].map(|mode| {
        let spu = started(0, start(mode, PCM8), 0, 4, 0);
        busy(&spu, 0, 1 << 60)
    });
    assert_eq!(modes, [true, true]);
}

#[test]
fn given_a_busy_channel_when_a_zero_is_written_to_bit_31_then_it_stops_at_once() {
    let mut spu = started(0, start(LOOP, PCM8), 0, 4, 0);
    write(&mut spu, ch(0, CNT_OFFSET + 3), Width::Byte, 0, 10);
    assert_eq!((busy(&spu, 0, 10), spu.channel(0).end()), (false, 0));
}

#[test]
fn given_a_busy_channel_when_start_is_written_again_then_the_end_does_not_move() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(
        &mut spu,
        ch(0, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, PCM8),
        500,
    );
    assert_eq!(spu.channel(0).end(), 1_024);
}

#[test]
fn given_an_ended_one_shot_when_start_is_written_again_then_it_runs_again_from_that_time() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(
        &mut spu,
        ch(0, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, PCM8),
        2_000,
    );
    assert_eq!(busy_around(&spu, 0, 2_000 + 1_024), (true, false));
}

#[test]
fn given_an_ended_one_shot_when_a_low_byte_is_written_then_the_stale_start_bit_is_folded_away() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(&mut spu, ch(0, CNT_OFFSET), Width::Byte, 0x40, 5_000);
    assert_eq!(spu.channel(0).cnt() & CNT_START, 0);
}

#[test]
fn given_a_busy_channel_when_tmr_is_written_then_the_end_is_not_recomputed() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(&mut spu, ch(0, TMR_OFFSET), Width::Half, 0, 1);
    assert_eq!(spu.channel(0).end(), 1_024);
}

/// The registers written with all ones, as (name, address in the device, width, what a read
/// returns, according to the note) rows that read differently.
fn read_back_failures() -> Vec<(&'static str, u32)> {
    let rows: [(&str, u32, Width, u32); 8] = [
        ("SOUNDxCNT", ch(5, CNT_OFFSET), Width::Word, 0xFF7F_837F),
        ("SOUNDCNT", SOUNDCNT_OFFSET, Width::Word, 0x0000_BF7F),
        ("SOUNDBIAS", SOUNDBIAS_OFFSET, Width::Word, 0x0000_03FF),
        ("SNDCAP0CNT", SNDCAP0CNT_OFFSET, Width::Byte, 0x8F),
        ("SNDCAP1CNT", SNDCAP1CNT_OFFSET, Width::Byte, 0x8F),
        ("SNDCAP0DAD", SNDCAP0DAD_OFFSET, Width::Word, 0x07FF_FFFC),
        ("SNDCAP1DAD", SNDCAP1DAD_OFFSET, Width::Word, 0x07FF_FFFC),
        ("SOUNDCNT bytes", SOUNDCNT_OFFSET, Width::Half, 0xBF7F),
    ];
    rows.iter()
        .filter_map(|&(name, offset, width, expected)| {
            let mut spu = Spu::new();
            write(&mut spu, offset, width, u32::MAX, 0);
            let actual = read(&spu, offset, width, 0);
            (actual != expected).then_some((name, actual))
        })
        .collect()
}

#[test]
fn given_all_ones_written_to_each_readable_register_when_read_back_then_only_its_bits_remain() {
    assert_eq!(read_back_failures(), Vec::new());
}

#[test]
fn given_all_ones_written_to_the_write_only_registers_when_read_then_each_reads_zero() {
    let offsets = [
        ch(3, SAD_OFFSET),
        ch(3, TMR_OFFSET),
        ch(3, PNT_OFFSET),
        ch(3, LEN_OFFSET),
        SNDCAP0LEN_OFFSET,
        SNDCAP1LEN_OFFSET,
    ];
    let mut spu = Spu::new();
    write_each(&mut spu, &offsets, Width::Word, u32::MAX);
    let values = offsets.map(|o| read(&spu, o, Width::Word, 0));
    assert_eq!(values, [0; 6]);
}

#[test]
fn given_all_ones_written_to_the_write_only_registers_when_inspected_then_the_masked_values_are_kept()
 {
    let mut spu = Spu::new();
    write(&mut spu, ch(3, SAD_OFFSET), Width::Word, u32::MAX, 0);
    write(&mut spu, ch(3, TMR_OFFSET), Width::Word, u32::MAX, 0);
    write(&mut spu, ch(3, LEN_OFFSET), Width::Word, u32::MAX, 0);
    write(&mut spu, SNDCAP1LEN_OFFSET, Width::Word, u32::MAX, 0);
    let c = spu.channel(3);
    assert_eq!(
        (
            c.sad(),
            c.tmr(),
            c.pnt(),
            c.length(),
            spu.capture(1).length()
        ),
        (0x07FF_FFFC, 0xFFFF, 0xFFFF, 0x003F_FFFF, 0xFFFF)
    );
}

#[test]
fn given_a_byte_written_into_a_wide_register_when_read_then_only_that_byte_changes() {
    let mut spu = Spu::new();
    write(&mut spu, SOUNDBIAS_OFFSET, Width::Half, 0x0200, 0);
    write(&mut spu, SOUNDBIAS_OFFSET, Width::Byte, 0x01, 0);
    assert_eq!(read(&spu, SOUNDBIAS_OFFSET, Width::Word, 0), 0x0201);
}

#[test]
fn given_unused_and_gap_addresses_when_written_and_read_then_they_read_zero() {
    let offsets = [0x10A, 0x10B, 0x10F, 0x120, 0x13F];
    let mut spu = Spu::new();
    write_each(&mut spu, &offsets, Width::Byte, 0xFF);
    assert_eq!(offsets.map(|o| read(&spu, o, Width::Byte, 0)), [0; 5]);
}

#[test]
fn given_a_capture_of_zero_words_when_one_shot_pcm16_then_it_lasts_one_word_at_channel_1s_rate() {
    // 1 word = 2 samples of 64 cycles.
    let mut spu = Spu::new();
    write(&mut spu, ch(1, TMR_OFFSET), Width::Half, TMR, 0);
    write(
        &mut spu,
        SNDCAP0CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START | CAP_ONE_SHOT),
        100,
    );
    assert_eq!(
        (cap_busy(&spu, 0, 227), cap_busy(&spu, 0, 228)),
        (true, false)
    );
}

#[test]
fn given_a_pcm8_capture_of_two_words_when_one_shot_then_it_lasts_eight_samples() {
    let mut spu = Spu::new();
    write(&mut spu, ch(1, TMR_OFFSET), Width::Half, TMR, 0);
    write(&mut spu, SNDCAP0LEN_OFFSET, Width::Half, 2, 0);
    let cnt = u32::from(CAP_START | CAP_ONE_SHOT | CAP_PCM8);
    write(&mut spu, SNDCAP0CNT_OFFSET, Width::Byte, cnt, 0);
    assert_eq!(
        (cap_busy(&spu, 0, 511), cap_busy(&spu, 0, 512)),
        (true, false)
    );
}

#[test]
fn given_capture_1_when_one_shot_then_it_runs_at_channel_3s_rate_not_channel_1s() {
    let mut spu = Spu::new();
    write(&mut spu, ch(1, TMR_OFFSET), Width::Half, 0, 0);
    write(&mut spu, ch(3, TMR_OFFSET), Width::Half, TMR, 0);
    write(
        &mut spu,
        SNDCAP1CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START | CAP_ONE_SHOT),
        0,
    );
    assert_eq!(
        (cap_busy(&spu, 1, 127), cap_busy(&spu, 1, 128)),
        (true, false)
    );
}

#[test]
fn given_a_loop_capture_when_much_time_passes_then_it_stays_busy() {
    let mut spu = Spu::new();
    write(
        &mut spu,
        SNDCAP0CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START),
        0,
    );
    assert!(cap_busy(&spu, 0, 1 << 60));
}

#[test]
fn given_a_busy_capture_when_a_zero_is_written_to_bit_7_then_it_stops_at_once() {
    let mut spu = Spu::new();
    write(
        &mut spu,
        SNDCAP0CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START),
        0,
    );
    write(&mut spu, SNDCAP0CNT_OFFSET, Width::Byte, 0, 10);
    assert_eq!((cap_busy(&spu, 0, 10), spu.capture(0).end()), (false, 0));
}

#[test]
fn given_a_slowest_timer_and_the_longest_length_when_started_then_the_end_does_not_wrap() {
    let mut spu = Spu::new();
    write(&mut spu, ch(0, PNT_OFFSET), Width::Half, 0xFFFF, 0);
    write(&mut spu, ch(0, LEN_OFFSET), Width::Word, LEN_MASK, 0);
    write(
        &mut spu,
        ch(0, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, ADPCM),
        1 << 61,
    );
    assert!(spu.channel(0).end() > 1 << 61);
}

#[test]
fn given_a_new_device_when_checked_then_it_is_well_formed() {
    assert!(Spu::new().well_formed(0));
}

#[test]
fn given_registers_written_with_all_ones_when_checked_then_the_device_is_well_formed() {
    let mut spu = Spu::new();
    let words: Vec<u32> = (0..BLOCK_BYTES).step_by(4).collect();
    write_each(&mut spu, &words, Width::Word, u32::MAX);
    assert!(spu.well_formed(0));
}

#[test]
fn given_a_state_with_an_unused_bit_set_when_checked_then_it_is_not_well_formed() {
    let mut bad_channel = Spu::new();
    bad_channel.channels[4].cnt = 1 << 7;
    let mut bad_bias = Spu::new();
    bad_bias.bias = 1 << 10;
    let mut bad_capture = Spu::new();
    bad_capture.captures[1].cnt = 1 << 4;
    let verdicts = [bad_channel, bad_bias, bad_capture].map(|spu| spu.well_formed(0));
    assert_eq!(verdicts, [false, false, false]);
}

#[test]
fn given_a_stopped_channel_with_an_end_time_when_checked_then_it_is_not_well_formed() {
    let mut spu = Spu::new();
    spu.channels[0].end = 5;
    assert!(!spu.well_formed(0));
}

#[test]
fn given_a_one_shot_past_its_end_when_checked_then_it_is_still_well_formed() {
    let spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    assert!(spu.well_formed(1 << 40));
}
