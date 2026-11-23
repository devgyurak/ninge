// SPDX-License-Identifier: Apache-2.0
//! The keypad stub, from `docs/hw/keypad.md`.

use super::*;
use crate::input::{Buttons, Touch};
use crate::io::device::{Lines, cx_at};
use crate::io::irq::Requests;

/// A read with no key held: the context carries the default input.
fn peek9(pad: &Keypad<true>, offset: u32, width: Width) -> u32 {
    Device::<true>::peek(pad, offset, width, cx_at(0))
}

fn peek7(pad: &Keypad<false>, offset: u32, width: Width) -> u32 {
    Device::<false>::peek(pad, offset, width, cx_at(0))
}

/// A read where the host's `input` is what the context carries, as `Io` hands it over.
fn peek9_with(pad: &Keypad<true>, input: &Input, offset: u32, width: Width) -> u32 {
    Device::<true>::peek(
        pad,
        offset,
        width,
        Cx {
            now: 0,
            host: input,
            rom: &[],
            lines: Lines {
                pen_irq_enabled: true,
                ..Lines::default()
            },
        },
    )
}

fn peek7_with(pad: &Keypad<false>, input: &Input, offset: u32, width: Width) -> u32 {
    Device::<false>::peek(
        pad,
        offset,
        width,
        Cx {
            now: 0,
            host: input,
            rom: &[],
            lines: Lines {
                pen_irq_enabled: true,
                ..Lines::default()
            },
        },
    )
}

#[test]
fn given_a_new_keypad_when_keyinput_is_read_then_all_ten_key_bits_are_set_on_both_cpus() {
    let (nine, seven) = (Keypad::<true>::new(), Keypad::<false>::new());
    assert_eq!(
        (peek9(&nine, 0, Width::Half), peek7(&seven, 0, Width::Half)),
        (0x03FF, 0x03FF)
    );
}

#[test]
fn given_a_new_arm7_keypad_when_extkeyin_is_read_then_nothing_is_pressed_and_the_hinge_is_open() {
    let pad = Keypad::<false>::new();
    assert_eq!(peek7(&pad, EXTKEYIN_OFFSET, Width::Half), 0x007F);
}

#[test]
fn given_an_arm9_keypad_when_the_extkeyin_offset_is_read_then_it_reads_zero() {
    let pad = Keypad::<true>::new();
    assert_eq!(peek9(&pad, EXTKEYIN_OFFSET, Width::Half), 0);
}

#[test]
fn given_a_halfword_write_of_ffff_to_keycnt_when_read_back_then_only_the_defined_bits_stay() {
    let mut pad = Keypad::<true>::new();
    Device::<true>::write(&mut pad, KEYCNT_OFFSET, Width::Half, 0xFFFF, cx_at(0));
    assert_eq!(peek9(&pad, KEYCNT_OFFSET, Width::Half), 0xC3FF);
}

#[test]
fn given_byte_writes_to_each_half_of_keycnt_when_read_back_then_each_half_is_masked_alone() {
    let mut pad = Keypad::<false>::new();
    Device::<false>::write(&mut pad, KEYCNT_OFFSET, Width::Byte, 0xAB, cx_at(0));
    Device::<false>::write(&mut pad, 3, Width::Byte, 0xFF, cx_at(0));
    assert_eq!(peek7(&pad, KEYCNT_OFFSET, Width::Half), 0xC3AB);
}

#[test]
fn given_a_word_write_at_keyinput_when_read_back_then_keyinput_is_unchanged_and_keycnt_took_the_high_half()
 {
    let mut pad = Keypad::<true>::new();
    Device::<true>::write(&mut pad, 0, Width::Word, 0x0001_0000, cx_at(0));
    assert_eq!(peek9(&pad, 0, Width::Word), 0x0001_03FF);
}

#[test]
fn given_writes_to_extkeyin_when_read_back_then_it_is_unchanged() {
    let mut pad = Keypad::<false>::new();
    Device::<false>::write(&mut pad, EXTKEYIN_OFFSET, Width::Half, 0, cx_at(0));
    assert_eq!(peek7(&pad, EXTKEYIN_OFFSET, Width::Half), 0x007F);
}

#[test]
fn given_a_word_read_at_rcnt_on_the_arm7_when_read_then_rcnt_is_zero_and_extkeyin_is_above_it() {
    let pad = Keypad::<false>::new();
    assert_eq!(peek7(&pad, 4, Width::Word), 0x007F_0000);
}

