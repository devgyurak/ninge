// SPDX-License-Identifier: Apache-2.0
//! Tests of `layout`.

use super::*;

fn size(cols: u16, rows: u16, cw: u16, ch: u16) -> TermSize {
    TermSize {
        cols,
        rows,
        cell_w: cw,
        cell_h: ch,
        px_known: true,
    }
}

/// Cases whose screen rect is not 4:3 in pixels (within one cell of rounding).
fn aspect_failures() -> Vec<(u16, u16, u16, u16)> {
    let cases = [
        (120, 50, 10, 20),
        (200, 60, 8, 16),
        (80, 24, 9, 18),
        (300, 100, 7, 15),
    ];
    cases
        .into_iter()
        .filter(|&(c, r, w, h)| {
            let a = compute(size(c, r, w, h)).expect("fits");
            let px_w = u32::from(a.top.width) * u32::from(w);
            let px_h = u32::from(a.top.height) * u32::from(h);
            let off = (px_w * 192).abs_diff(px_h * 256);
            // Flooring the column count loses less than one cell of width.
            off > 192 * u32::from(w)
        })
        .collect()
}

#[test]
fn given_several_cell_sizes_when_computing_then_screens_keep_4_by_3() {
    assert_eq!(aspect_failures(), Vec::new());
}

#[test]
fn given_roomy_terminal_when_computing_then_screens_are_stacked_and_centred() {
    let a = compute(size(120, 50, 10, 20)).expect("fits");
    let left_w = 92;
    assert_eq!(
        (a.top.x, a.bottom.x, a.bottom.y),
        (
            (left_w - a.top.width) / 2,
            (left_w - a.top.width) / 2,
            a.top.y + a.top.height
        )
    );
}

#[test]
fn given_zero_cell_size_when_computing_then_none_instead_of_dividing_by_zero() {
    assert_eq!(compute(size(120, 50, 0, 0)), None);
}

#[test]
fn given_tiny_terminal_when_computing_then_none() {
    assert_eq!(compute(size(2, 1, 8, 16)), None);
}

#[test]
fn given_huge_terminal_when_computing_then_cells_clamp_to_diacritic_table() {
    let a = compute(size(2000, 2000, 1, 1)).expect("fits");
    assert_eq!((a.top.width <= 297, a.top.height <= 297), (true, true));
}

#[test]
fn given_wide_terminal_when_computing_then_panels_take_a_side_column() {
    let a = compute(size(120, 50, 10, 20)).expect("fits");
    assert_eq!((a.registers.x, a.registers.width, a.disasm.y), (92, 28, 25));
}

#[test]
fn given_window_report_when_pixels_known_then_cell_size_is_divided() {
    assert_eq!(
        TermSize::from_window(100, 50, 1000, 1000),
        size(100, 50, 10, 20)
    );
}

#[test]
fn given_window_report_without_pixels_when_building_then_fallback_is_flagged() {
    let s = TermSize::from_window(100, 50, 0, 0);
    assert_eq!((s.cell_w, s.cell_h, s.px_known), (8, 16, false));
}
