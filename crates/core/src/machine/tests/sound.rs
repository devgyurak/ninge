// SPDX-License-Identifier: Apache-2.0
//! The sound registers and POWCNT2 through `Machine`
//! (`docs/hw/sound-registers.md`): the ARM7's bus reaching the device, the ARM9 not seeing it, and
//! a savestate taken mid-playback. Expected values are worked out by hand from the note: with TMR `FFF0h` a sample is 64 master cycles, so 4 words of PCM8 (16
//! samples) last 1,024.

use super::dma::machine;
use super::*;
use crate::cpu::Bus;

const SOUND0CNT: u32 = 0x0400_0400;
const SOUND0TMR: u32 = 0x0400_0408;
const SOUND0LEN: u32 = 0x0400_040C;
const SOUNDCNT: u32 = 0x0400_0500;
const SOUNDBIAS: u32 = 0x0400_0504;
const POWCNT2: u32 = 0x0400_0304;
/// Start (bit 31), one-shot (repeat 2), PCM8.
const ONE_SHOT_PCM8: u32 = 1 << 31 | 2 << 27;

/// A machine whose channel 0 was started as a 4-word PCM8 one-shot at the ARM7's clock plus
/// 1,000; the start time.
fn playing() -> (Machine, u64) {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1_000;
    m.bus7(at).write16(SOUND0TMR, 0xFFF0);
    m.bus7(at).write32(SOUND0LEN, 4);
    m.bus7(at).write32(SOUND0CNT, ONE_SHOT_PCM8);
    (m, at)
}

fn busy(m: &mut Machine, at: u64) -> bool {
    m.bus7(at).read32(SOUND0CNT) & (1 << 31) != 0
}

#[test]
fn given_the_arm7_wrote_soundcnt_when_each_cpu_reads_it_then_only_the_arm7_sees_it() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write16(SOUNDCNT, 0xFFFF);
    let arm7 = m.bus7(at + 1).read16(SOUNDCNT);
    let arm9 = m.bus9().read16(SOUNDCNT);
    assert_eq!((arm7, arm9), (0xBF7F, 0));
}

#[test]
fn given_the_arm9_writes_to_the_sound_window_when_the_arm7_reads_then_nothing_changed() {
    // SOUNDBIAS keeps its start value, 200h (`sound-registers.md`, "Start values").
    let mut m = machine();
    m.bus9().write32(SOUNDBIAS, 0x1FF);
    let at = m.inspect().arm7_clock() + 1;
    assert_eq!(m.bus7(at).read32(SOUNDBIAS), 0x200);
}

#[test]
fn given_a_started_one_shot_when_the_arm7_polls_then_the_bit_clears_at_the_computed_end() {
    let (mut m, at) = playing();
    let polls = [at + 1_023, at + 1_024].map(|t| busy(&mut m, t));
    assert_eq!(polls, [true, false]);
}

#[test]
fn given_a_savestate_mid_playback_when_loaded_then_the_bit_clears_at_the_same_time() {
    let (m, at) = playing();
    let state = m.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    let polls = [at + 1_023, at + 1_024].map(|t| busy(&mut loaded, t));
    assert_eq!((result, polls), (Ok(()), [true, false]));
}

#[test]
fn given_a_slower_tmr_written_while_playing_when_the_arm7_polls_then_the_bit_clears_at_the_moved_end()
 {
    // `spu.md`, "Position": sample 0 ends at 64, the other 15 take 4 * 20h = 128: 64 + 15 * 128.
    let (mut m, at) = playing();
    m.bus7(at + 1).write16(SOUND0TMR, 0xFFE0);
    let polls = [at + 1_983, at + 1_984].map(|t| busy(&mut m, t));
    assert_eq!(polls, [true, false]);
}