#[test]
fn given_a_keycnt_write_on_the_arm9_copy_when_the_arm7_copy_is_read_then_it_is_still_clear() {
    let mut nine = Keypad::<true>::new();
    let seven = Keypad::<false>::new();
    Device::<true>::write(&mut nine, KEYCNT_OFFSET, Width::Half, 0x4001, cx_at(0));
    assert_eq!((nine.keycnt(), seven.keycnt()), (0x4001, 0));
}

#[test]
fn given_held_keys_in_the_context_when_keyinput_is_read_then_each_reads_zero_on_both_cpus() {
    let input = held(Buttons::RIGHT | Buttons::L);
    let (nine, seven) = (Keypad::<true>::new(), Keypad::<false>::new());
    // Right is bit 4 and L is bit 9: 0x03FF without them.
    assert_eq!(
        (
            peek9_with(&nine, &input, 0, Width::Half),
            peek7_with(&seven, &input, 0, Width::Half)
        ),
        (0x01EF, 0x01EF)
    );
}

#[test]
fn given_x_and_y_in_the_context_when_the_arm7_reads_extkeyin_then_bits_0_and_1_are_clear() {
    let pad = Keypad::<false>::new();
    let input = held(Buttons::X | Buttons::Y);
    assert_eq!(
        peek7_with(&pad, &input, EXTKEYIN_OFFSET, Width::Half),
        0x007C
    );
}

#[test]
fn given_a_closed_hinge_in_the_context_when_the_arm7_reads_extkeyin_then_bit_7_is_set() {
    let pad = Keypad::<false>::new();
    let input = Input::new().with_hinge_closed(true);
    assert_eq!(
        peek7_with(&pad, &input, EXTKEYIN_OFFSET, Width::Half),
        0x00FF
    );
}

#[test]
fn given_x_in_the_context_when_the_arm9_reads_the_extkeyin_offset_then_it_still_reads_zero() {
    let pad = Keypad::<true>::new();
    assert_eq!(
        peek9_with(&pad, &held(Buttons::X), EXTKEYIN_OFFSET, Width::Half),
        0
    );
}

#[test]
fn given_a_keycnt_when_a_key_is_held_in_the_context_then_keycnt_is_unchanged() {
    let mut pad = Keypad::<true>::new();
    Device::<true>::write(&mut pad, KEYCNT_OFFSET, Width::Half, 0xC003, cx_at(0));
    assert_eq!(
        peek9_with(&pad, &held(Buttons::A), KEYCNT_OFFSET, Width::Half),
        0xC003
    );
}

fn held(buttons: Buttons) -> Input {
    Input::new().with_buttons(buttons)
}

/// EXTKEYIN as the ARM7 reads it with the panel at `touch` and the touch controller's pen output
/// enabled or not, as `Io` hands both over.
fn extkeyin_with(touch: Option<Touch>, pen_irq_enabled: bool) -> u32 {
    let input = Input::new().with_touch(touch);
    Device::<false>::peek(
        &Keypad::<false>::new(),
        6,
        Width::Half,
        Cx {
            now: 0,
            host: &input,
            rom: &[],
            lines: Lines {
                pen_irq_enabled,
                ..Lines::default()
            },
        },
    )
}

#[test]
fn given_a_touched_panel_and_the_pen_output_enabled_when_extkeyin_is_read_then_the_pen_bit_is_zero()
{
    assert_eq!(extkeyin_with(Some(Touch { x: 1, y: 2 }), true), 0x003F);
}

#[test]
fn given_a_touched_panel_and_the_pen_output_disabled_when_extkeyin_is_read_then_the_pen_bit_is_one()
{
    assert_eq!(extkeyin_with(Some(Touch { x: 1, y: 2 }), false), 0x007F);
}

#[test]
fn given_an_untouched_panel_and_the_pen_output_enabled_when_extkeyin_is_read_then_the_pen_bit_is_one()
 {
    assert_eq!(extkeyin_with(None, true), 0x007F);
}

