// SPDX-License-Identifier: Apache-2.0
//! ninge-core: the emulator itself. Pure logic with no IO; inputs in, frames out.

use serde::{Deserialize, Serialize};

/// Width of each screen in pixels (`docs/hw/display.md`).
pub const SCREEN_WIDTH: usize = 256;
/// Height of each screen in pixels (`docs/hw/display.md`).
pub const SCREEN_HEIGHT: usize = 192;

/// One of the two physical screens.
///
/// `Top` and `Bottom` are the physical screens, not graphics engines A/B: which engine shows
/// on which screen is set by POWCNT1 (Phase 2-3). See `docs/hw/display.md`.
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
    // `Vec`s because serde does not derive for arrays this large. The buffers are derived
    // output, not savestate data: they are skipped on save, and a load recreates them blank
    // (restoring the invariant) until the next frame is drawn. The savestate format itself
    // is owned by #19.
    #[serde(skip, default = "blank_screen")]
    top: Vec<u32>,
    #[serde(skip, default = "blank_screen")]
    bottom: Vec<u32>,
}

/// A black screen of exactly `SCREEN_WIDTH * SCREEN_HEIGHT` pixels.
fn blank_screen() -> Vec<u32> {
    vec![0x0000_0000; SCREEN_WIDTH * SCREEN_HEIGHT]
}

impl Machine {
    /// Creates a machine with both screens black.
    ///
    /// Black is a Phase 0 placeholder value, not the hardware power-on state.
    pub fn new() -> Self {
        // TODO(phase2): execution entry point (run a frame) arrives with the ARM9 bring-up.
        Self {
            top: blank_screen(),
            bottom: blank_screen(),
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
    use serde::de::value::{Error, MapDeserializer};

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
    fn given_empty_savestate_map_when_deserializing_machine_then_both_screens_are_full_size() {
        // TODO(phase2): save -> load round trip with the savestate encoder (#19).
        let empty = MapDeserializer::<_, Error>::new(std::iter::empty::<(&str, u32)>());
        let machine = Machine::deserialize(empty).expect("an empty map is a valid Machine");
        let lengths = [
            machine.frame(Screen::Top).len(),
            machine.frame(Screen::Bottom).len(),
        ];
        assert_eq!(lengths, [49_152, 49_152]);
    }
}