#[test]
fn given_a_savestate_after_a_tmr_write_when_loaded_then_the_bit_clears_at_the_moved_end() {
    let (mut m, at) = playing();
    m.bus7(at + 1).write16(SOUND0TMR, 0xFFE0);
    let state = m.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    let polls = [at + 1_983, at + 1_984].map(|t| busy(&mut loaded, t));
    assert_eq!((result, polls), (Ok(()), [true, false]));
}

#[test]
fn given_a_tmr_written_while_playing_when_inspected_then_the_end_time_has_moved() {
    let (mut m, at) = playing();
    m.bus7(at + 1).write16(SOUND0TMR, 0xFFE0);
    assert_eq!(m.inspect().spu().channel(0).end(), at + 1_984);
}

#[test]
fn given_the_same_sound_writes_when_two_machines_save_then_the_states_are_identical() {
    let (a, _) = playing();
    let (b, _) = playing();
    assert_eq!(a.save_state(), b.save_state());
}

#[test]
fn given_a_started_channel_when_inspected_then_the_registers_and_end_time_are_visible() {
    let (m, at) = playing();
    let channel = m.inspect().spu().channel(0);
    assert_eq!(
        (channel.tmr(), channel.length(), channel.end()),
        (0xFFF0, 4, at + 1_024)
    );
}

#[test]
fn given_a_new_machine_when_the_arm7_reads_powcnt2_then_the_speakers_are_on_and_wifi_is_off() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    assert_eq!(m.bus7(at).read16(POWCNT2), 1);
}

#[test]
fn given_all_ones_written_to_powcnt2_when_read_back_then_only_bits_0_and_1_remain() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write16(POWCNT2, 0xFFFF);
    let read = m.bus7(at + 1).read16(POWCNT2);
    assert_eq!((read, m.inspect().power().powcnt2()), (3, 3));
}

// ------------------------------------------------------------------------- the sample clock
//
// Expected values come from `docs/hw/spu.md` ("Output samples"): one output sample per 2,048 master
// cycles, on the grid `k * 2048`. A frame is 1,120,380 cycles, so the frame that ends at line 0
// holds the samples at 2048 * 1..=547 (the next, 548 * 2048, is past it), and the second frame the
// 547 from 548 to 1094; each is a left and a right `i16`.

/// `i16` values (two a sample) one frame of audio holds, as worked out above.
const FRAME_AUDIO_LEN: usize = 2 * 547;
/// Main RAM where the test data sits, and the PCM8 bytes `40h`, which play as `4000h`.
const DATA_ADDR: u32 = 0x0200_1000;
const PCM8_QUARTER: u32 = 0x4040_4040;
/// Start, loop (repeat 1), PCM8, with SOUND0SAD at [`DATA_ADDR`].
const LOOP_PCM8: u32 = 1 << 31 | 1 << 27;
const SOUND0SAD: u32 = 0x0400_0404;

/// Master enable with the master volume 127 (read as 128), and the usual bias `200h`.
const MASTER_ON: u16 = 1 << 15 | 0x7F;
const BIAS_MEDIUM: u16 = 0x200;
/// The channel's volume 127 (read as 128) with the panning 0: all of it on the left.
const FULL_LEFT: u32 = 0x7F;

/// Master volume 127 and enable, the bias at the medium level: a zero signal is the sample 0 and a
/// channel at [`FULL_LEFT`] gives its PCM16 value (to a multiple of 64) on the left and 0 on the right
/// (`spu.md`, "The mixer": `4000h` in is `4000h` out).
fn mixer_on(m: &mut Machine, at: u64) {
    m.bus7(at).write16(SOUNDCNT, MASTER_ON);
    m.bus7(at).write16(SOUNDBIAS, BIAS_MEDIUM);
}

