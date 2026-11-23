// SPDX-License-Identifier: Apache-2.0
//! Tests of `cursor`: the mode, movement and clamping, the pen from Space and from a drag, and the
//! crosshair's pixels.

use super::*;
use crate::input::FALLBACK_HOLD_FRAMES;

fn event(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
    KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
}

fn press(c: char) -> KeyEvent {
    event(KeyCode::Char(c), KeyEventKind::Press)
}

fn release(c: char) -> KeyEvent {
    event(KeyCode::Char(c), KeyEventKind::Release)
}

fn tab() -> KeyEvent {
    event(KeyCode::Tab, KeyEventKind::Press)
}

/// A cursor on a terminal that reports release, with cursor mode on and `events` taken.
fn active(events: &[KeyEvent]) -> Cursor {
    let mut cursor = Cursor::new(true);
    cursor.handle(&tab());
    feed(&mut cursor, events);
    cursor
}

/// Takes `events` in order.
fn feed(cursor: &mut Cursor, events: &[KeyEvent]) {
    events.iter().for_each(|e| cursor.handle(e));
}

const fn at(x: u8, y: u8) -> Option<Touch> {
    Some(Touch { x, y })
}

/// The rows `(name, keys, want)` whose moves do not end at the position wanted.
fn move_failures(rows: &[(&'static str, &str, (u8, u8))]) -> Vec<(&'static str, (u8, u8))> {
    rows.iter()
        .map(|(name, keys, want)| {
            let events: Vec<KeyEvent> = keys.chars().map(press).collect();
            (*name, active(&events).position(), *want)
        })
        .filter(|(_, got, want)| got != want)
        .map(|(name, got, _)| (name, got))
        .collect()
}

#[test]
fn given_a_new_cursor_when_asked_then_it_is_off_and_at_the_centre() {
    let cursor = Cursor::new(true);
    assert_eq!((cursor.active(), cursor.position()), (false, (128, 96)));
}

#[test]
fn given_cursor_mode_when_hjkl_are_pressed_then_it_moves_by_the_step() {
    let rows = [
        ("h", "h", (128 - STEP, 96)),
        ("l", "l", (128 + STEP, 96)),
        ("k", "k", (128, 96 - STEP)),
        ("j", "j", (128, 96 + STEP)),
        ("two lefts and a down", "hhj", (128 - 2 * STEP, 96 + STEP)),
        ("upper case", "HJ", (128 - STEP, 96 + STEP)),
    ];
    assert_eq!(move_failures(&rows), vec![]);
}

#[test]
fn given_the_edges_when_the_cursor_keeps_moving_then_it_stops_inside_the_panel() {
    let many = |c: char| c.to_string().repeat(100);
    let rows = [
        ("left", many('h'), (0, 96)),
        ("right", many('l'), (255, 96)),
        ("up", many('k'), (128, 0)),
        ("down", many('j'), (128, 191)),
    ];
    let failures: Vec<&str> = rows
        .iter()
        .filter(|(_, keys, want)| {
            let events: Vec<KeyEvent> = keys.chars().map(press).collect();
            active(&events).position() != *want
        })
        .map(|(name, ..)| *name)
        .collect();
    assert_eq!(failures, Vec::<&str>::new());
}

#[test]
fn given_cursor_mode_off_when_hjkl_space_and_d_are_pressed_then_nothing_changes() {
    let mut cursor = Cursor::new(true);
    feed(
        &mut cursor,
        &[press('h'), press('j'), press(' '), press('d')],
    );
    assert_eq!(cursor, Cursor::new(true));
}

#[test]
fn given_cursor_mode_when_tab_is_pressed_again_then_it_is_off() {
    let mut cursor = active(&[event(KeyCode::Tab, KeyEventKind::Release)]);
    cursor.handle(&tab());
    assert!(!cursor.active());
}

#[test]
fn given_a_repeat_move_when_it_arrives_then_the_cursor_moves_again() {
    let repeat = event(KeyCode::Char('l'), KeyEventKind::Repeat);
    assert_eq!(
        active(&[press('l'), repeat]).position(),
        (128 + 2 * STEP, 96)
    );
}

#[test]
fn given_cursor_mode_when_space_is_held_then_the_pen_is_down_at_the_cursor() {
    assert_eq!(
        active(&[press('l'), press(' ')]).touch(),
        at(128 + STEP, 96)
    );
}

#[test]
fn given_space_held_when_it_is_released_then_the_pen_is_up() {
    assert_eq!(active(&[press(' '), release(' ')]).touch(), None);
}

#[test]
fn given_space_held_when_the_cursor_moves_then_the_pen_follows() {
    assert_eq!(
        active(&[press(' '), press('j')]).touch(),
        at(128, 96 + STEP)
    );
}

#[test]
fn given_no_space_when_cursor_mode_is_on_then_the_pen_is_up() {
    assert_eq!(active(&[]).touch(), None);
}

#[test]
fn given_a_legacy_terminal_when_space_is_tapped_then_the_pen_is_down_for_the_fixed_count() {
    let mut cursor = Cursor::new(false);
    cursor.handle(&tab());
    cursor.handle(&press(' '));
    let down: Vec<bool> = (0..=FALLBACK_HOLD_FRAMES)
        .map(|_| {
            let touching = cursor.touch().is_some();
            cursor.end_frame();
            touching
        })
        .collect();
    let mut expected = vec![true; usize::from(FALLBACK_HOLD_FRAMES)];
    expected.push(false);
    assert_eq!(down, expected);
}

#[test]
fn given_d_when_pressed_then_the_pen_stays_down_after_space_is_gone_and_while_moving() {
    let cursor = active(&[press('d'), release('d'), press('l')]);
    assert_eq!(cursor.touch(), at(128 + STEP, 96));
}

#[test]
fn given_a_drag_when_d_is_pressed_again_then_the_pen_lifts() {
    let cursor = active(&[press('d'), release('d'), press('d')]);
    assert_eq!(cursor.touch(), None);
}

#[test]
fn given_d_on_a_legacy_terminal_when_repeat_presses_arrive_then_the_drag_toggles_once() {
    let mut cursor = Cursor::new(false);
    cursor.handle(&tab());
    feed(&mut cursor, &[press('d'), press('d'), press('d')]);
    assert!(cursor.touch().is_some());
}

#[test]
fn given_tab_on_a_legacy_terminal_when_repeat_presses_arrive_then_the_mode_toggles_once() {
    let mut cursor = Cursor::new(false);
    feed(&mut cursor, &[tab(), tab(), tab()]);
    assert!(cursor.active());
}

#[test]
fn given_a_drag_when_the_mode_is_left_then_the_pen_is_up_and_a_later_return_starts_clean() {
    let mut cursor = active(&[press('d'), release('d')]);
    cursor.handle(&event(KeyCode::Tab, KeyEventKind::Release));
    cursor.handle(&tab());
    cursor.handle(&event(KeyCode::Tab, KeyEventKind::Release));
    cursor.handle(&tab());
    assert_eq!(cursor.touch(), None);
}

#[test]
fn given_a_drag_when_focus_is_lost_then_the_pen_is_up_but_the_mode_and_place_stay() {
    let mut cursor = active(&[press('d'), release('d'), press('l')]);
    cursor.release_all();
    assert_eq!(
        (cursor.touch(), cursor.active(), cursor.position()),
        (None, true, (128 + STEP, 96))
    );
}

#[test]
fn given_ctrl_with_a_cursor_key_when_pressed_then_it_does_nothing() {
    let ctrl_l = KeyEvent::new_with_kind(
        KeyCode::Char('l'),
        KeyModifiers::CONTROL,
        KeyEventKind::Press,
    );
    assert_eq!(active(&[ctrl_l]).position(), (128, 96));
}

#[test]
fn given_a_key_without_a_cursor_meaning_when_pressed_then_nothing_changes() {
    assert_eq!(active(&[press('x'), press('q')]), active(&[]));
}

// ------------------------------------------------------------------------------------ overlay

/// A black panel.
fn black() -> Vec<u32> {
    vec![0; SCREEN_WIDTH * SCREEN_HEIGHT]
}

/// The pixels, as `(x, y)`, that are not black.
fn lit(pixels: &[u32]) -> Vec<(usize, usize)> {
    pixels
        .iter()
        .enumerate()
        .filter(|(_, p)| **p != 0)
        .map(|(i, _)| (i % SCREEN_WIDTH, i / SCREEN_WIDTH))
        .collect()
}

#[test]
fn given_a_black_panel_when_the_crosshair_is_drawn_then_a_plus_of_white_pixels_appears() {
    let mut pixels = black();
    overlay(&mut pixels, (100, 50));
    let mut want: Vec<(usize, usize)> = (97..=103).map(|x| (x, 50)).collect();
    want.extend((47..=53).filter(|y| *y != 50).map(|y| (100, y)));
    want.sort_by_key(|&(x, y)| (y, x));
    assert_eq!(
        (lit(&pixels), pixels[50 * SCREEN_WIDTH + 100]),
        (want, 0x00FF_FFFF)
    );
}

#[test]
fn given_a_white_panel_when_the_crosshair_is_drawn_then_its_pixels_turn_black() {
    let mut pixels = vec![0x00FF_FFFF; SCREEN_WIDTH * SCREEN_HEIGHT];
    overlay(&mut pixels, (100, 50));
    assert_eq!(
        (
            pixels[50 * SCREEN_WIDTH + 100],
            pixels[50 * SCREEN_WIDTH + 97]
        ),
        (0, 0)
    );
}

#[test]
fn given_the_crosshair_drawn_twice_when_compared_with_the_original_then_it_is_gone() {
    let mut pixels: Vec<u32> = (0..SCREEN_WIDTH * SCREEN_HEIGHT)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761) & 0x00FF_FFFF)
        .collect();
    let original = pixels.clone();
    overlay(&mut pixels, (100, 50));
    overlay(&mut pixels, (100, 50));
    assert_eq!(pixels, original);
}

#[test]
fn given_the_corners_when_the_crosshair_is_drawn_then_it_is_clipped_to_the_panel() {
    let drawn = |at: (u8, u8)| {
        let mut pixels = black();
        overlay(&mut pixels, at);
        lit(&pixels).len()
    };
    // Four arms of 3 and the center is 13; at a corner only two arms and the center are left.
    assert_eq!(
        (
            drawn((0, 0)),
            drawn((255, 0)),
            drawn((0, 191)),
            drawn((255, 191))
        ),
        (7, 7, 7, 7)
    );
}

#[test]
fn given_a_crosshair_when_drawn_then_the_alpha_byte_is_left_alone() {
    let mut pixels = vec![0xFF00_0000; SCREEN_WIDTH * SCREEN_HEIGHT];
    overlay(&mut pixels, (10, 10));
    assert_eq!(pixels[10 * SCREEN_WIDTH + 10], 0xFFFF_FFFF);
}
