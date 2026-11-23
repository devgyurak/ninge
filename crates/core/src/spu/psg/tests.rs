// SPDX-License-Identifier: Apache-2.0
//! The PSG wave and the noise generator. Expected values come from `docs/hw/spu.md` ("PSG
//! (channels 8-13)", "Noise (channels 14-15)"): GBATEK's duty table, drawn `_` for LOW and `-` for
//! HIGH, and the noise formula worked by hand.

use super::*;

/// The wave for `duty` over `samples` samples as GBATEK draws it.
fn drawn(duty: u32, samples: u64) -> String {
    (0..samples)
        .map(|n| {
            if duty_level(n, duty) == HIGH {
                '-'
            } else {
                '_'
            }
        })
        .collect()
}

/// The duties whose drawn wave over 24 samples is not GBATEK's.
fn duty_failures() -> Vec<(u32, String)> {
    [
        (0, "_______-_______-_______-"),
        (1, "______--______--______--"),
        (2, "_____---_____---_____---"),
        (3, "____----____----____----"),
        (4, "___-----___-----___-----"),
        (5, "__------__------__------"),
        (6, "_-------_-------_-------"),
        (7, "________________________"),
    ]
    .iter()
    .map(|&(duty, want)| (duty, drawn(duty, 24), want))
    .filter(|(_, got, want)| got != want)
    .map(|(duty, got, _)| (duty, got))
    .collect()
}

#[test]
fn given_each_duty_when_drawn_over_three_waves_then_it_matches_gbateks_table() {
    assert_eq!(duty_failures(), Vec::new());
}

#[test]
fn given_the_levels_when_read_then_high_and_low_are_plus_and_minus_7fff() {
    assert_eq!((duty_level(7, 0), duty_level(0, 0)), (0x7FFF, -0x7FFF));
}

/// The outputs of `steps` noise steps from the start.
fn noise_run(steps: u64) -> Vec<i32> {
    let mut noise = Noise::default();
    (0..steps)
        .map(|_| {
            noise.step();
            noise.level()
        })
        .collect()
}

#[test]
fn given_a_started_noise_when_stepped_then_the_first_outputs_follow_the_formula() {
    // X = 7FFFh. Step 1: carry 1 -> LOW, X = 3FFFh ^ 6000h = 5FFFh.
    // Step 2: carry 1 -> LOW, X = 2FFFh ^ 6000h = 4FFFh.
    // Step 3: carry 1 -> LOW, X = 27FFh ^ 6000h = 47FFh.
    // Step 4: carry 1 -> LOW, X = 23FFh ^ 6000h = 43FFh.
    assert_eq!(noise_run(4), vec![LOW, LOW, LOW, LOW]);
}

#[test]
fn given_a_noise_register_with_a_zero_low_bit_when_stepped_then_the_output_is_high() {
    let mut noise = Noise {
        ticks: 1,
        x: 0x0002,
        high: false,
    };
    noise.step();
    assert_eq!((noise.level(), noise.register()), (HIGH, 0x0001));
}

#[test]
fn given_a_started_noise_when_stepped_four_times_then_the_register_follows_the_hand_values() {
    let mut noise = Noise::default();
    let registers: Vec<u16> = (0..4)
        .map(|_| {
            noise.step();
            noise.register()
        })
        .collect();
    assert_eq!(registers, vec![0x5FFF, 0x4FFF, 0x47FF, 0x43FF]);
}

#[test]
fn given_a_noise_that_has_not_stepped_when_read_then_it_outputs_nothing() {
    assert_eq!(Noise::default().level(), 0);
}

#[test]
fn given_a_noise_that_ran_when_its_register_is_15_bits_then_it_is_well_formed_and_16_bits_is_not() {
    let wide = Noise {
        x: 0x8000,
        ..Noise::default()
    };
    assert_eq!(
        (Noise::default().well_formed(), wide.well_formed()),
        (true, false)
    );
}

#[test]
fn given_a_noise_run_when_long_then_the_register_stays_15_bits() {
    let mut noise = Noise::default();
    let widest = (0..100_000)
        .map(|_| {
            noise.step();
            noise.register()
        })
        .max();
    assert_eq!(widest.map(|x| x <= 0x7FFF), Some(true));
}
