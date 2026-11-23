// SPDX-License-Identifier: Apache-2.0
//! Tests of `input`: the keymap, press, release and repeat, the fixed-frame fallback, focus loss,
//! and the lid. A table goes through a harness that returns the rows that differ.

use super::*;
use crossterm::event::KeyEventState;

fn event(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind,
        state: KeyEventState::NONE,
    }
}

fn press(code: KeyCode) -> KeyEvent {
    event(code, KeyModifiers::NONE, KeyEventKind::Press)
}

fn release(code: KeyCode) -> KeyEvent {
    event(code, KeyModifiers::NONE, KeyEventKind::Release)
}

fn repeat(code: KeyCode) -> KeyEvent {
    event(code, KeyModifiers::NONE, KeyEventKind::Repeat)
}

/// Keys on a terminal that reports release, with `events` taken in order.
fn reported(events: &[KeyEvent]) -> Keys {
    let mut keys = Keys::new(true);
    events.iter().for_each(|e| keys.handle(e));
    keys
}

/// Keys on a terminal that does not report release, with `events` taken in order.
fn legacy(events: &[KeyEvent]) -> Keys {
    let mut keys = Keys::new(false);
    events.iter().for_each(|e| keys.handle(e));
    keys
}

/// The rows `(name, buttons)` whose press does not give exactly the buttons wanted.
fn keymap_failures(rows: &[(&'static str, KeyCode, Buttons)]) -> Vec<(&'static str, u16)> {
    rows.iter()
        .filter(|(_, code, want)| reported(&[press(*code)]).buttons() != *want)
        .map(|(name, code, _)| (*name, reported(&[press(*code)]).buttons().bits()))
        .collect()
}

#[test]
fn given_each_mapped_key_when_pressed_then_it_holds_its_button() {
    let rows = [
        ("up", KeyCode::Up, Buttons::UP),
        ("down", KeyCode::Down, Buttons::DOWN),
        ("left", KeyCode::Left, Buttons::LEFT),
        ("right", KeyCode::Right, Buttons::RIGHT),
        ("x is a", KeyCode::Char('x'), Buttons::A),
        ("z is b", KeyCode::Char('z'), Buttons::B),
        ("s is x", KeyCode::Char('s'), Buttons::X),
        ("a is y", KeyCode::Char('a'), Buttons::Y),
        ("q is l", KeyCode::Char('q'), Buttons::L),
        ("w is r", KeyCode::Char('w'), Buttons::R),
        ("enter is start", KeyCode::Enter, Buttons::START),
        ("backspace is select", KeyCode::Backspace, Buttons::SELECT),
    ];
    assert_eq!(keymap_failures(&rows), vec![]);
}

#[test]
fn given_a_key_without_a_meaning_when_pressed_then_nothing_is_held() {
    let rows = [
        ("tab", KeyCode::Tab, Buttons::empty()),
        ("escape", KeyCode::Esc, Buttons::empty()),
        ("space", KeyCode::Char(' '), Buttons::empty()),
        ("e", KeyCode::Char('e'), Buttons::empty()),
        ("f1", KeyCode::F(1), Buttons::empty()),
    ];
    assert_eq!(keymap_failures(&rows), vec![]);
}

#[test]
fn given_shifted_letters_when_pressed_then_they_hold_the_same_buttons_as_lower_case() {
    let rows = [
        ("X", KeyCode::Char('X'), Buttons::A),
        ("Z", KeyCode::Char('Z'), Buttons::B),
        ("S", KeyCode::Char('S'), Buttons::X),
        ("A", KeyCode::Char('A'), Buttons::Y),
        ("Q", KeyCode::Char('Q'), Buttons::L),
        ("W", KeyCode::Char('W'), Buttons::R),
    ];
    assert_eq!(keymap_failures(&rows), vec![]);
}

#[test]
fn given_a_shift_modifier_when_a_letter_is_pressed_then_the_button_is_held() {
    let shifted = event(KeyCode::Char('x'), KeyModifiers::SHIFT, KeyEventKind::Press);
    assert_eq!(reported(&[shifted]).buttons(), Buttons::A);
}

#[test]
fn given_a_held_key_when_its_release_arrives_then_the_button_is_let_go() {
    let keys = reported(&[press(KeyCode::Char('x')), release(KeyCode::Char('x'))]);
    assert_eq!(keys.buttons(), Buttons::empty());
}

#[test]
fn given_a_release_with_shift_when_the_press_was_lower_case_then_the_button_is_let_go() {
    let shifted_release = event(
        KeyCode::Char('X'),
        KeyModifiers::SHIFT,
        KeyEventKind::Release,
    );
    let keys = reported(&[press(KeyCode::Char('x')), shifted_release]);
    assert_eq!(keys.buttons(), Buttons::empty());
}

#[test]
fn given_a_held_key_when_ctrl_is_down_at_its_release_then_the_button_is_still_let_go() {
    let ctrl_release = event(
        KeyCode::Char('x'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    );
    let keys = reported(&[press(KeyCode::Char('x')), ctrl_release]);
    assert_eq!(keys.buttons(), Buttons::empty());
}

#[test]
fn given_two_held_keys_when_one_is_released_then_the_other_stays_held() {
    let keys = reported(&[
        press(KeyCode::Char('x')),
        press(KeyCode::Up),
        release(KeyCode::Char('x')),
    ]);
    assert_eq!(keys.buttons(), Buttons::UP);
}

#[test]
fn given_two_keys_when_pressed_together_then_both_buttons_are_held() {
    let keys = reported(&[press(KeyCode::Char('x')), press(KeyCode::Right)]);
    assert_eq!(keys.buttons(), Buttons::A | Buttons::RIGHT);
}

#[test]
fn given_a_repeat_when_it_arrives_for_a_held_key_then_nothing_changes() {
    let held = reported(&[press(KeyCode::Char('x'))]);
    let repeated = reported(&[press(KeyCode::Char('x')), repeat(KeyCode::Char('x'))]);
    assert_eq!(repeated, held);
}

#[test]
fn given_a_repeat_alone_when_it_arrives_then_no_key_is_held() {
    assert_eq!(
        reported(&[repeat(KeyCode::Char('x'))]).buttons(),
        Buttons::empty()
    );
}

#[test]
fn given_a_press_with_ctrl_alt_or_super_when_taken_then_no_button_is_held() {
    let with = |m| event(KeyCode::Char('x'), m, KeyEventKind::Press);
    let held: Vec<u16> = [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ]
    .into_iter()
    .map(|m| reported(&[with(m)]).buttons().bits())
    .collect();
    assert_eq!(held, vec![0, 0, 0]);
}

#[test]
fn given_a_held_key_when_frames_pass_on_a_terminal_that_reports_release_then_it_stays_held() {
    let mut keys = reported(&[press(KeyCode::Char('x'))]);
    pass(&mut keys, 100);
    assert_eq!(keys.buttons(), Buttons::A);
}

/// `frames` frames pass: the end of each.
fn pass(keys: &mut Keys, frames: u8) {
    (0..frames).for_each(|_| keys.end_frame());
}

/// The frames, counted from the press, in which the key reads as held on a terminal that does not
/// report release: the frame number is the one the key is read in, before `end_frame`.
fn legacy_held_frames(frames: u8) -> Vec<u8> {
    let mut keys = legacy(&[press(KeyCode::Char('x'))]);
    (1..=frames)
        .filter(|_| {
            let held = keys.buttons() == Buttons::A;
            keys.end_frame();
            held
        })
        .collect()
}

#[test]
fn given_a_press_on_a_terminal_without_release_when_frames_pass_then_it_is_held_for_the_fixed_count()
 {
    let expected: Vec<u8> = (1..=FALLBACK_HOLD_FRAMES).collect();
    assert_eq!(legacy_held_frames(FALLBACK_HOLD_FRAMES + 4), expected);
}

#[test]
fn given_a_press_on_a_terminal_without_release_when_a_repeat_press_arrives_then_the_hold_restarts()
{
    let mut keys = legacy(&[press(KeyCode::Char('x'))]);
    pass(&mut keys, FALLBACK_HOLD_FRAMES - 1);
    keys.handle(&press(KeyCode::Char('x')));
    pass(&mut keys, FALLBACK_HOLD_FRAMES - 1);
    assert_eq!(keys.buttons(), Buttons::A);
}

#[test]
fn given_a_held_key_when_focus_is_lost_then_every_button_is_let_go() {
    let mut keys = reported(&[press(KeyCode::Char('x')), press(KeyCode::Up)]);
    keys.release_all();
    assert_eq!(keys.buttons(), Buttons::empty());
}

#[test]
fn given_the_microphone_key_when_pressed_and_released_then_the_microphone_follows_it() {
    let down = reported(&[press(KeyCode::Char('m'))]);
    let up = reported(&[press(KeyCode::Char('m')), release(KeyCode::Char('m'))]);
    assert_eq!(
        (down.mic(), down.buttons(), up.mic()),
        (true, Buttons::empty(), false)
    );
}

#[test]
fn given_the_microphone_key_when_focus_is_lost_then_the_player_stops_blowing() {
    let mut keys = reported(&[press(KeyCode::Char('M'))]);
    keys.release_all();
    assert!(!keys.mic());
}

#[test]
fn given_the_lid_key_when_pressed_twice_then_the_lid_closes_and_opens() {
    let tick = |keys: &mut Keys, code| {
        keys.handle(&press(code));
        keys.handle(&release(code));
        keys.hinge_closed()
    };
    let mut keys = Keys::new(true);
    let states = (
        tick(&mut keys, KeyCode::Char('`')),
        tick(&mut keys, KeyCode::Char('`')),
    );
    assert_eq!(states, (true, false));
}

#[test]
fn given_the_lid_key_on_a_legacy_terminal_when_its_repeat_presses_arrive_then_it_toggles_once() {
    let lid = press(KeyCode::Char('`'));
    let keys = legacy(&[lid, lid, lid]);
    assert!(keys.hinge_closed());
}

#[test]
fn given_the_lid_key_on_a_legacy_terminal_when_it_runs_down_and_is_pressed_again_then_it_opens() {
    let lid = press(KeyCode::Char('`'));
    let mut keys = legacy(&[lid]);
    pass(&mut keys, FALLBACK_HOLD_FRAMES);
    keys.handle(&lid);
    assert!(!keys.hinge_closed());
}

#[test]
fn given_a_closed_lid_when_focus_is_lost_then_it_stays_closed() {
    let mut keys = reported(&[press(KeyCode::Char('`'))]);
    keys.release_all();
    assert!(keys.hinge_closed());
}

#[test]
fn given_the_lid_key_when_pressed_then_no_button_is_held() {
    assert_eq!(
        reported(&[press(KeyCode::Char('`'))]).buttons(),
        Buttons::empty()
    );
}

#[test]
fn given_buttons_mic_and_lid_when_the_input_is_built_then_it_carries_all_three() {
    let keys = reported(&[
        press(KeyCode::Char('x')),
        press(KeyCode::Char('m')),
        press(KeyCode::Char('`')),
    ]);
    assert_eq!(
        keys.input(),
        Input::new()
            .with_buttons(Buttons::A)
            .with_mic(true)
            .with_hinge_closed(true)
    );
}

#[test]
fn given_no_key_when_the_input_is_built_then_it_is_the_untouched_console() {
    assert_eq!(Keys::new(true).input(), Input::new());
}

#[test]
fn given_a_new_keys_when_asked_then_it_says_whether_release_is_reported() {
    assert_eq!(
        (
            Keys::new(true).release_reported(),
            Keys::new(false).release_reported()
        ),
        (true, false)
    );
}

#[test]
fn given_the_key_table_when_each_key_is_indexed_then_it_finds_its_own_slot() {
    let misplaced: Vec<usize> = KEYS
        .iter()
        .enumerate()
        .filter(|(slot, key)| key.index() != *slot)
        .map(|(slot, _)| slot)
        .collect();
    assert_eq!(misplaced, Vec::<usize>::new());
}
