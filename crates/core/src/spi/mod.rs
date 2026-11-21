// SPDX-License-Identifier: Apache-2.0
//! The serial devices behind the SPI bus (`docs/hw/spi.md`): the synthetic firmware
//! (`docs/hw/firmware.md`). The register window that reaches them is `io::spi`.

pub mod firmware;
pub mod touch;

pub use firmware::{FIRMWARE_LEN, FirmwareSettings, Language, build, newest_user_settings};
pub use touch::Tsc;

/// The touch-screen calibration points the synthetic firmware stores, and the pair of points the
/// touch controller of #41 must use to turn a pixel into ADC values (`docs/hw/firmware.md`, "The
/// touch calibration constant").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchCalibration {
    /// ADC (12-bit) positions of the two points, as `(x, y)`.
    pub adc: [(u16, u16); 2],
    /// Screen positions of the two points, as `(x, y)`, originated at 1,1 as GBATEK's formula
    /// expects.
    pub screen: [(u8, u8); 2],
}

/// ADC counts per screen pixel on both axes. With this ratio GBATEK's conversion
/// `scr = (adc - adc1) * (scr2 - scr1) / (adc2 - adc1) + (scr1 - 1)` is exact in integers, and a
/// pixel `p` reads as the ADC value `ADC_PER_PIXEL * p` (`docs/hw/firmware.md`).
pub const ADC_PER_PIXEL: u16 = 16;

/// Our own calibration constant (`docs/hw/firmware.md`).
// TODO(hw-unverified): real consoles carry per-unit values; ours is chosen for exactness.
pub const CALIBRATION: TouchCalibration = TouchCalibration {
    adc: [(512, 512), (3568, 2544)],
    screen: [(33, 33), (224, 160)],
};

#[cfg(test)]
mod tests;
