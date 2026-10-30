// SPDX-License-Identifier: Apache-2.0
//! Dummy frames for the display test. Not an emulation decision: the core draws nothing
//! yet, so the frontend shows a moving pattern to prove the output path.

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
mod tests {
    use super::*;

    #[test]
    fn given_same_frame_number_when_generating_twice_then_buffers_are_equal() {
        assert_eq!(dummy_frame(5, 0), dummy_frame(5, 0));
    }

    #[test]
    fn given_next_frame_number_when_generating_then_buffer_changes() {
        assert_ne!(dummy_frame(5, 0), dummy_frame(6, 0));
    }

    #[test]
    fn given_different_phase_when_generating_then_screens_differ() {
        assert_ne!(dummy_frame(5, 0), dummy_frame(5, 1));
    }

    #[test]
    fn given_frame_when_generating_then_it_has_one_pixel_per_screen_position() {
        assert_eq!(dummy_frame(0, 0).len(), 256 * 192);
    }

    #[test]
    fn given_frame_when_generating_then_high_byte_is_zero() {
        assert!(dummy_frame(9, 1).iter().all(|p| p >> 24 == 0));
    }
}
