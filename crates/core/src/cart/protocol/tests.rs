// SPDX-License-Identifier: Apache-2.0
//! What a ROM read returns, from `docs/hw/cart-protocol.md` ("The chip ID", "Commands"). The
//! tables go through harness functions that list the cases that differ.

use super::{byte_at, chip_id, start_of};

const MIB: usize = 1 << 20;

/// (image length, expected chip ID bytes 1 to 4) cases whose chip ID differs.
fn wrong_chip_ids(cases: &[(usize, [u8; 4])]) -> Vec<usize> {
    cases
        .iter()
        .filter(|&&(len, bytes)| chip_id(len).to_le_bytes() != bytes)
        .map(|&(len, _)| len)
        .collect()
}

/// (address, image length, expected start) cases whose start differs.
fn wrong_starts(cases: &[(u32, usize, u32)]) -> Vec<(u32, usize)> {
    cases
        .iter()
        .filter(|&&(address, len, start)| start_of(address, len) != start)
        .map(|&(address, len, _)| (address, len))
        .collect()
}

#[test]
fn given_images_of_known_sizes_when_the_chip_id_is_built_then_the_size_byte_is_mibs_minus_one() {
    // The sizes GBATEK's list of known IDs names: 8, 16, 32, 64, 128 MiB.
    let cases = [
        (8 * MIB, [0xC2, 0x07, 0, 0]),
        (16 * MIB, [0xC2, 0x0F, 0, 0]),
        (32 * MIB, [0xC2, 0x1F, 0, 0]),
        (64 * MIB, [0xC2, 0x3F, 0, 0]),
        (128 * MIB, [0xC2, 0x7F, 0, 0x80]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<usize>::new());
}

#[test]
fn given_images_between_chip_sizes_when_the_chip_id_is_built_then_the_size_rounds_up_to_a_power_of_two()
 {
    let cases = [
        (0, [0xC2, 0x00, 0, 0]),
        (1, [0xC2, 0x00, 0, 0]),
        (MIB, [0xC2, 0x00, 0, 0]),
        (MIB + 1, [0xC2, 0x01, 0, 0]),
        (9 * MIB, [0xC2, 0x0F, 0, 0]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<usize>::new());
}

#[test]
fn given_images_above_64_mib_when_the_chip_id_is_built_then_the_variant_bit_is_set() {
    let cases = [
        (64 * MIB + 1, [0xC2, 0x7F, 0, 0x80]),
        (64 * MIB, [0xC2, 0x3F, 0, 0x00]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<usize>::new());
}

#[test]
fn given_an_image_above_128_mib_when_the_chip_id_is_built_then_the_size_byte_stops_at_7f() {
    assert_eq!(chip_id(256 * MIB).to_le_bytes()[1], 0x7F);
}

#[test]
fn given_the_chip_id_when_read_as_a_word_then_the_first_byte_is_in_bits_0_to_7() {
    assert_eq!(chip_id(16 * MIB), 0x0000_0FC2);
}

#[test]
fn given_addresses_below_8000h_when_a_read_starts_then_they_redirect_to_8000h_plus_the_low_9_bits()
{
    let cases = [
        (0x0000_0000, MIB, 0x8000),
        (0x0000_0123, MIB, 0x8123),
        (0x0000_7FFF, MIB, 0x81FF),
        (0x0000_0200, MIB, 0x8000),
        (0x0000_8000, MIB, 0x8000),
    ];
    assert_eq!(wrong_starts(&cases), Vec::<(u32, usize)>::new());
}

#[test]
fn given_addresses_above_8000h_when_a_read_starts_then_they_are_unchanged() {
    let cases = [
        (0x0000_8001, MIB, 0x8001),
        (0x000F_FFFF, MIB, 0xF_FFFF),
        (0x0001_2345, 2 * MIB, 0x1_2345),
    ];
    assert_eq!(wrong_starts(&cases), Vec::<(u32, usize)>::new());
}

#[test]
fn given_addresses_past_the_chip_when_a_read_starts_then_they_mirror_and_low_results_redirect() {
    // A 1 MiB chip: 0010_0000h mirrors to 0, which redirects; 0010_8004h mirrors to 8004h.
    let cases = [
        (0x0010_0000, MIB, 0x8000),
        (0x0010_8004, MIB, 0x8004),
        (0xFFFF_FFFF, MIB, 0xF_FFFF),
        (0x0020_0010, 2 * MIB, 0x8010),
    ];
    assert_eq!(wrong_starts(&cases), Vec::<(u32, usize)>::new());
}

#[test]
fn given_an_image_when_a_byte_past_its_end_is_read_then_it_is_ff() {
    let rom = [1_u8, 2, 3];
    assert_eq!(
        [byte_at(&rom, 0), byte_at(&rom, 2), byte_at(&rom, 3)],
        [1, 3, 0xFF]
    );
}