/// A machine whose channel 0 plays four words of `40h` PCM8 bytes in a loop, started at `at`.
fn looping(at: u64) -> Machine {
    let mut m = machine();
    mixer_on(&mut m, at);
    super::dma::poke_words(&mut m, DATA_ADDR, &[PCM8_QUARTER; 4]);
    m.bus7(at).write32(SOUND0SAD, DATA_ADDR);
    m.bus7(at).write16(SOUND0TMR, 0xFFF0);
    m.bus7(at).write32(SOUND0LEN, 4);
    m.bus7(at).write32(SOUND0CNT, LOOP_PCM8 | FULL_LEFT);
    m
}

/// `steps` debug steps of the ARM9.
fn step_arm9(m: &mut Machine, steps: u64) {
    (0..steps).for_each(|_| m.step_arm9());
}

/// The first and the last left sample of the audio and its length.
fn audio_ends(m: &Machine) -> (i16, i16, usize) {
    let audio = m.audio();
    (audio[0], audio[audio.len() - 2], audio.len())
}

#[test]
fn given_a_booted_machine_when_a_frame_runs_then_audio_holds_one_stereo_pair_per_2048_cycles() {
    let mut m = machine();
    m.run_frame(crate::Input::new());
    let first = m.audio().len();
    m.run_frame(crate::Input::new());
    assert_eq!((first, m.audio().len()), (FRAME_AUDIO_LEN, FRAME_AUDIO_LEN));
}

#[test]
fn given_the_mixer_on_and_no_channel_started_when_a_frame_runs_then_every_sample_is_zero() {
    let mut m = machine();
    mixer_on(&mut m, 1);
    m.run_frame(crate::Input::new());
    assert_eq!(m.audio().iter().filter(|&&s| s != 0).count(), 0);
}

#[test]
fn given_the_master_enable_off_when_a_frame_runs_then_every_sample_is_the_bias_alone() {
    // Bias 0 is level 0, which is -8000h; bias 200h would be 0 (`spu.md`, "The gates").
    let mut m = machine();
    m.bus7(1).write16(SOUNDBIAS, 0);
    m.run_frame(crate::Input::new());
    assert_eq!(m.audio().iter().filter(|&&s| s != -0x8000).count(), 0);
}

#[test]
fn given_a_program_that_never_sets_the_bias_when_only_the_master_is_enabled_then_silence_is_zero() {
    // `sound-registers.md`, "Start values": a program finds the bias at 200h, the medium level.
    let mut m = machine();
    m.bus7(1).write16(SOUNDCNT, MASTER_ON);
    m.run_frame(crate::Input::new());
    assert_eq!(m.audio().iter().filter(|&&s| s != 0).count(), 0);
}

#[test]
fn given_a_program_that_never_sets_the_bias_when_a_channel_plays_then_its_level_is_not_clipped() {
    // As `looping`, without the write to SOUNDBIAS: the channel's `4000h` comes out as `4000h`.
    // With a bias of 0 the same level would sit 200h lower, and anything below it would be cut.
    let mut m = machine();
    m.bus7(3_000).write16(SOUNDCNT, MASTER_ON);
    super::dma::poke_words(&mut m, DATA_ADDR, &[PCM8_QUARTER; 4]);
    m.bus7(3_000).write32(SOUND0SAD, DATA_ADDR);
    m.bus7(3_000).write16(SOUND0TMR, 0xFFF0);
    m.bus7(3_000).write32(SOUND0LEN, 4);
    m.bus7(3_000).write32(SOUND0CNT, LOOP_PCM8 | FULL_LEFT);
    m.run_frame(crate::Input::new());
    assert_eq!(audio_ends(&m), (0, 0x4000, FRAME_AUDIO_LEN));
}

#[test]
fn given_the_speakers_off_in_powcnt2_when_a_channel_plays_then_every_sample_is_zero() {
    let mut m = looping(3_000);
    m.bus7(3_000).write16(POWCNT2, 0);
    m.run_frame(crate::Input::new());
    assert_eq!(m.audio().iter().filter(|&&s| s != 0).count(), 0);
}

