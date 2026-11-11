// SPDX-License-Identifier: Apache-2.0
//! The keypad stub, from `docs/hw/keypad.md`.

use super::*;

fn peek9(pad: &Keypad<true>, offset: u32, width: Width) -> u32 {
    Device::<true>::peek(pad, offset, width, 0)
}

fn peek7(pad: &Keypad<false>, offset: u32, width: Width) -> u32 {
    Device::<false>::peek(pad, offset, width, 0)
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
    Device::<true>::write(&mut pad, KEYCNT_OFFSET, Width::Half, 0xFFFF, 0);
    assert_eq!(peek9(&pad, KEYCNT_OFFSET, Width::Half), 0xC3FF);
}

#[test]
fn given_byte_writes_to_each_half_of_keycnt_when_read_back_then_each_half_is_masked_alone() {
    let mut pad = Keypad::<false>::new();
    Device::<false>::write(&mut pad, KEYCNT_OFFSET, Width::Byte, 0xAB, 0);
    Device::<false>::write(&mut pad, 3, Width::Byte, 0xFF, 0);
    assert_eq!(peek7(&pad, KEYCNT_OFFSET, Width::Half), 0xC3AB);
}

#[test]
fn given_a_word_write_at_keyinput_when_read_back_then_keyinput_is_unchanged_and_keycnt_took_the_high_half()
 {
    let mut pad = Keypad::<true>::new();
    Device::<true>::write(&mut pad, 0, Width::Word, 0x0001_0000, 0);
    assert_eq!(peek9(&pad, 0, Width::Word), 0x0001_03FF);
}

#[test]
fn given_writes_to_extkeyin_when_read_back_then_it_is_unchanged() {
    let mut pad = Keypad::<false>::new();
    Device::<false>::write(&mut pad, EXTKEYIN_OFFSET, Width::Half, 0, 0);
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
    Device::<true>::write(&mut nine, KEYCNT_OFFSET, Width::Half, 0x4001, 0);
    assert_eq!((nine.keycnt(), seven.keycnt()), (0x4001, 0));
}
