// SPDX-License-Identifier: Apache-2.0
//! ninge-runner: headless runner for tests, traces, and golden frames.

use ninge_core::{Machine, Screen};

/// Builds a machine and returns the top screen's frame.
// TODO(phase0): ROM loading (#3).
pub fn render_top_frame() -> Vec<u32> {
    Machine::new().frame(Screen::Top).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_fresh_runner_when_rendering_top_frame_then_it_has_one_screen_of_pixels() {
        assert_eq!(
            render_top_frame().len(),
            ninge_core::SCREEN_WIDTH * ninge_core::SCREEN_HEIGHT
        );
    }
}
