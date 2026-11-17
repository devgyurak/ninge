// SPDX-License-Identifier: Apache-2.0
//! What a ROM read returns, from `docs/hw/cart-protocol.md` ("The chip ID", "Commands"). The
//! tables go through harness functions that list the cases that differ. Images are zero-filled
//! with the header's device capacity byte (`014h`) set as the case needs.

use super::{byte_at, chip_id, start_of};

const MIB: usize = 1 << 20;
const CAPACITY: usize = 0x14;

/// An image of `len` bytes whose header names a device capacity of `128 KiB << n`.
fn image(len: usize, n: u8) -> Vec<u8> {
    let mut rom = vec![0; len.max(CAPACITY + 1)];
    rom[CAPACITY] = n;
    rom
}

/// (image length, capacity n, expected chip ID bytes 1 to 4) cases whose chip ID differs.
fn wrong_chip_ids(cases: &[(usize, u8, [u8; 4])]) -> Vec<(usize, u8)> {
    cases
        .iter()
        .filter(|&&(len, n, bytes)| chip_id(&image(len, n)).to_le_bytes() != bytes)
        .map(|&(len, n, _)| (len, n))
        .collect()
}

/// (address, image length, capacity n, expected start) cases whose start differs.
fn wrong_starts(cases: &[(u32, usize, u8, u32)]) -> Vec<(u32, usize, u8)> {
    cases
        .iter()
        .filter(|&&(address, len, n, start)| start_of(address, &image(len, n)) != start)
        .map(|&(address, len, n, _)| (address, len, n))
        .collect()
}

#[test]
fn given_headers_naming_a_capacity_when_the_chip_id_is_built_then_the_size_byte_is_mibs_minus_one()
{
    // 128 KiB << n for the sizes GBATEK's list of known IDs names: 8, 16, 32, 64, 128 MiB.
    let cases = [
        (0x200, 6, [0xC2, 0x07, 0, 0]),
        (0x200, 7, [0xC2, 0x0F, 0, 0]),
        (0x200, 8, [0xC2, 0x1F, 0, 0]),
        (0x200, 9, [0xC2, 0x3F, 0, 0]),
        (0x200, 10, [0xC2, 0x7F, 0, 0x80]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<(usize, u8)>::new());
}

#[test]
fn given_a_header_smaller_than_the_image_when_the_chip_id_is_built_then_the_image_length_wins() {
    // An 8 MiB image under a header that names 1 MiB reports 8 MiB, so the mirror cannot hide data.
    let cases = [
        (8 * MIB, 3, [0xC2, 0x07, 0, 0]),
        (9 * MIB, 0, [0xC2, 0x0F, 0, 0]),
        (MIB + 1, 3, [0xC2, 0x01, 0, 0]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<(usize, u8)>::new());
}

#[test]
fn given_small_capacities_and_images_when_the_chip_id_is_built_then_the_size_stops_at_1_mib() {
    let cases = [
        (0, 0, [0xC2, 0x00, 0, 0]),
        (1, 0, [0xC2, 0x00, 0, 0]),
        (MIB, 3, [0xC2, 0x00, 0, 0]),
        (0x200, 5, [0xC2, 0x03, 0, 0]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<(usize, u8)>::new());
}

#[test]
fn given_chips_around_64_mib_when_the_chip_id_is_built_then_the_variant_bit_is_set_only_above() {
    let cases = [
        (0x200, 9, [0xC2, 0x3F, 0, 0x00]),
        (0x200, 10, [0xC2, 0x7F, 0, 0x80]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<(usize, u8)>::new());
}

#[test]
fn given_a_capacity_that_overflows_a_shift_when_the_chip_id_is_built_then_the_size_byte_stops_at_7f()
 {
    let cases = [
        (0x200, 11, [0xC2, 0x7F, 0, 0x80]),
        (0x200, 0xFF, [0xC2, 0x7F, 0, 0x80]),
    ];
    assert_eq!(wrong_chip_ids(&cases), Vec::<(usize, u8)>::new());
}

#[test]
fn given_an_image_shorter_than_the_header_when_the_chip_id_is_built_then_the_capacity_reads_as_0() {
    assert_eq!(chip_id(&[0xFF; 4]), 0x0000_00C2);
}

#[test]
fn given_the_chip_id_when_read_as_a_word_then_the_first_byte_is_in_bits_0_to_7() {
    assert_eq!(chip_id(&image(0x200, 7)), 0x0000_0FC2);
}

#[test]
fn given_addresses_below_8000h_when_a_read_starts_then_they_redirect_to_8000h_plus_the_low_9_bits()
{
    let cases = [
        (0x0000_0000, 0x200, 0, 0x8000),
        (0x0000_0123, 0x200, 0, 0x8123),
        (0x0000_7FFF, 0x200, 0, 0x81FF),
        (0x0000_0200, 0x200, 0, 0x8000),
        (0x0000_8000, 0x200, 0, 0x8000),
    ];
    assert_eq!(wrong_starts(&cases), Vec::<(u32, usize, u8)>::new());
}

#[test]
fn given_addresses_above_8000h_when_a_read_starts_then_they_are_unchanged() {
    let cases = [
        (0x0000_8001, 0x200, 0, 0x8001),
        (0x000F_FFFF, 0x200, 0, 0xF_FFFF),
        (0x0001_2345, 0x200, 4, 0x1_2345),
    ];
    assert_eq!(wrong_starts(&cases), Vec::<(u32, usize, u8)>::new());
}

#[test]
fn given_addresses_past_the_chip_when_a_read_starts_then_they_mirror_and_low_results_redirect() {
    // A 1 MiB chip: 0010_0000h mirrors to 0, which redirects; 0010_8004h mirrors to 8004h.
    let cases = [
        (0x0010_0000, 0x200, 0, 0x8000),
        (0x0010_8004, 0x200, 0, 0x8004),
        (0xFFFF_FFFF, 0x200, 0, 0xF_FFFF),
        (0x0020_0010, 0x200, 4, 0x8010),
    ];
    assert_eq!(wrong_starts(&cases), Vec::<(u32, usize, u8)>::new());
}

#[test]
fn given_a_trimmed_image_when_a_read_starts_past_its_end_then_the_chip_the_header_names_decides_the_mirror()
 {
    // A 4 KiB image under a header naming 2 MiB (n = 4): 0010_9000h is inside the chip, so it is
    // read as is (and returns FFh past the image); the same address on a 1 MiB chip mirrors.
    let cases = [
        (0x0010_9000, 0x1000, 4, 0x10_9000),
        (0x0010_9000, 0x1000, 0, 0x9000),
    ];
    assert_eq!(wrong_starts(&cases), Vec::<(u32, usize, u8)>::new());
}

#[test]
fn given_an_image_when_a_byte_past_its_end_is_read_then_it_is_ff() {
    let rom = [1_u8, 2, 3];
    assert_eq!(
        [byte_at(&rom, 0), byte_at(&rom, 2), byte_at(&rom, 3)],
        [1, 3, 0xFF]
    );
}
