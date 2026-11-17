// SPDX-License-Identifier: Apache-2.0
//! The keypad stub, from `docs/hw/keypad.md`.

use super::*;
use crate::input::Buttons;
use crate::io::device::cx_at;

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
