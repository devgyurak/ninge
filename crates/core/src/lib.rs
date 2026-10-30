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
    // The ROM image is an input, not machine state: it is skipped on save and a savestate
    // loads onto a machine that was handed the ROM again, so a savestate never embeds the
    // (possibly commercial) image. Defaults to empty.
    // TODO(phase2): the savestate loader (#19) must meet these constraints:
    //  1. Re-attach the ROM and check it matches the state; a machine without a ROM must
    //     refuse to run.
    //  2. The derived `PartialEq` compares `rom` too, so a round-trip comparison must
    //     exclude it.
    //  3. `Clone` copies the whole ROM; revisit before rewind and run-ahead.
    #[serde(skip)]
    rom: Vec<u8>,
    // Frames completed by `run_frame`; plain state, so it is serialized. A savestate
    // without it is refused, never half-loaded (AGENTS.md §2 rule 5).
    frame_count: u64,
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
        Self {
            top: blank_screen(),
            bottom: blank_screen(),
            rom: Vec::new(),
            frame_count: 0,
        }
    }

    /// Hands the machine a ROM image, kept verbatim.
    ///
    /// Loading a ROM starts from power-on; every other piece of state is reset. This is the
    /// host API's contract, not hardware behavior, and a Phase 0 placeholder.
    ///
    /// Any length is accepted, including empty or odd: the image is guest-controlled input
    /// and never panics the core.
    pub fn load_rom(&mut self, rom: Vec<u8>) {
        // TODO(phase2): header parse and direct boot (#17).
        *self = Self::new();
        self.rom = rom;
    }

    /// Returns the loaded ROM image; empty until [`Machine::load_rom`] is called.
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Runs the machine for one frame.
    pub fn run_frame(&mut self) {
        // TODO(phase2): execution entry point (#17) arrives with the ARM9 bring-up; until then a
        // frame only advances the counter and leaves both screens black.
        self.frame_count = self.frame_count.wrapping_add(1);
    }

    /// Returns how many frames [`Machine::run_frame`] has completed.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
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
    fn given_savestate_map_with_frame_count_when_deserializing_machine_then_both_screens_are_full_size()
     {
        // TODO(phase2): save -> load round trip with the savestate encoder (#19).
        let map = MapDeserializer::<_, Error>::new([("frame_count", 0u32)].into_iter());
        let machine = Machine::deserialize(map).expect("a map with frame_count is a valid Machine");
        let lengths = [
            machine.frame(Screen::Top).len(),
            machine.frame(Screen::Bottom).len(),
        ];
        assert_eq!(lengths, [49_152, 49_152]);
    }

    #[test]
    fn given_rom_bytes_when_loading_then_rom_is_kept_verbatim() {
        let mut machine = Machine::new();
        machine.load_rom(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
        assert_eq!(machine.rom(), &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
    }

    #[test]
    fn given_new_machine_when_running_one_frame_then_frame_count_is_one() {
        let mut machine = Machine::new();
        machine.run_frame();
        assert_eq!(machine.frame_count(), 1);
    }

    #[test]
    fn given_new_machine_when_reading_counters_then_frame_count_is_zero_and_rom_is_empty() {
        let machine = Machine::new();
        assert_eq!((machine.frame_count(), machine.rom().len()), (0, 0));
    }

    #[test]
    fn given_empty_rom_when_running_a_frame_then_rom_stays_empty_and_frame_count_is_one() {
        let mut machine = Machine::new();
        machine.load_rom(Vec::new());
        machine.run_frame();
        assert_eq!((machine.rom().len(), machine.frame_count()), (0, 1));
    }

    #[test]
    fn given_odd_length_rom_when_running_a_frame_then_rom_is_unchanged_and_frame_count_is_one() {
        let mut machine = Machine::new();
        machine.load_rom(vec![1, 2, 3]);
        machine.run_frame();
        assert_eq!(
            (machine.rom().to_vec(), machine.frame_count()),
            (vec![1, 2, 3], 1)
        );
    }

    #[test]
    fn given_three_frames_run_when_reading_frame_count_then_it_is_three() {
        let mut machine = Machine::new();
        machine.run_frame();
        machine.run_frame();
        machine.run_frame();
        assert_eq!(machine.frame_count(), 3);
    }

    #[test]
    fn given_savestate_map_with_frame_count_7_when_deserializing_machine_then_rom_is_empty_and_frame_count_is_seven()
     {
        let map = MapDeserializer::<_, Error>::new([("frame_count", 7u32)].into_iter());
        let machine = Machine::deserialize(map).expect("a map with frame_count is a valid Machine");
        assert_eq!((machine.rom().len(), machine.frame_count()), (0, 7));
    }

    #[test]
    fn given_savestate_map_without_frame_count_when_deserializing_machine_then_it_is_refused() {
        let map = MapDeserializer::<_, Error>::new(std::iter::empty::<(&str, u32)>());
        assert!(Machine::deserialize(map).is_err());
    }

    #[test]
    fn given_machine_that_ran_two_frames_when_loading_rom_then_frame_count_is_zero() {
        let mut machine = Machine::new();
        machine.run_frame();
        machine.run_frame();
        machine.load_rom(vec![1, 2, 3]);
        assert_eq!(machine.frame_count(), 0);
    }
}