#[test]
fn given_a_looping_pcm8_channel_when_a_frame_runs_then_the_samples_after_the_start_carry_its_level()
{
    let mut m = looping(3_000);
    m.run_frame(crate::Input::new());
    // The pair at 2048 comes before the start at 3000; the last pair is long after it, and the
    // first sample is the ARM7-time-ahead case: its event is earlier than the write.
    assert_eq!(audio_ends(&m), (0, 0x4000, FRAME_AUDIO_LEN));
}

#[test]
fn given_a_looping_channel_when_the_frame_runs_then_the_pair_after_the_start_is_the_first_to_sound()
{
    let mut m = looping(3_000);
    m.run_frame(crate::Input::new());
    // Pairs are at 2048 (before 3000) and 4096, 6144 (after it).
    assert_eq!(&m.audio()[..6], &[0, 0, 0x4000, 0, 0x4000, 0]);
}

#[test]
fn given_a_frame_run_when_the_next_starts_then_the_audio_of_the_first_is_dropped() {
    let mut m = looping(3_000);
    m.run_frame(crate::Input::new());
    let first = m.audio()[..8].to_vec();
    m.run_frame(crate::Input::new());
    assert_eq!((first.len(), &m.audio()[..2]), (8, &[0x4000, 0][..]));
}

#[test]
fn given_a_savestate_taken_mid_playback_when_loaded_then_the_next_frame_matches_an_uninterrupted_run()
 {
    let mut straight = looping(3_000);
    straight.run_frame(crate::Input::new());
    let state = straight.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    straight.run_frame(crate::Input::new());
    loaded.run_frame(crate::Input::new());
    assert_eq!(
        (
            result,
            loaded.audio() == straight.audio(),
            loaded.save_state() == straight.save_state()
        ),
        (Ok(()), true, true)
    );
}

#[test]
fn given_a_loaded_state_when_listing_pending_then_the_next_sample_is_on_the_grid_after_now() {
    let mut m = machine();
    m.run_frame(crate::Input::new());
    let state = m.save_state();
    let mut loaded = machine();
    loaded
        .load_state(&state)
        .expect("a state we just saved loads");
    let now = loaded.inspect().scheduler().now();
    let samples: Vec<u64> = loaded
        .inspect()
        .scheduler()
        .pending()
        .iter()
        .filter(|event| event.kind == crate::scheduler::EventKind::Sample)
        .map(|event| event.time)
        .collect();
    assert_eq!(samples, vec![(now / 2_048 + 1) * 2_048]);
}

#[test]
fn given_two_machines_with_the_same_sound_when_each_runs_a_frame_then_audio_and_state_are_equal() {
    let mut a = looping(3_000);
    let mut b = looping(3_000);
    a.run_frame(crate::Input::new());
    b.run_frame(crate::Input::new());
    assert_eq!(
        (a.audio() == b.audio(), a.save_state() == b.save_state()),
        (true, true)
    );
}

#[test]
fn given_debug_steps_far_past_a_frame_when_no_frame_runs_then_the_audio_stays_within_its_bound() {
    let mut m = machine();
    step_arm9(&mut m, 3 * 1_120_380);
    assert!(m.audio().len() <= super::super::AUDIO_CAPACITY);
}

#[test]
fn given_an_adpcm_loop_at_one_sample_per_output_when_a_frame_runs_then_the_audio_follows_the_decoder()
 {
    // TMR FE00h is 4 * 512 = 2,048 master cycles a sample, so each output sample is one decode.
    // The header is value 0, index 88 (step 7FFFh); the nibbles 2 then Ah (bytes A2h) give
    // 4095 + 16383 = 20478 and then, at index 87 (step 7462h = 29794), 20478 - (3724 + 14897) =
    // 1857. At full left and master 127 the 10-bit level is the value >> 6, and the output is that
    // times 64: 20416 and 1856. The first output sample is at 2048, the start.
    let mut m = machine();
    mixer_on(&mut m, 2_048);
    super::dma::poke_words(
        &mut m,
        DATA_ADDR,
        &[0x0058_0000, 0xA2A2_A2A2, 0xA2A2_A2A2, 0xA2A2_A2A2],
    );
    m.bus7(2_048).write32(SOUND0SAD, DATA_ADDR);
    m.bus7(2_048).write16(SOUND0TMR, 0xFE00);
    m.bus7(2_048).write16(SOUND0CNT + 0xA, 1);
    m.bus7(2_048).write32(SOUND0LEN, 3);
    m.bus7(2_048)
        .write32(SOUND0CNT, 1 << 31 | 1 << 27 | 2 << 29 | FULL_LEFT);
    m.run_frame(crate::Input::new());
    assert_eq!(&m.audio()[..4], &[20_416, 0, 1_856, 0]);
}

