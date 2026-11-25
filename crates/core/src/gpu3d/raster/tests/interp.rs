// SPDX-License-Identifier: Apache-2.0
//! Colour along a polygon (`docs/hw/rasterizer.md`, "Colour along a polygon").

use super::*;
use proptest::prelude::*;

/// The red channel (5 bits) of every pixel in line `y` that is covered, left to right.
fn reds(lines: &[Line3d], y: usize) -> Vec<u16> {
    lines[y]
        .iter()
        .filter(|p| p.alpha != 0)
        .map(|p| p.color & 0x1F)
        .collect()
}

/// A triangle with a red corner of `a` at the left and `b` at the right, one W throughout.
fn gradient(a: u16, b: u16) -> Ram {
    scene(&[(
        opaque(1),
        vec![
            vertex(10, 10, 0, a),
            vertex(10, 90, 0, a),
            vertex(200, 50, 0, b),
        ],
    )])
}

/// The failures of a gradient from `a` to `b`: a channel outside the corners' range, or one that
/// steps the wrong way along the line.
fn gradient_failures(a: u16, b: u16) -> Vec<String> {
    let lines = draw(&gradient(a, b), 0, &regs());
    let row = reds(&lines, 50);
    let (lo, hi) = (a.min(b), a.max(b));
    let mut failures = vec![];
    row.iter()
        .filter(|&&r| r < lo || r > hi)
        .for_each(|r| failures.push(format!("{r} outside {lo}..={hi}")));
    row.windows(2)
        .filter(|w| (a < b && w[1] < w[0]) || (a > b && w[1] > w[0]))
        .for_each(|w| failures.push(format!("{} then {}", w[0], w[1])));
    failures
}

proptest! {
    #[test]
    fn given_two_corner_colours_when_a_line_is_drawn_then_the_channel_stays_between_them_and_is_monotone(
        a in 0u16..32,
        b in 0u16..32,
    ) {
        prop_assert_eq!(gradient_failures(a, b), Vec::<String>::new());
    }
}

#[test]
fn given_a_black_to_white_gradient_when_a_line_is_drawn_then_it_runs_from_dark_to_bright() {
    let lines = draw(&gradient(0, 0x1F), 0, &regs());
    let row = reds(&lines, 50);
    assert_eq!(
        (row.first().copied(), row.last().copied()),
        (Some(0), Some(31))
    );
}
