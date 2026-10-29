// SPDX-License-Identifier: Apache-2.0
//! ninge: the terminal frontend. Terminal output arrives with issue #5.

use ninge_core::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH};

/// Application entry point; returns the process banner for now.
pub fn run() -> String {
    format!(
        "ninge: two screens of {SCREEN_WIDTH}x{SCREEN_HEIGHT} ({} pixels each)",
        Machine::new().frame(ninge_core::Screen::Top).len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_frontend_when_run_then_banner_reports_screen_size() {
        assert_eq!(run(), "ninge: two screens of 256x192 (49152 pixels each)");
    }
}