#[test]
fn given_an_adpcm_savestate_mid_playback_when_loaded_then_the_next_frame_matches_an_uninterrupted_run()
 {
    let build = || {
        let mut m = machine();
        super::dma::poke_words(&mut m, DATA_ADDR, &[0, 0x0000_4321, 0x5555_0000, 0]);
        m.bus7(2_048).write32(SOUND0SAD, DATA_ADDR);
        m.bus7(2_048).write16(SOUND0TMR, 0xFE00);
        m.bus7(2_048).write16(SOUND0CNT + 0xA, 1);
        m.bus7(2_048).write32(SOUND0LEN, 3);
        m.bus7(2_048)
            .write32(SOUND0CNT, 1 << 31 | 1 << 27 | 2 << 29);
        m
    };
    let mut straight = build();
    straight.run_frame(crate::Input::new());
    let state = straight.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    straight.run_frame(crate::Input::new());
    loaded.run_frame(crate::Input::new());
    assert_eq!(
        (
            result,
            loaded.audio() == straight.audio(),
            loaded.save_state() == straight.save_state()
        ),
        (Ok(()), true, true)
    );
}

#[test]
fn given_a_psg_channel_at_one_sample_per_output_when_a_frame_runs_then_the_audio_is_its_wave() {
    // TMR FE00h is one wave position per output sample; duty 3 is four LOW then four HIGH. Channel
    // 8 started at 2048, so the first output sample is its first position.
    let mut m = machine();
    mixer_on(&mut m, 2_048);
    let channel_8 = SOUND0CNT + 8 * 0x10;
    m.bus7(2_048).write16(channel_8 + 8, 0xFE00);
    m.bus7(2_048)
        .write32(channel_8, 1 << 31 | 1 << 27 | 3 << 29 | 3 << 24 | FULL_LEFT);
    m.run_frame(crate::Input::new());
    // LOW is -7FFFh: >> 6 gives -512, and 64 times that is -8000h; HIGH 7FFFh gives 511 and 7FC0h.
    let (low, high) = (-0x8000, 0x7FC0);
    assert_eq!(
        &m.audio()[..16],
        &[
            low, 0, low, 0, low, 0, low, 0, high, 0, high, 0, high, 0, high, 0
        ]
    );
}

#[test]
fn given_a_noise_savestate_mid_playback_when_loaded_then_the_next_frame_matches_an_uninterrupted_run()
 {
    let build = || {
        let mut m = machine();
        let channel_14 = SOUND0CNT + 14 * 0x10;
        m.bus7(2_048).write16(channel_14 + 8, 0xFFF0);
        m.bus7(2_048)
            .write32(channel_14, 1 << 31 | 1 << 27 | 3 << 29);
        m
    };
    let mut straight = build();
    straight.run_frame(crate::Input::new());
    let state = straight.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    straight.run_frame(crate::Input::new());
    loaded.run_frame(crate::Input::new());
    assert_eq!(
        (
            result,
            loaded.audio() == straight.audio(),
            loaded.save_state() == straight.save_state()
        ),
        (Ok(()), true, true)
    );
}

