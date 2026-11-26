// SPDX-License-Identifier: Apache-2.0
//! Tests of `picture`.

use super::*;

#[test]
fn given_the_extreme_15_bit_colours_when_expanded_then_red_is_the_low_bits_and_blue_the_high_bits()
{
    assert_eq!(
        [expand(0), expand(0x7FFF), expand(0x001F), expand(0x7C00)],
        [0x000000, 0xFFFFFF, 0xFF0000, 0x0000FF]
    );
}

#[test]
fn given_a_region_when_painted_then_only_its_64_by_48_pixels_change() {
    let mut grid = vec![0u16; SCREEN_WIDTH * 192];
    paint_region(&mut grid, 1, 0, |_, _| 1);
    let painted = grid.iter().filter(|c| **c == 1).count();
    assert_eq!(
        (
            painted,
            grid[24 * SCREEN_WIDTH + 96],
            grid[24 * SCREEN_WIDTH + 95]
        ),
        (64 * 48, 1, 0)
    );
}

#[test]
fn given_two_pictures_when_compared_then_the_first_differing_pixels_are_listed_with_their_positions()
 {
    let got = vec![expand(5), expand(6), expand(7)];
    assert_eq!(
        differences(&got, &[5, 9, 7]),
        vec![(1, 0, expand(6), expand(9))]
    );
}

#[test]
fn given_palette_entries_below_32_when_coloured_then_every_entry_is_distinct() {
    let mut colours: Vec<u16> = (0..32).map(palette_colour).collect();
    colours.sort_unstable();
    colours.dedup();
    assert_eq!(colours.len(), 32);
}
