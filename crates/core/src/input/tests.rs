// SPDX-License-Identifier: Apache-2.0
//! The host's input types, from `docs/hw/keypad.md`.

use super::*;

#[test]
fn given_the_default_input_when_inspected_then_nothing_is_held_and_the_hinge_is_open() {
    assert_eq!(
        Input::default(),
        Input {
            buttons: Buttons::empty(),
            touch: None,
            hinge_closed: false,
        }
    );
}

#[test]
fn given_buttons_a_and_y_when_or_ed_then_the_bits_are_the_keyinput_and_extkeyin_positions() {
    assert_eq!((Buttons::A | Buttons::Y).bits(), 0x0801);
}

#[test]
fn given_bits_above_y_when_truncated_then_only_the_twelve_buttons_remain() {
    assert_eq!(Buttons::from_bits_truncate(0xFFFF).bits(), 0x0FFF);
}

#[test]
fn given_a_set_when_asked_for_a_member_and_a_non_member_then_contains_tells_them_apart() {
    let held = Buttons::START | Buttons::L;
    assert_eq!(
        (
            held.contains(Buttons::START),
            held.contains(Buttons::L),
            held.contains(Buttons::A),
            held.contains(Buttons::START | Buttons::A)
        ),
        (true, true, false, false)
    );
}