#[test]
fn given_a_loaded_state_when_a_frame_runs_then_the_audio_buffer_keeps_its_reserved_capacity() {
    let mut saver = machine();
    saver.run_frame(crate::Input::new());
    let mut loaded = machine();
    let result = loaded.load_state(&saver.save_state());
    let before = loaded.audio.capacity();
    loaded.run_frame(crate::Input::new());
    assert_eq!(
        (
            result,
            before >= super::super::AUDIO_CAPACITY,
            loaded.audio.capacity() == before
        ),
        (Ok(()), true, true)
    );
}

// ------------------------------------------------------------------------------ the capture
//
// Expected values come from `docs/hw/sound-capture.md` ("Sources", "Format", "Model"). TMR `FE00h`
// makes a capture sample `4 * 200h = 2048` master cycles, one per output sample, so each output
// sample event completes exactly one slot. Channel 0 plays `4000h` at full volume, so capture 0
// from channel 0 writes `4000h` (PCM16) or `40h` (PCM8).

const SNDCAP0CNT: u32 = 0x0400_0508;
const SNDCAP0DAD: u32 = 0x0400_0510;
const SNDCAP0LEN: u32 = 0x0400_0514;
const SOUND1TMR: u32 = 0x0400_0418;
/// Where the captures land: main RAM, away from the channel's data.
const CAPTURE_ADDR: u32 = 0x0200_2000;
/// The bytes the buffer holds before the capture writes it.
const SEED: u32 = 0xAAAA_AAAA;
/// Source = channel 0, loop, PCM16 / one-shot, PCM8; with bit 7 set.
const CAP_LOOP_CHANNEL: u32 = 1 << 7 | 1 << 1;
const CAP_ONE_SHOT_PCM8_CHANNEL: u32 = 1 << 7 | 1 << 3 | 1 << 2 | 1 << 1;

/// A machine with [`looping`] on channel 0 and capture 0 started at `at` with `cnt`, `len` words at
/// `dad`, one slot per output sample, the buffer at [`CAPTURE_ADDR`] seeded.
fn capturing(at: u64, cnt: u32, len: u16, dad: u32) -> Machine {
    let mut m = looping(at);
    super::dma::poke_words(&mut m, CAPTURE_ADDR, &[SEED; 4]);
    m.bus7(at).write16(SOUND1TMR, 0xFE00);
    m.bus7(at).write32(SNDCAP0DAD, dad);
    m.bus7(at).write16(SNDCAP0LEN, len);
    m.bus7(at)
        .write8(SNDCAP0CNT, u8::try_from(cnt).expect("one byte"));
    m
}

fn halves(m: &mut Machine, addr: u32, n: u32) -> Vec<u16> {
    (0..n).map(|i| m.bus9().read16(addr + 2 * i)).collect()
}

fn bytes(m: &mut Machine, addr: u32, n: u32) -> Vec<u8> {
    (0..n).map(|i| m.bus9().read8(addr + i)).collect()
}

#[test]
fn given_a_loop_capture_of_a_playing_channel_when_a_frame_runs_then_main_ram_holds_its_samples() {
    // 2 words: 4 halfword slots, all filled by the 547 events of the frame.
    let mut m = capturing(3_000, CAP_LOOP_CHANNEL, 2, CAPTURE_ADDR);
    m.run_frame(crate::Input::new());
    assert_eq!(halves(&mut m, CAPTURE_ADDR, 4), vec![0x4000; 4]);
}

#[test]
fn given_a_one_shot_pcm8_capture_when_a_frame_runs_then_only_its_buffer_is_written_and_busy_clears()
{
    // 1 word: 4 byte slots; the fifth byte keeps the seed.
    let mut m = capturing(3_000, CAP_ONE_SHOT_PCM8_CHANNEL, 1, CAPTURE_ADDR);
    m.run_frame(crate::Input::new());
    let busy = m.bus7(1_200_000).read8(SNDCAP0CNT) & 0x80 != 0;
    assert_eq!(
        (bytes(&mut m, CAPTURE_ADDR, 5), busy),
        (vec![0x40, 0x40, 0x40, 0x40, 0xAA], false)
    );
}

