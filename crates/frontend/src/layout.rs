// SPDX-License-Identifier: Apache-2.0
//! Screen and panel layout: pure arithmetic from terminal size to cell rectangles.
//!
//! Only the vertical (original) arrangement exists; layout modes and swap are #62.

use crate::diacritics::DIACRITICS;
use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use ratatui::layout::Rect;

/// Assumed cell size in pixels when the terminal does not report one.
pub const FALLBACK_CELL_PX: (u16, u16) = (8, 16);

/// Widest side panel column, in cells.
const PANEL_MAX_COLS: u16 = 28;

/// Terminal size in cells plus the pixel size of one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    /// Width in cells.
    pub cols: u16,
    /// Height in cells.
    pub rows: u16,
    /// Cell width in pixels.
    pub cell_w: u16,
    /// Cell height in pixels.
    pub cell_h: u16,
    /// False when the cell size is the fallback guess.
    pub px_known: bool,
}

impl TermSize {
    /// Builds a size from a window report; zero pixel fields fall back to
    /// [`FALLBACK_CELL_PX`] and clear `px_known`.
    pub fn from_window(cols: u16, rows: u16, px_w: u16, px_h: u16) -> Self {
        let known = cols > 0 && rows > 0 && px_w >= cols && px_h >= rows;
        let (cell_w, cell_h) = if known {
            (px_w / cols, px_h / rows)
        } else {
            FALLBACK_CELL_PX
        };
        Self {
            cols,
            rows,
            cell_w,
            cell_h,
            px_known: known,
        }
    }
}

/// Where everything goes, in terminal cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Areas {
    /// Top screen image.
    pub top: Rect,
    /// Bottom screen image.
    pub bottom: Rect,
    /// Register panel placeholder.
    pub registers: Rect,
    /// Disassembly panel placeholder.
    pub disasm: Rect,
}

/// Lays the two screens (aspect-correct, stacked, centred) left of two panels.
/// `None` means the terminal is too small to show a screen.
pub fn compute(size: TermSize) -> Option<Areas> {
    if size.cell_w == 0 || size.cell_h == 0 {
        return None;
    }
    let panel_w = PANEL_MAX_COLS.min(size.cols / 3);
    let left_w = size.cols - panel_w;
    let half_h = size.rows / 2;
    let (cw, ch) = (u32::from(size.cell_w), u32::from(size.cell_h));
    let (sw, sh) = (SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
    let table = DIACRITICS.len() as u32;
    let rows_by_width = u32::from(left_w) * cw * sh / (sw * ch);
    let rows = u32::from(half_h).min(rows_by_width).min(table);
    let cols = (rows * ch * sw / (sh * cw))
        .min(u32::from(left_w))
        .min(table);
    if rows == 0 || cols == 0 {
        return None;
    }
    let (cols, rows) = (cols as u16, rows as u16);
    let x = (left_w - cols) / 2;
    let y = (size.rows - 2 * rows) / 2;
    Some(Areas {
        top: Rect::new(x, y, cols, rows),
        bottom: Rect::new(x, y + rows, cols, rows),
        registers: Rect::new(left_w, 0, panel_w, half_h),
        disasm: Rect::new(left_w, half_h, panel_w, size.rows - half_h),
    })
}

#[cfg(test)]
mod tests {
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
}
