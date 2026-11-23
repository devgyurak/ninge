// SPDX-License-Identifier: Apache-2.0
//! Tests of the resampler and the rate control. Expected counts come from the rates: the machine
//! makes 67,027,964 / 2048 samples a second, so N input pairs become N * D * 2048 / 67,027,964
//! pairs at D Hz.

use super::*;
use proptest::prelude::*;

const MACHINE_HZ: f64 = 67_027_964.0 / 2048.0;

/// `count` constant pairs.
fn constant(value: i16, count: usize) -> Vec<i16> {
    vec![value; count * 2]
}

fn convert(resampler: &mut Resampler, input: &[i16]) -> Vec<i16> {
    let mut out = Vec::new();
    resampler.process(input, &mut out);
    out
}

/// Output pairs for `count` input pairs at `dest_hz`, minus the ideal count.
fn count_error(dest_hz: u32, count: usize) -> f64 {
    let out = convert(&mut Resampler::for_machine(dest_hz), &constant(7, count));
    out.len() as f64 / 2.0 - count as f64 * f64::from(dest_hz) / MACHINE_HZ
}

/// Count errors for a table of device rates.
fn count_errors_beyond_one(rates: &[u32]) -> Vec<(u32, f64)> {
    rates
        .iter()
        .map(|&rate| (rate, count_error(rate, 40_000)))
        .filter(|(_, error)| error.abs() > 1.0)
        .collect()
}

#[test]
fn given_common_device_rates_when_input_is_converted_then_the_output_count_is_within_one_pair_of_the_ratio()
 {
    assert_eq!(
        count_errors_beyond_one(&[8_000, 22_050, 44_100, 48_000, 96_000, 192_000]),
        vec![]
    );
}

/// Output pairs after `frames` frames of 547 input pairs at `dest_hz`.
fn pairs_after_frames(dest_hz: u32, frames: usize) -> usize {
    let mut resampler = Resampler::for_machine(dest_hz);
    let frame = constant(5, 547);
    let mut out = Vec::new();
    let mut produced = 0;
    for _ in 0..frames {
        out.clear();
        resampler.process(&frame, &mut out);
        produced += out.len() / 2;
    }
    produced
}

#[test]
fn given_an_hour_of_frames_when_converted_then_the_output_count_has_not_drifted() {
    // 215,374 frames of 547 pairs: an hour at 59.826 Hz.
    let ideal = 215_374.0 * 547.0 * 48_000.0 / MACHINE_HZ;
    let produced = pairs_after_frames(48_000, 215_374);
    assert!(
        (produced as f64 - ideal).abs() <= 1.0,
        "{produced} pairs against an ideal {ideal}"
    );
}

#[test]
fn given_equal_rates_when_converted_then_samples_pass_through_one_input_pair_late() {
    let mut resampler = Resampler::new(48_000, 1, 48_000);
    assert_eq!(
        convert(&mut resampler, &[10, -10, 20, -20, 30, -30]),
        vec![0, 0, 10, -10, 20, -20]
    );
}

#[test]
fn given_double_the_rate_when_converted_then_the_pairs_between_are_the_linear_midpoints() {
    let mut resampler = Resampler::new(1, 1, 2);
    assert_eq!(
        convert(&mut resampler, &[100, -100, 200, -200]),
        vec![0, 0, 50, -50, 100, -100, 150, -150]
    );
}

#[test]
fn given_a_constant_signal_when_it_crosses_call_boundaries_then_it_stays_constant() {
    let mut resampler = Resampler::for_machine(44_100);
    convert(&mut resampler, &constant(1_000, 1));
    let steady: Vec<i16> = [1, 7, 547, 2, 300]
        .iter()
        .flat_map(|&count| convert(&mut resampler, &constant(1_000, count)))
        .collect();
    let off: Vec<usize> = steady
        .iter()
        .enumerate()
        .filter(|&(_, &sample)| sample != 1_000)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(off, Vec::<usize>::new());
}

