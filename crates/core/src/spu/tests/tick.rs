// SPDX-License-Identifier: Apache-2.0
//! `Spu::tick`: one output sample is the pipeline the machine used to assemble by hand (catch the
//! generators up, read the samples, mix, apply the gate, work out the capture writes), in one call
//! (`docs/hw/spu.md`, "Output samples").

use super::capture::running;
use super::*;

/// The generators caught up to `time`, and the spu holding them.
fn caught_up(spu: &Spu, time: u64, read: impl Fn(u32) -> u8 + Copy) -> ([Decoders; CHANNELS], Spu) {
    let decoders: [Decoders; CHANNELS] =
        std::array::from_fn(|x| spu.channel(x).advanced(time, read));
    let mut stored = spu.clone();
    stored.set_decoders(decoders);
    (decoders, stored)
}

/// The pipeline written out step by step, as the machine did it before `tick` existed.
fn by_hand(
    spu: &Spu,
    time: u64,
    audible: bool,
    read: impl Fn(u32) -> u8 + Copy,
) -> ([Decoders; CHANNELS], [i16; 2]) {
    let (decoders, stored) = caught_up(spu, time, read);
    let samples = std::array::from_fn(|x| stored.channel(x).sample(time, read));
    let mix = stored.mix(&samples, stored.adding(time));
    (decoders, stored.output_mixed(&mix, audible))
}

/// A device with channel 0 playing PCM8 from address 0.
fn playing() -> Spu {
    master_on(started(0, start(ONE_SHOT, PCM8) | 0x7F, 0, 4, STARTED_AT))
}

/// `spu` with the master enable and the master volume at their top, so a sample is heard.
fn master_on(mut spu: Spu) -> Spu {
    write(&mut spu, SOUNDCNT_OFFSET, Width::Half, 0x807F, 0);
    spu
}

#[test]
fn given_a_playing_channel_when_a_tick_runs_audible_then_the_output_is_the_hand_made_pipeline() {
    let spu = playing();
    let time = at_sample(2);
    assert_eq!(
        spu.tick(time, true, memory).output,
        by_hand(&spu, time, true, memory).1
    );
}

#[test]
fn given_a_playing_channel_when_a_tick_runs_gated_then_the_output_is_the_gated_hand_made_one() {
    let spu = playing();
    let time = at_sample(2);
    assert_eq!(
        spu.tick(time, false, memory).output,
        by_hand(&spu, time, false, memory).1
    );
}

#[test]
fn given_an_adpcm_channel_when_a_tick_runs_then_the_decoders_are_the_ones_caught_up_to_the_time() {
    let spu = master_on(started(0, start(ONE_SHOT, 2) | 0x7F, 0, 4, STARTED_AT));
    let time = at_sample(3);
    assert_eq!(
        spu.tick(time, true, memory).decoders,
        by_hand(&spu, time, true, memory).0
    );
}

#[test]
fn given_a_running_capture_when_a_tick_runs_then_its_writes_are_the_ones_the_unit_works_out() {
    let mut spu = playing();
    running(&mut spu, 0, 0);
    let time = 1_000_000;
    let (_, stored) = caught_up(&spu, time, memory);
    let samples = std::array::from_fn(|x| stored.channel(x).sample(time, memory));
    let mix = stored.mix(&samples, stored.adding(time));
    let captures = spu.tick(time, true, memory).captures;
    assert_eq!(
        (captures[0].is_some(), captures),
        (true, stored.capture_writes(&mix, &samples, time))
    );
}

#[test]
fn given_an_adpcm_channel_late_in_its_run_when_a_tick_runs_then_the_output_uses_the_caught_up_decoder()
 {
    // A stale decoder would still read 0 here; the caught-up one gives the table's 563.
    let spu = master_on(started(0, start(ONE_SHOT, ADPCM) | 0x7F, 0, 4, STARTED_AT));
    let time = at_sample(23);
    let output = spu.tick(time, true, adpcm_memory).output;
    assert_eq!(
        (output != [0, 0], output),
        (true, by_hand(&spu, time, true, adpcm_memory).1)
    );
}
