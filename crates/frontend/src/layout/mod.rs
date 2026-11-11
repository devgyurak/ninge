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
mod tests;
