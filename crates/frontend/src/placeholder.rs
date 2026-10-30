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
mod tests {
    use super::*;

    #[test]
    fn given_row_and_column_when_building_cell_then_placeholder_has_both_diacritics() {
        assert_eq!(
            cell_symbol(1, 2).as_deref(),
            Some("\u{10EEEE}\u{30D}\u{30E}")
        );
    }

    #[test]
    fn given_column_past_table_when_building_cell_then_none() {
        assert_eq!(cell_symbol(0, 297), None);
    }

    #[test]
    fn given_image_id_when_coloring_then_id_is_the_24_bit_foreground() {
        assert_eq!(id_color(0x01_02_03), Color::Rgb(1, 2, 3));
    }

    #[test]
    fn given_widget_when_rendering_then_cells_hold_symbol_and_id_color() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 3));
        ScreenImage { image_id: 9 }.render(Rect::new(1, 1, 2, 2), &mut buf);
        let c = &buf[(2, 2)];
        assert_eq!(
            (c.symbol(), c.fg),
            ("\u{10EEEE}\u{30D}\u{30D}", Color::Rgb(0, 0, 9))
        );
    }

    #[test]
    fn given_rendered_grid_when_diffing_then_no_cell_is_skipped_as_wide() {
        let area = Rect::new(0, 0, 6, 4);
        let mut next = Buffer::empty(area);
        ScreenImage { image_id: 1 }.render(area, &mut next);
        assert_eq!(Buffer::empty(area).diff(&next).len(), 24);
    }

    #[test]
    fn given_area_outside_buffer_when_rendering_then_it_does_not_panic() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 2));
        ScreenImage { image_id: 1 }.render(Rect::new(0, 0, 5, 5), &mut buf);
        assert_eq!(buf[(1, 1)].symbol(), "\u{10EEEE}\u{30D}\u{30D}");
    }
}
