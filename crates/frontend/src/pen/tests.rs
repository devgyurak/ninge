// SPDX-License-Identifier: Apache-2.0
//! Tests of `pen`: the mouse report to a panel point in both units, and the pen's press, drag and
//! release. A table goes through a harness that returns the rows that differ.

use super::*;
use crossterm::event::KeyModifiers;

/// The bottom screen at cell (10, 5), 20 x 10 cells of 10 x 20 pixels: pixels 100..300 across and
/// 100..300 down.
const BOTTOM: Rect = Rect::new(10, 5, 20, 10);

const fn target(unit: Unit) -> Target {
    Target {
        unit,
        bottom: BOTTOM,
        cell: (10, 20),
    }
}

const fn at(x: u8, y: u8) -> Option<Touch> {
    Some(Touch { x, y })
}

/// The rows `(name, column, row, want)` whose point is not the one wanted.
fn failures(
    unit: Unit,
    rows: &[(&'static str, u16, u16, Option<Touch>)],
) -> Vec<(&'static str, Option<Touch>)> {
    rows.iter()
        .filter(|(_, c, r, want)| touch_at(*c, *r, &target(unit)) != *want)
        .map(|(name, c, r, _)| (*name, touch_at(*c, *r, &target(unit))))
        .collect()
}

#[test]
fn given_pixel_reports_when_mapped_then_the_corners_and_the_centre_scale_to_the_panel() {
    let rows = [
        ("top left", 100, 100, at(0, 0)),
        ("bottom right", 299, 299, at(254, 191)),
        ("top right", 299, 100, at(254, 0)),
        ("bottom left", 100, 299, at(0, 191)),
        ("centre", 200, 200, at(128, 96)),
    ];
    assert_eq!(failures(Unit::Pixels, &rows), vec![]);
}

#[test]
fn given_pixel_reports_one_past_each_edge_when_mapped_then_they_are_outside() {
    let rows = [
        ("left", 99, 200, None),
        ("right", 300, 200, None),
        ("above", 200, 99, None),
        ("below", 200, 300, None),
    ];
    assert_eq!(failures(Unit::Pixels, &rows), vec![]);
}

#[test]
fn given_cell_reports_when_mapped_then_each_lands_on_the_centre_of_its_cell() {
    let rows = [
        ("first cell", 10, 5, at(6, 9)),
        ("last cell", 29, 14, at(249, 182)),
        ("middle column", 20, 10, at(134, 105)),
    ];
    assert_eq!(failures(Unit::Cells, &rows), vec![]);
}

#[test]
fn given_cell_reports_one_past_each_edge_when_mapped_then_they_are_outside() {
    let rows = [
        ("left", 9, 10, None),
        ("right", 30, 10, None),
        ("above", 20, 4, None),
        ("below", 20, 15, None),
        ("origin", 0, 0, None),
    ];
    assert_eq!(failures(Unit::Cells, &rows), vec![]);
}

#[test]
fn given_a_fallback_cell_size_when_a_pixel_report_is_mapped_then_the_box_follows_that_size() {
    let fallback = Target {
        cell: (8, 16),
        ..target(Unit::Pixels)
    };
    // Box: 80..240 across, 80..240 down.
    assert_eq!(
        (
            touch_at(80, 80, &fallback),
            touch_at(79, 80, &fallback),
            touch_at(240, 80, &fallback)
        ),
        (at(0, 0), None, None)
    );
}

#[test]
fn given_a_screen_with_no_area_when_a_report_is_mapped_then_it_is_outside() {
    let empty = Target {
        bottom: Rect::new(10, 5, 0, 0),
        ..target(Unit::Cells)
    };
    assert_eq!(touch_at(10, 5, &empty), None);
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

const LEFT: MouseButton = MouseButton::Left;

/// The pen after `events`, in pixels.
fn pen_after(events: &[MouseEvent]) -> Pen {
    let mut pen = Pen::new();
    events
        .iter()
        .for_each(|e| pen.handle(e, &target(Unit::Pixels)));
    pen
}

#[test]
fn given_a_left_press_on_the_screen_when_handled_then_the_pen_is_down_at_the_point() {
    let pen = pen_after(&[mouse(MouseEventKind::Down(LEFT), 100, 100)]);
    assert_eq!(pen.touch(), at(0, 0));
}

#[test]
fn given_a_left_press_off_the_screen_when_handled_then_the_pen_stays_up() {
    let pen = pen_after(&[mouse(MouseEventKind::Down(LEFT), 50, 50)]);
    assert_eq!(pen.touch(), None);
}

#[test]
fn given_a_drag_when_handled_then_the_pen_follows_the_point() {
    let pen = pen_after(&[
        mouse(MouseEventKind::Down(LEFT), 100, 100),
        mouse(MouseEventKind::Drag(LEFT), 200, 200),
    ]);
    assert_eq!(pen.touch(), at(128, 96));
}

#[test]
fn given_a_drag_off_the_screen_when_handled_then_the_pen_lifts_and_returns_on_re_entry() {
    let mut pen = Pen::new();
    let t = target(Unit::Pixels);
    pen.handle(&mouse(MouseEventKind::Down(LEFT), 200, 200), &t);
    pen.handle(&mouse(MouseEventKind::Drag(LEFT), 400, 200), &t);
    let off = pen.touch();
    pen.handle(&mouse(MouseEventKind::Drag(LEFT), 100, 100), &t);
    assert_eq!((off, pen.touch()), (None, at(0, 0)));
}

#[test]
fn given_a_release_when_handled_then_the_pen_lifts() {
    let pen = pen_after(&[
        mouse(MouseEventKind::Down(LEFT), 200, 200),
        mouse(MouseEventKind::Up(LEFT), 200, 200),
    ]);
    assert_eq!(pen.touch(), None);
}

#[test]
fn given_other_buttons_or_motion_when_handled_then_the_pen_does_not_move() {
    let right = MouseButton::Right;
    let pen = pen_after(&[
        mouse(MouseEventKind::Down(right), 200, 200),
        mouse(MouseEventKind::Moved, 200, 200),
        mouse(MouseEventKind::ScrollDown, 200, 200),
    ]);
    assert_eq!(pen.touch(), None);
}

#[test]
fn given_a_right_click_while_the_pen_is_down_when_handled_then_the_pen_stays_where_it_was() {
    let pen = pen_after(&[
        mouse(MouseEventKind::Down(LEFT), 200, 200),
        mouse(MouseEventKind::Down(MouseButton::Right), 100, 100),
        mouse(MouseEventKind::Up(MouseButton::Right), 100, 100),
    ]);
    assert_eq!(pen.touch(), at(128, 96));
}

#[test]
fn given_a_pen_down_when_lifted_then_it_is_up() {
    let mut pen = pen_after(&[mouse(MouseEventKind::Down(LEFT), 200, 200)]);
    pen.lift();
    assert_eq!(pen.touch(), None);
}
