// SPDX-License-Identifier: Apache-2.0
//! ninge-core: the emulator itself. Pure logic with no IO; inputs in, frames out.

use serde::{Deserialize, Serialize};

/// Width of each screen in pixels (`docs/hw/display.md`).
pub const SCREEN_WIDTH: usize = 256;
/// Height of each screen in pixels (`docs/hw/display.md`).
pub const SCREEN_HEIGHT: usize = 192;

/// One of the two physical screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Screen {
    /// The upper screen.
    Top,
    /// The lower screen, which carries the touch panel.
    Bottom,
}

/// The whole emulated machine; it owns all emulator state.
///
/// Frame buffers are `u32` per pixel in xRGB8888 (`0x00RRGGBB`). That is the emulator's
/// host-side output format, not a hardware fact (`docs/hw/display.md`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    // Invariant: each buffer holds exactly SCREEN_WIDTH * SCREEN_HEIGHT pixels. They are
    // `Vec`s because serde does not derive for arrays this large.
    top: Vec<u32>,
    bottom: Vec<u32>,
}

impl Machine {
    /// Creates a machine with both screens black.
    ///
    /// Black is a Phase 0 placeholder value, not the hardware power-on state.
    pub fn new() -> Self {
        // TODO(phase2): execution entry point (run a frame) arrives with the ARM9 bring-up.
        Self {
            top: vec![0x0000_0000; SCREEN_WIDTH * SCREEN_HEIGHT],
            bottom: vec![0x0000_0000; SCREEN_WIDTH * SCREEN_HEIGHT],
        }
    }

    /// Returns the pixels of one screen, row by row, as xRGB8888.
    pub fn frame(&self, screen: Screen) -> &[u32] {
        match screen {
            Screen::Top => &self.top,
            Screen::Bottom => &self.bottom,
        }
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;

    fn assert_savestate_data<T: Serialize + DeserializeOwned>(_state: &T) {}

    #[test]
    fn given_new_machine_when_reading_each_screen_then_length_is_256_by_192() {
        let machine = Machine::new();
        let lengths = [
            machine.frame(Screen::Top).len(),
            machine.frame(Screen::Bottom).len(),
        ];
        assert_eq!(lengths, [49_152, 49_152]);
    }

    #[test]
    fn given_new_machine_when_reading_frames_then_every_pixel_is_black() {
        let machine = Machine::default();
        let all_black = [Screen::Top, Screen::Bottom]
            .iter()
            .all(|&s| machine.frame(s).iter().all(|&p| p == 0));
        assert!(all_black);
    }

    #[test]
    fn given_machine_when_checked_against_serde_bounds_then_it_is_savestate_data() {
        // TODO: the real save -> load round trip comes with the savestate encoder issue.
        assert_savestate_data(&Machine::new());
    }
}