#[test]
fn given_a_touched_panel_with_x_held_and_the_hinge_closed_when_extkeyin_is_read_then_each_bit_is_its_own()
 {
    // X held (bit 0 clear), pen down (bit 6 clear), hinge closed (bit 7 set).
    let input = Input::new()
        .with_touch(Some(Touch { x: 1, y: 2 }))
        .with_buttons(Buttons::X)
        .with_hinge_closed(true);
    let value = Device::<false>::peek(
        &Keypad::<false>::new(),
        6,
        Width::Half,
        Cx {
            now: 0,
            host: &input,
            rom: &[],
            lines: Lines {
                pen_irq_enabled: true,
                ..Lines::default()
            },
        },
    );
    assert_eq!(value, 0x00BE);
}

#[test]
fn given_a_touched_panel_when_the_arm9_reads_its_keypad_then_no_pen_bit_reaches_it() {
    let input = Input::new().with_touch(Some(Touch { x: 1, y: 2 }));
    assert_eq!(
        peek9_with(&Keypad::<true>::new(), &input, 6, Width::Half),
        0
    );
}

// ------------------------------------------------------------------------------- interrupts

/// KEYCNT with the IRQ enable (bit 14) set and `keys` selected, in OR mode.
const fn or_of(keys: u16) -> u16 {
    0x4000 | keys
}

/// KEYCNT with the IRQ enable and the AND condition (bit 15) set and `keys` selected.
const fn and_of(keys: u16) -> u16 {
    0xC000 | keys
}

const A_B: u16 = 0b11;

/// An ARM9 keypad whose KEYCNT was written with `keycnt`.
fn pad9(keycnt: u16) -> Keypad<true> {
    let mut pad = Keypad::<true>::new();
    Device::<true>::write(
        &mut pad,
        KEYCNT_OFFSET,
        Width::Half,
        u32::from(keycnt),
        cx_at(0),
    );
    pad
}

fn pad7(keycnt: u16) -> Keypad<false> {
    let mut pad = Keypad::<false>::new();
    Device::<false>::write(
        &mut pad,
        KEYCNT_OFFSET,
        Width::Half,
        u32::from(keycnt),
        cx_at(0),
    );
    pad
}

/// The cases whose condition differs from the expectation: (KEYCNT, buttons held).
fn condition_failures(cases: &[(u16, Buttons, bool)]) -> Vec<(u16, Buttons, bool)> {
    cases
        .iter()
        .copied()
        .filter(|&(keycnt, buttons, expected)| {
            pad9(keycnt).condition_met(&held(buttons)) != expected
        })
        .collect()
}

#[test]
fn given_or_mode_when_selected_keys_are_held_then_the_condition_needs_any_one_of_them() {
    let failures = condition_failures(&[
        (or_of(A_B), Buttons::A, true),
        (or_of(A_B), Buttons::B, true),
        (or_of(A_B), Buttons::A | Buttons::B, true),
        (or_of(A_B), Buttons::START, false),
        (or_of(A_B), Buttons::empty(), false),
    ]);
    assert_eq!(failures, vec![]);
}

#[test]
fn given_and_mode_when_selected_keys_are_held_then_the_condition_needs_every_one_of_them() {
    let failures = condition_failures(&[
        (and_of(A_B), Buttons::A, false),
        (and_of(A_B), Buttons::A | Buttons::B, true),
        (and_of(A_B), Buttons::A | Buttons::B | Buttons::START, true),
        (and_of(A_B), Buttons::START, false),
    ]);
    assert_eq!(failures, vec![]);
}

#[test]
fn given_no_key_selected_when_keys_are_held_then_neither_mode_meets_the_condition() {
    let failures = condition_failures(&[
        (or_of(0), Buttons::A, false),
        (and_of(0), Buttons::A, false),
        (and_of(0), Buttons::empty(), false),
    ]);
    assert_eq!(failures, vec![]);
}

#[test]
fn given_x_and_y_held_when_every_key_bit_is_selected_then_the_condition_is_not_met() {
    let failures = condition_failures(&[
        (or_of(0x03FF), Buttons::X | Buttons::Y, false),
        (and_of(0x03FF), Buttons::X | Buttons::Y, false),
    ]);
    assert_eq!(failures, vec![]);
}

#[test]
fn given_the_irq_enable_clear_when_the_condition_holds_then_a_latch_requests_nothing() {
    let requests = pad9(A_B).latch_requests(&Input::new(), &held(Buttons::A));
    assert_eq!(requests, Requests::default());
}

#[test]
fn given_the_irq_enable_set_when_the_condition_holds_at_a_latch_then_the_arm9_requests_bit_12() {
    let requests = pad9(or_of(A_B)).latch_requests(&Input::new(), &held(Buttons::A));
    assert_eq!(
        requests,
        Requests {
            arm9: 1 << 12,
            arm7: 0
        }
    );
}

