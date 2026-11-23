// SPDX-License-Identifier: Apache-2.0
//! Tests of `gamepad`: the button table, the stick's deadzone and diagonals, disconnects, and the
//! events that carry no button. A table goes through a harness that returns the rows that differ.
//! `gilrs` event codes cannot be built outside the library, so the button and axis arms of
//! `translate` and the adapter are the ones only a pad exercises.

use super::*;

/// The rows `(name, button, want)` whose press does not hold exactly the buttons wanted.
fn button_failures(rows: &[(&'static str, Button, Buttons)]) -> Vec<(&'static str, u16)> {
    rows.iter()
        .map(|(name, button, want)| {
            let mut pad = Pad::new();
            pad.handle(PadEvent::Button(*button, true));
            (*name, pad.buttons(), *want)
        })
        .filter(|(_, got, want)| got != want)
        .map(|(name, got, _)| (name, got.bits()))
        .collect()
}

#[test]
fn given_each_mapped_pad_button_when_pressed_then_it_holds_its_console_button_by_position() {
    let rows = [
        ("east is a", Button::East, Buttons::A),
        ("south is b", Button::South, Buttons::B),
        ("north is x", Button::North, Buttons::X),
        ("west is y", Button::West, Buttons::Y),
        ("left shoulder is l", Button::LeftTrigger, Buttons::L),
        ("right shoulder is r", Button::RightTrigger, Buttons::R),
        ("start", Button::Start, Buttons::START),
        ("select", Button::Select, Buttons::SELECT),
        ("dpad up", Button::DPadUp, Buttons::UP),
        ("dpad down", Button::DPadDown, Buttons::DOWN),
        ("dpad left", Button::DPadLeft, Buttons::LEFT),
        ("dpad right", Button::DPadRight, Buttons::RIGHT),
    ];
    assert_eq!(button_failures(&rows), vec![]);
}

#[test]
fn given_pad_buttons_with_no_console_meaning_when_pressed_then_nothing_is_held() {
    let rows = [
        ("mode", Button::Mode, Buttons::empty()),
        ("c", Button::C, Buttons::empty()),
        ("z", Button::Z, Buttons::empty()),
        ("left trigger 2", Button::LeftTrigger2, Buttons::empty()),
        ("right trigger 2", Button::RightTrigger2, Buttons::empty()),
        ("left thumb", Button::LeftThumb, Buttons::empty()),
        ("unknown", Button::Unknown, Buttons::empty()),
    ];
    assert_eq!(button_failures(&rows), vec![]);
}

/// The buttons held with the left stick at (`x`, `y`).
fn stick(x: f32, y: f32) -> Buttons {
    let mut pad = Pad::new();
    pad.handle(PadEvent::Axis(Axis::LeftStickX, x));
    pad.handle(PadEvent::Axis(Axis::LeftStickY, y));
    pad.buttons()
}

/// The rows `(name, x, y, want)` whose stick does not hold exactly the buttons wanted.
fn stick_failures(rows: &[(&'static str, f32, f32, Buttons)]) -> Vec<(&'static str, u16)> {
    rows.iter()
        .filter(|(_, x, y, want)| stick(*x, *y) != *want)
        .map(|(name, x, y, _)| (*name, stick(*x, *y).bits()))
        .collect()
}

#[test]
fn given_the_left_stick_when_leaned_past_the_deadzone_then_the_dpad_direction_is_held() {
    let rows = [
        ("right", 1.0, 0.0, Buttons::RIGHT),
        ("left", -1.0, 0.0, Buttons::LEFT),
        ("up", 0.0, 1.0, Buttons::UP),
        ("down", 0.0, -1.0, Buttons::DOWN),
    ];
    assert_eq!(stick_failures(&rows), vec![]);
}

#[test]
fn given_the_left_stick_at_the_deadzone_edge_when_read_then_only_past_it_presses() {
    let past = DEADZONE + 0.01;
    let rows = [
        ("at the edge, right", DEADZONE, 0.0, Buttons::empty()),
        ("at the edge, left", -DEADZONE, 0.0, Buttons::empty()),
        ("at the edge, up", 0.0, DEADZONE, Buttons::empty()),
        ("at the edge, down", 0.0, -DEADZONE, Buttons::empty()),
        ("just past, right", past, 0.0, Buttons::RIGHT),
        ("just past, left", -past, 0.0, Buttons::LEFT),
        ("just past, up", 0.0, past, Buttons::UP),
        ("just past, down", 0.0, -past, Buttons::DOWN),
        ("at rest", 0.0, 0.0, Buttons::empty()),
        ("a small drift", 0.2, -0.2, Buttons::empty()),
    ];
    assert_eq!(stick_failures(&rows), vec![]);
}

#[test]
fn given_the_left_stick_leaned_diagonally_when_read_then_two_directions_are_held() {
    let rows = [
        ("up right", 0.9, 0.9, Buttons::UP | Buttons::RIGHT),
        ("down left", -0.9, -0.9, Buttons::DOWN | Buttons::LEFT),
        ("up left", -0.9, 0.9, Buttons::UP | Buttons::LEFT),
        ("down right", 0.9, -0.9, Buttons::DOWN | Buttons::RIGHT),
    ];
    assert_eq!(stick_failures(&rows), vec![]);
}

/// The buttons held after every axis of `axes` moved fully.
fn after_axes(axes: &[Axis]) -> Buttons {
    let mut pad = Pad::new();
    axes.iter()
        .for_each(|axis| pad.handle(PadEvent::Axis(*axis, 1.0)));
    pad.buttons()
}

#[test]
fn given_other_axes_when_they_move_then_nothing_is_held() {
    let axes = [
        Axis::RightStickX,
        Axis::RightStickY,
        Axis::LeftZ,
        Axis::DPadX,
        Axis::Unknown,
    ];
    assert_eq!(after_axes(&axes), Buttons::empty());
}

#[test]
fn given_a_dpad_button_and_the_stick_when_both_lean_the_same_way_then_the_direction_is_held_once() {
    let mut pad = Pad::new();
    pad.handle(PadEvent::Button(Button::DPadRight, true));
    pad.handle(PadEvent::Axis(Axis::LeftStickX, 1.0));
    assert_eq!(pad.buttons(), Buttons::RIGHT);
}

#[test]
fn given_a_dpad_button_and_the_stick_when_the_stick_returns_then_the_button_still_holds() {
    let mut pad = Pad::new();
    pad.handle(PadEvent::Button(Button::DPadRight, true));
    pad.handle(PadEvent::Axis(Axis::LeftStickX, 1.0));
    pad.handle(PadEvent::Axis(Axis::LeftStickX, 0.0));
    assert_eq!(pad.buttons(), Buttons::RIGHT);
}

#[test]
fn given_two_pad_buttons_when_one_is_released_then_the_other_stays_held() {
    let mut pad = Pad::new();
    pad.handle(PadEvent::Button(Button::East, true));
    pad.handle(PadEvent::Button(Button::South, true));
    pad.handle(PadEvent::Button(Button::East, false));
    assert_eq!(pad.buttons(), Buttons::B);
}

#[test]
fn given_buttons_and_the_stick_when_the_pad_disconnects_then_everything_lets_go() {
    let mut pad = Pad::new();
    pad.handle(PadEvent::Button(Button::East, true));
    pad.handle(PadEvent::Axis(Axis::LeftStickX, 1.0));
    pad.handle(PadEvent::Disconnected);
    assert_eq!(pad.buttons(), Buttons::empty());
}

#[test]
fn given_buttons_when_focus_is_lost_then_everything_lets_go() {
    let mut pad = Pad::new();
    pad.handle(PadEvent::Button(Button::East, true));
    pad.release_all();
    assert_eq!(pad.buttons(), Buttons::empty());
}

#[test]
fn given_events_without_a_console_meaning_when_translated_then_they_are_ignored() {
    assert_eq!(
        (
            translate(&EventType::Dropped),
            translate(&EventType::ForceFeedbackEffectCompleted)
        ),
        (None, None)
    );
}

#[test]
fn given_a_disconnect_when_translated_then_it_is_passed_on_and_a_connect_is_ignored() {
    assert_eq!(
        (
            translate(&EventType::Connected),
            translate(&EventType::Disconnected)
        ),
        (None, Some(PadEvent::Disconnected))
    );
}

#[test]
fn given_a_new_pad_state_when_asked_then_nothing_is_held() {
    assert_eq!(Pad::new().buttons(), Buttons::empty());
}
