// SPDX-License-Identifier: Apache-2.0
//! Tests of the device's pure parts: the sample formats, the channel mapping, the callback body
//! and the choice to open a device. The `cpal` adapter itself needs a device and has no test.

use super::*;
use std::cell::Cell;

#[test]
fn given_the_extremes_and_zero_when_made_into_f32_then_they_are_minus_one_zero_and_just_under_one()
{
    assert_eq!(
        (
            f32::from_i16(i16::MIN),
            f32::from_i16(0),
            f32::from_i16(i16::MAX)
        ),
        (-1.0, 0.0, 32_767.0 / 32_768.0)
    );
}

#[test]
fn given_the_extremes_and_zero_when_made_into_u16_then_zero_is_the_center() {
    assert_eq!(
        (
            u16::from_i16(i16::MIN),
            u16::from_i16(-1),
            u16::from_i16(0),
            u16::from_i16(i16::MAX)
        ),
        (0, 32_767, 32_768, 65_535)
    );
}

#[test]
fn given_a_sample_when_made_into_i16_then_it_is_unchanged() {
    assert_eq!(
        (i16::from_i16(-12_345), i16::from_i16(i16::MAX)),
        (-12_345, i16::MAX)
    );
}

/// What channels 0..`channels` play for the pair.
fn channel_row(pair: [i16; 2], channels: usize) -> Vec<i16> {
    (0..channels)
        .map(|channel| channel_sample(pair, channel, channels))
        .collect()
}

#[test]
fn given_one_channel_when_mapped_then_it_plays_the_mean_of_left_and_right() {
    assert_eq!(
        (
            channel_row([1_000, 3_000], 1),
            channel_row([i16::MAX, i16::MAX], 1),
            channel_row([i16::MIN, i16::MIN], 1)
        ),
        (vec![2_000], vec![i16::MAX], vec![i16::MIN])
    );
}

#[test]
fn given_two_channels_when_mapped_then_they_play_left_and_right() {
    assert_eq!(channel_row([7, -9], 2), vec![7, -9]);
}

#[test]
fn given_six_channels_when_mapped_then_left_and_right_lead_and_the_rest_are_silent() {
    assert_eq!(channel_row([7, -9], 6), vec![7, -9, 0, 0, 0, 0]);
}

/// `pair_count` pairs `(k, -k)` for k from 1.
fn ring_with(pair_count: i16) -> AudioRing {
    let ring = AudioRing::new(4_096);
    let samples: Vec<i16> = (1..=pair_count).flat_map(|k| [k, -k]).collect();
    ring.push(&samples);
    ring
}

#[test]
fn given_a_stereo_f32_callback_when_rendered_then_the_pairs_come_out_in_order_as_floats() {
    let ring = ring_with(2);
    let mut out = [9.0_f32; 4];
    render(&ring, 2, &mut out);
    assert_eq!(
        out,
        [
            1.0 / 32_768.0,
            -1.0 / 32_768.0,
            2.0 / 32_768.0,
            -2.0 / 32_768.0
        ]
    );
}

#[test]
fn given_a_six_channel_callback_when_rendered_then_each_frame_has_left_right_and_four_zeros() {
    let ring = ring_with(2);
    let mut out = [5_i16; 12];
    render(&ring, 6, &mut out);
    assert_eq!(out, [1, -1, 0, 0, 0, 0, 2, -2, 0, 0, 0, 0]);
}

#[test]
fn given_a_mono_callback_when_rendered_then_each_frame_is_the_mean() {
    let ring = AudioRing::new(8);
    ring.push(&[10, 30, -10, -30]);
    let mut out = [5_i16; 2];
    render(&ring, 1, &mut out);
    assert_eq!(out, [20, -20]);
}

#[test]
fn given_a_u16_callback_when_rendered_then_the_samples_center_on_32768() {
    let ring = ring_with(1);
    let mut out = [0_u16; 2];
    render(&ring, 2, &mut out);
    assert_eq!(out, [32_769, 32_767]);
}

#[test]
fn given_a_callback_larger_than_the_chunk_when_rendered_then_every_pair_is_in_order() {
    let ring = ring_with(1_300);
    let mut out = vec![0_i16; 1_300 * 2];
    render(&ring, 2, &mut out);
    let want: Vec<i16> = (1..=1_300).flat_map(|k| [k, -k]).collect();
    assert_eq!((out, ring.fill(), ring.underruns()), (want, 0, 0));
}

#[test]
fn given_a_ring_with_less_than_asked_when_rendered_then_the_rest_is_silence_and_counted() {
    let ring = ring_with(1);
    let mut out = [5_i16; 8];
    render(&ring, 2, &mut out);
    assert_eq!((out, ring.underruns()), ([1, -1, 0, 0, 0, 0, 0, 0], 3));
}

#[test]
fn given_a_frame_cut_short_when_rendered_then_the_partial_frame_still_gets_its_pair() {
    let ring = ring_with(2);
    let mut out = [5_i16; 5];
    render(&ring, 2, &mut out);
    assert_eq!(out, [1, -1, 2, -2, 0]);
}

#[test]
fn given_zero_channels_when_rendered_then_it_is_treated_as_one() {
    let ring = AudioRing::new(8);
    ring.push(&[10, 30]);
    let mut out = [0_i16; 1];
    render(&ring, 0, &mut out);
    assert_eq!(out, [20]);
}

#[test]
fn given_sound_wanted_with_a_rom_and_a_device_when_chosen_then_the_device_is_used() {
    assert_eq!(use_audio(true, true, || Ok::<_, ()>(7)), Some(7));
}

#[test]
fn given_an_open_that_fails_when_chosen_then_there_is_no_audio() {
    assert_eq!(use_audio(true, true, || Err::<u8, _>("no device")), None);
}

#[test]
fn given_no_audio_or_no_rom_when_chosen_then_no_device_is_even_tried() {
    let tries = Cell::new(0);
    let open = || {
        tries.set(tries.get() + 1);
        Ok::<_, ()>(1)
    };
    let off = use_audio(false, true, open);
    let silent_pattern = use_audio(true, false, open);
    assert_eq!((off, silent_pattern, tries.get()), (None, None, 0));
}

#[test]
fn given_the_no_device_error_when_shown_then_it_says_so() {
    assert_eq!(OpenError::NoDevice.to_string(), "no sound output device");
}

#[test]
fn given_an_unsupported_format_when_shown_then_it_names_the_format() {
    assert_eq!(
        OpenError::Unsupported(SampleFormat::I24).to_string(),
        "unsupported sample format I24"
    );
}
