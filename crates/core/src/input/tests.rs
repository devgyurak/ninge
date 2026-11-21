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
            wall_time: 0,
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

#[test]
fn given_the_builder_methods_when_chained_then_each_sets_its_own_field() {
    let touch = Touch { x: 7, y: 9 };
    assert_eq!(
        Input::new()
            .with_buttons(Buttons::A)
            .with_touch(Some(touch))
            .with_hinge_closed(true)
            .with_wall_time(5),
        Input {
            buttons: Buttons::A,
            touch: Some(touch),
            hinge_closed: true,
            wall_time: 5,
        }
    );
}

#[test]
fn given_a_new_input_when_compared_with_the_default_then_they_are_equal() {
    assert_eq!(Input::new(), Input::default());
}

#[test]
fn given_a_wall_time_when_set_with_the_builder_then_the_field_holds_it_and_the_rest_is_unchanged() {
    assert_eq!(
        Input::new().with_wall_time(86_400),
        Input {
            wall_time: 86_400,
            ..Input::default()
        }
    );
}