#[test]
fn given_the_irq_enable_set_when_the_condition_holds_at_a_latch_then_the_arm7_requests_bit_12() {
    let requests = pad7(or_of(A_B)).latch_requests(&Input::new(), &held(Buttons::A));
    assert_eq!(
        requests,
        Requests {
            arm9: 0,
            arm7: 1 << 12
        }
    );
}

#[test]
fn given_each_cpu_selecting_its_own_key_when_a_is_held_then_only_the_cpu_that_selected_it_requests()
{
    let input = held(Buttons::A);
    let requests = pad9(or_of(0b01))
        .latch_requests(&Input::new(), &input)
        .merge(pad7(or_of(0b10)).latch_requests(&Input::new(), &input));
    assert_eq!(
        requests,
        Requests {
            arm9: 1 << 12,
            arm7: 0
        }
    );
}

#[test]
fn given_a_key_held_across_two_latches_when_the_second_latches_then_it_requests_again() {
    let input = held(Buttons::A);
    let requests = pad9(or_of(A_B)).latch_requests(&input, &input);
    assert_eq!(
        requests,
        Requests {
            arm9: 1 << 12,
            arm7: 0
        }
    );
}

#[test]
fn given_the_hinge_closed_when_it_opens_at_a_latch_then_the_arm7_requests_bit_22_with_keycnt_clear()
{
    let requests =
        Keypad::<false>::new().latch_requests(&Input::new().with_hinge_closed(true), &Input::new());
    assert_eq!(
        requests,
        Requests {
            arm9: 0,
            arm7: 1 << 22
        }
    );
}

#[test]
fn given_the_hinge_closed_when_it_opens_at_a_latch_then_the_arm9_requests_nothing() {
    let requests =
        Keypad::<true>::new().latch_requests(&Input::new().with_hinge_closed(true), &Input::new());
    assert_eq!(requests, Requests::default());
}

#[test]
fn given_the_hinge_when_it_closes_or_stays_put_then_the_arm7_requests_nothing() {
    let (open, closed) = (Input::new(), Input::new().with_hinge_closed(true));
    let pad = Keypad::<false>::new();
    assert_eq!(
        [
            pad.latch_requests(&open, &closed),
            pad.latch_requests(&open, &open),
            pad.latch_requests(&closed, &closed),
        ],
        [Requests::default(); 3]
    );
}

/// The effect of a write of `value` at `offset` with `input` latched.
fn write_effect(pad: &mut Keypad<true>, offset: u32, value: u32, input: &Input) -> Effect {
    Device::<true>::write(
        pad,
        offset,
        Width::Half,
        value,
        Cx {
            host: input,
            ..cx_at(0)
        },
    )
}

#[test]
fn given_a_held_selected_key_when_keycnt_is_written_with_the_irq_enabled_then_bit_12_is_requested()
{
    let effect = write_effect(
        &mut Keypad::<true>::new(),
        KEYCNT_OFFSET,
        u32::from(or_of(A_B)),
        &held(Buttons::A),
    );
    assert_eq!(
        effect.requests,
        Requests {
            arm9: 1 << 12,
            arm7: 0
        }
    );
}

#[test]
fn given_no_key_held_when_keycnt_is_written_with_the_irq_enabled_then_nothing_is_requested() {
    let effect = write_effect(
        &mut Keypad::<true>::new(),
        KEYCNT_OFFSET,
        u32::from(or_of(A_B)),
        &Input::new(),
    );
    assert_eq!(effect.requests, Requests::default());
}

#[test]
fn given_a_held_selected_key_when_keyinput_is_written_then_nothing_is_requested() {
    let mut pad = pad9(or_of(A_B));
    let effect = write_effect(&mut pad, 0, 0, &held(Buttons::A));
    assert_eq!(effect.requests, Requests::default());
}

#[test]
fn given_the_hinge_opening_with_a_met_keypad_condition_when_latched_then_the_arm7_requests_both_bits()
 {
    let requests =
        pad7(or_of(A_B)).latch_requests(&Input::new().with_hinge_closed(true), &held(Buttons::A));
    assert_eq!(
        requests,
        Requests {
            arm9: 0,
            arm7: (1 << 12) | (1 << 22)
        }
    );
}
