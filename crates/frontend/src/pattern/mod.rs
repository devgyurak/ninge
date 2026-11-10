// SPDX-License-Identifier: Apache-2.0
//! Dummy frames shown when no ROM is given. Not an emulation decision: a moving pattern
//! that exercises the output path without a machine.

use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};

const BARS: [u32; 8] = [
    0x00FF_FFFF,
    0x00FF_FF00,
    0x0000_FFFF,
    0x0000_FF00,
    0x00FF_00FF,
    0x00FF_0000,
    0x0000_00FF,
    0x0000_0000,
];

/// An xRGB8888 frame of colour bars that scroll with `frame_no`, darker toward the
/// bottom. `phase` offsets the bars so the two screens differ. Pure: same inputs, same
/// pixels.
pub fn dummy_frame(frame_no: u64, phase: u32) -> Vec<u32> {
    let shift = (frame_no as u32).wrapping_mul(2).wrapping_add(phase * 32);
    (0..SCREEN_HEIGHT as u32)
        .flat_map(|y| {
            let shade = 255 - y * 128 / SCREEN_HEIGHT as u32;
            (0..SCREEN_WIDTH as u32).map(move |x| {
                let bar = BARS[(((x + shift) % SCREEN_WIDTH as u32) / 32) as usize];
                let ch = |sh: u32| (((bar >> sh) & 0xFF) * shade / 255) << sh;
                ch(16) | ch(8) | ch(0)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
