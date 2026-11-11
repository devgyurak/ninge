// SPDX-License-Identifier: Apache-2.0
//! Unicode-placeholder cells: a grid of characters a kitty-protocol terminal replaces
//! with slices of an image's virtual placement.

use crate::diacritics::diacritic;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

/// The placeholder base character.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// The cell text for (`row`, `col`): placeholder plus both diacritics. Every cell names
/// its row and column because ratatui redraws only changed cells, so the terminal's
/// inference from the previous cell cannot be relied on. `None` past the table.
pub fn cell_symbol(row: usize, col: usize) -> Option<String> {
    Some(
        [PLACEHOLDER, diacritic(row)?, diacritic(col)?]
            .iter()
            .collect(),
    )
}

/// The foreground colour that encodes a 24-bit image id.
pub fn id_color(image_id: u32) -> Color {
    Color::Rgb(
        (image_id >> 16) as u8,
        (image_id >> 8) as u8,
        image_id as u8,
    )
}

/// Widget that fills its area with placeholder cells for `image_id`.
pub struct ScreenImage {
    /// Image id the cells refer to.
    pub image_id: u32,
}

impl Widget for ScreenImage {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for row in 0..area.height {
            for col in 0..area.width {
                let symbol = cell_symbol(usize::from(row), usize::from(col));
                let cell = buf.cell_mut((area.x + col, area.y + row));
                if let (Some(symbol), Some(cell)) = (symbol, cell) {
                    cell.set_symbol(&symbol).set_fg(id_color(self.image_id));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