#[test]
fn given_a_capture_into_arm7_wram_when_a_frame_runs_then_the_arm7_reads_the_samples_and_the_arm9_does_not()
 {
    // The ARM7-only WRAM at 0380_0000h is reachable through the ARM7's view alone.
    let mut m = capturing(3_000, CAP_LOOP_CHANNEL, 2, 0x0380_0000);
    m.run_frame(crate::Input::new());
    let arm7 = m.bus7(1_200_000).read16(0x0380_0000);
    let arm9 = m.bus9().read16(0x0380_0000);
    assert_eq!((arm7, arm9), (0x4000, 0));
}

#[test]
fn given_a_savestate_taken_mid_capture_when_loaded_then_the_next_frame_matches_an_uninterrupted_run()
 {
    let mut straight = capturing(3_000, CAP_LOOP_CHANNEL, 3, CAPTURE_ADDR);
    straight.run_frame(crate::Input::new());
    let state = straight.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    straight.run_frame(crate::Input::new());
    loaded.run_frame(crate::Input::new());
    let ram = [&mut straight, &mut loaded].map(|m| halves(m, CAPTURE_ADDR, 6));
    assert_eq!(
        (
            result,
            ram[0] == ram[1],
            loaded.audio() == straight.audio(),
            loaded.save_state() == straight.save_state()
        ),
        (Ok(()), true, true, true)
    );
}

#[test]
fn given_two_machines_with_the_same_capture_when_each_runs_a_frame_then_memory_audio_and_state_match()
 {
    let mut a = capturing(3_000, CAP_LOOP_CHANNEL, 3, CAPTURE_ADDR);
    let mut b = capturing(3_000, CAP_LOOP_CHANNEL, 3, CAPTURE_ADDR);
    a.run_frame(crate::Input::new());
    b.run_frame(crate::Input::new());
    let ram = [&mut a, &mut b].map(|m| halves(m, CAPTURE_ADDR, 6));
    assert_eq!(
        (
            ram[0] == ram[1],
            a.audio() == b.audio(),
            a.save_state() == b.save_state()
        ),
        (true, true, true)
    );
}

#[test]
fn given_a_running_capture_when_inspected_then_the_position_and_next_address_are_visible() {
    // Started at 3,000 with a period of 2,048: after 3 periods three slots are written.
    let m = capturing(3_000, CAP_LOOP_CHANNEL, 2, CAPTURE_ADDR);
    let at = 3_000 + 3 * 2_048;
    let capture = m.inspect().spu().capture(0);
    assert_eq!(
        (capture.position(at), capture.next_address(at)),
        (3, CAPTURE_ADDR + 6)
    );
}

/// A machine whose channel 0 plays PCM8 `-128` (byte `80h`, the HALTCNT halt mode), with a one-shot
/// PCM8 capture of it started at `at` whose second byte lands on HALTCNT.
fn capturing_into_haltcnt(at: u64) -> Machine {
    let mut m = looping(at);
    super::dma::poke_words(&mut m, DATA_ADDR, &[0x8080_8080; 4]);
    m.bus7(at).write16(SOUND1TMR, 0xFE00);
    m.bus7(at).write32(SNDCAP0DAD, 0x0400_0300);
    m.bus7(at).write16(SNDCAP0LEN, 1);
    m.bus7(at)
        .write8(SNDCAP0CNT, CAP_ONE_SHOT_PCM8_CHANNEL as u8);
    m
}

#[test]
fn given_a_capture_whose_byte_lands_on_haltcnt_when_the_frame_runs_then_the_arm7_is_halted() {
    let mut m = capturing_into_haltcnt(3_000);
    m.run_frame(crate::Input::new());
    assert!(m.inspect().arm7().halted());
}
