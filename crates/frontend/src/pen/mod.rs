// SPDX-License-Identifier: Apache-2.0
//! The mouse as the touch pen: where a terminal mouse report lands on the bottom screen, and
//! whether the pen is down. Pure arithmetic and state, no terminal (`docs/hw/touch-power.md` for
//! what the console does with the point).
//!
//! The terminal reports a position in cells, or in pixels when SGR-Pixels (mode 1016) is on.
//! crossterm turns the report's 1-based numbers into 0-based ones, so both units are taken as a
//! 0-based offset from the window's top left. A point in pixels lands at its scaled place inside
//! the bottom image; a point in cells lands at the center of its cell, so a terminal that cannot
//! report pixels still works.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH, Touch};
use ratatui::layout::Rect;

/// What a mouse report's column and row count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Cells of the terminal grid.
    Cells,
    /// Pixels of the window (SGR-Pixels).
    Pixels,
}

/// Where the bottom screen is on the terminal, and how a report is counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// The unit of a report's column and row.
    pub unit: Unit,
    /// The cells the bottom screen's image covers.
    pub bottom: Rect,
    /// One cell's size in pixels: the real one, or the fallback guess (`layout::TermSize`).
    pub cell: (u16, u16),
}

/// The panel point under a report at (`column`, `row`), or `None` outside the bottom screen.
pub fn touch_at(column: u16, row: u16, target: &Target) -> Option<Touch> {
    let (w, h) = (SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
    let (column, row) = (u32::from(column), u32::from(row));
    let bottom = target.bottom;
    // The box the report is placed in, and the position inside it, in the box's own units;
    // `twice` counts half units, so a cell's center is a whole number.
    let (left, top, width, height, x, y, twice) = match target.unit {
        Unit::Pixels => {
            let (cw, ch) = (u32::from(target.cell.0), u32::from(target.cell.1));
            (
                u32::from(bottom.x) * cw,
                u32::from(bottom.y) * ch,
                u32::from(bottom.width) * cw,
                u32::from(bottom.height) * ch,
                column,
                row,
                1,
            )
        }
        Unit::Cells => (
            u32::from(bottom.x),
            u32::from(bottom.y),
            u32::from(bottom.width),
            u32::from(bottom.height),
            column,
            row,
            2,
        ),
    };
    let inside = |p: u32, start: u32, len: u32| p >= start && p - start < len;
    if width == 0 || height == 0 || !inside(x, left, width) || !inside(y, top, height) {
        return None;
    }
    // Pixels: the point itself; cells: the middle of the cell (2p + 1 half units).
    let along = |p: u32, start: u32, len: u32, screen: u32| {
        let half = (p - start) * twice + u32::from(twice == 2);
        (half * screen / (len * twice)).min(screen - 1) as u8
    };
    Some(Touch {
        x: along(x, left, width, w),
        y: along(y, top, height, h),
    })
}

/// Whether the left button holds the pen down, and where.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pen {
    touch: Option<Touch>,
}

impl Pen {
    /// The pen up.
    pub const fn new() -> Self {
        Self { touch: None }
    }

    /// Takes a mouse report: the left button's press and drag put the pen down at the point (or
    /// lift it when the point is outside the bottom screen: a stylus dragged off the panel stops
    /// touching, and touches again when it comes back), its release lifts it. Other buttons and
    /// plain motion change nothing.
    pub fn handle(&mut self, event: &MouseEvent, target: &Target) {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
                self.touch = touch_at(event.column, event.row, target);
            }
            MouseEventKind::Up(MouseButton::Left) => self.touch = None,
            _ => {}
        }
    }

    /// Lifts the pen, as when the terminal loses focus or the layout moves under it.
    pub fn lift(&mut self) {
        self.touch = None;
    }

    /// Where the pen touches the panel, if it is down.
    pub const fn touch(&self) -> Option<Touch> {
        self.touch
    }
}

#[cfg(test)]
mod tests;