#[test]
fn given_a_stream_when_split_at_any_point_then_the_output_is_the_unsplit_output() {
    let input: Vec<i16> = (0..400).flat_map(|k: i16| [k * 40, -k * 40]).collect();
    let whole = convert(&mut Resampler::for_machine(48_000), &input);
    let mut split = Resampler::for_machine(48_000);
    let parts = [&input[..2], &input[2..300], &input[300..]];
    let joined: Vec<i16> = parts
        .iter()
        .flat_map(|part| convert(&mut split, part))
        .collect();
    assert_eq!(joined, whole);
}

#[test]
fn given_a_trailing_half_pair_when_converted_then_it_is_ignored() {
    let mut resampler = Resampler::new(1, 1, 1);
    assert_eq!(convert(&mut resampler, &[10, -10, 20]), vec![0, 0]);
}

#[test]
fn given_a_trim_beyond_the_limit_when_set_then_it_is_clamped_to_half_a_percent() {
    let mut resampler = Resampler::for_machine(48_000);
    resampler.set_trim_ppm(100_000);
    let high = resampler.trim_ppm();
    resampler.set_trim_ppm(-100_000);
    assert_eq!((high, resampler.trim_ppm()), (5_000, -5_000));
}

#[test]
fn given_a_trim_when_converted_then_a_positive_one_makes_more_and_a_negative_one_fewer_pairs() {
    let make = |trim| {
        let mut resampler = Resampler::for_machine(48_000);
        resampler.set_trim_ppm(trim);
        convert(&mut resampler, &constant(3, 20_000)).len()
    };
    let (slow, plain, fast) = (make(-5_000), make(0), make(5_000));
    // 0.5 % of about 29,000 pairs is about 146 pairs, 292 samples.
    assert_eq!(
        (slow < plain, plain < fast, (fast - slow) / 2 > 250),
        (true, true, true)
    );
}

#[test]
fn given_a_fill_above_the_target_when_the_trim_is_asked_then_it_is_negative() {
    assert!(trim_for_fill(2_400, 2_000) < 0);
}

#[test]
fn given_a_fill_below_the_target_when_the_trim_is_asked_then_it_is_positive() {
    assert!(trim_for_fill(1_600, 2_000) > 0);
}

#[test]
fn given_a_fill_at_the_target_when_the_trim_is_asked_then_it_is_zero() {
    assert_eq!(trim_for_fill(2_000, 2_000), 0);
}

#[test]
fn given_a_fill_far_from_the_target_when_the_trim_is_asked_then_it_is_clamped() {
    assert_eq!(
        (trim_for_fill(0, 2_000), trim_for_fill(1_000_000, 2_000)),
        (MAX_TRIM_PPM, -MAX_TRIM_PPM)
    );
}

#[test]
fn given_a_zero_target_when_the_trim_is_asked_then_it_is_zero() {
    assert_eq!(trim_for_fill(500, 0), 0);
}

/// Output pairs for a stream split into calls of `sizes` input pairs.
fn pairs_from_split(dest_hz: u32, sizes: &[usize]) -> usize {
    let mut resampler = Resampler::for_machine(dest_hz);
    sizes
        .iter()
        .map(|&size| convert(&mut resampler, &constant(9, size)).len() / 2)
        .sum()
}

proptest! {
    #[test]
    fn given_any_device_rate_and_split_when_converted_then_the_count_is_within_one_of_the_ratio(
        dest_hz in 8_000_u32..192_001,
        sizes in proptest::collection::vec(0_usize..700, 1..30),
    ) {
        let produced = pairs_from_split(dest_hz, &sizes);
        let ideal = sizes.iter().sum::<usize>() as f64 * f64::from(dest_hz) / MACHINE_HZ;
        prop_assert!((produced as f64 - ideal).abs() <= 1.0);
    }
}
