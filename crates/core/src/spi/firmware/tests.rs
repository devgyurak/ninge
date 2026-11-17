// SPDX-License-Identifier: Apache-2.0
//! The synthetic firmware, from `docs/hw/firmware.md`. Expected values come from the note and
//! GBATEK's layout, never from running the builder; a table goes through a harness that returns
//! the rows that differ.

use proptest::prelude::*;

use super::*;
use crate::cart::{crc16, crc16_update};
use crate::spi::CALIBRATION;

const AREA0: usize = USER_SETTINGS_OFFSET;
const AREA1: usize = USER_SETTINGS_OFFSET + USER_AREA_LEN;

fn image() -> Vec<u8> {
    build(&FirmwareSettings::default())
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// The rows `(offset, expected bytes)` whose bytes in `bytes` differ.
fn differing<'a>(bytes: &[u8], rows: &[(usize, &'a [u8])]) -> Vec<(usize, &'a [u8])> {
    rows.iter()
        .filter(|(at, want)| &bytes[*at..*at + want.len()] != *want)
        .copied()
        .collect()
}

/// The UTF-16 text of an area's field, cut by its stored length.
fn text_of(area: &[u8], at: usize, len_at: usize) -> String {
    let units: Vec<u16> = (0..usize::from(u16_at(area, len_at)))
        .map(|i| u16_at(area, at + i * 2))
        .collect();
    String::from_utf16_lossy(&units)
}

fn area(image: &[u8], index: usize) -> &[u8] {
    &image[USER_SETTINGS_OFFSET + index * USER_AREA_LEN..][..USER_AREA_LEN]
}

fn with_counters(c0: u16, c1: u16) -> Vec<u8> {
    let mut image = image();
    for (at, counter) in [(AREA0, c0), (AREA1, c1)] {
        image[at + AREA_COUNTER..at + AREA_COUNTER + 2].copy_from_slice(&counter.to_le_bytes());
        let crc = crc16(&image[at..at + USER_COPY_LEN]);
        image[at + AREA_CRC..at + AREA_CRC + 2].copy_from_slice(&crc.to_le_bytes());
    }
    image
}

/// Which area `newest_user_settings` returns, by address; `None` when it returns none.
fn newest_index(image: &[u8]) -> Option<usize> {
    newest_user_settings(image)
        .map(|found| usize::from(std::ptr::eq(found.as_ptr(), image[AREA1..].as_ptr())))
}

#[test]
fn given_the_defaults_when_built_then_the_image_is_the_chip_size() {
    assert_eq!(image().len(), 0x4_0000);
}

#[test]
fn given_the_same_settings_when_built_twice_then_the_bytes_are_identical() {
    assert_eq!(image(), image());
}

#[test]
fn given_the_defaults_when_the_header_is_read_then_each_field_has_the_value_of_the_note() {
    let image = image();
    let rows: [(usize, &[u8]); 12] = [
        (0x00, &[0, 0, 0, 0, 0, 0, 0, 0]),
        (0x08, b"NNGE"),
        (0x0C, &[0; 8]),
        (0x14, &[0x00, 0x20]),
        (0x16, &[0, 0]),
        (0x18, &[0xFF; 5]),
        (0x1D, &[0xFF]),
        (0x1E, &[0xFF, 0xFF]),
        (0x20, &[0xC0, 0x7F]),
        (0x22, &[0xFF, 0xFF, 0xFF, 0xFF]),
        (0x26, &[0, 0]),
        (0x28, &[0xFF, 0xFF]),
    ];
    assert_eq!(differing(&image, &rows), Vec::<(usize, &[u8])>::new());
}

#[test]
fn given_the_header_when_the_user_settings_offset_is_read_then_it_points_at_the_first_area() {
    assert_eq!(usize::from(u16_at(&image(), 0x20)) * 8, 0x3_FE00);
}

#[test]
fn given_the_wifi_block_when_its_crc_is_recomputed_from_zero_then_it_matches_the_stored_one() {
    let image = image();
    let length = usize::from(u16_at(&image, 0x2C));
    let crc = image[0x2C..0x2C + length]
        .iter()
        .fold(0, |crc, &b| crc16_update(crc, b));
    assert_eq!((length, u16_at(&image, 0x2A)), (0x138, crc));
}

#[test]
fn given_the_wifi_block_when_its_crc_is_taken_with_the_user_settings_start_then_it_differs() {
    let image = image();
    assert_ne!(crc16(&image[0x2C..0x2C + 0x138]), u16_at(&image, 0x2A));
}

#[test]
fn given_the_wifi_block_when_the_mac_is_read_then_it_is_a_locally_administered_unicast_address() {
    let first = image()[0x36];
    assert_eq!((first & 0x02, first & 0x01), (0x02, 0x00));
}

#[test]
fn given_the_wifi_block_when_its_fields_are_read_then_each_has_the_value_of_the_note() {
    let image = image();
    let rows: [(usize, &[u8]); 8] = [
        (0x2E, &[0, 0]),
        (0x3C, &[0xFE, 0x3F]),
        (0x3E, &[0xFF, 0xFF]),
        (0x40, &[0x02, 0x18, 0x0C, 0x01]),
        (0x162, &[0x19, 0xFF]),
        (0x164, &[0xFF; 4]),
        (0x1FC, &[0xFF; 4]),
        (0x200, &[0xFF; 4]),
    ];
    assert_eq!(differing(&image, &rows), Vec::<(usize, &[u8])>::new());
}

#[test]
fn given_both_areas_when_their_crcs_are_recomputed_then_each_matches() {
    let image = image();
    let stored = [0, 1].map(|i| u16_at(area(&image, i), AREA_CRC));
    let computed = [0, 1].map(|i| crc16(&area(&image, i)[..0x70]));
    assert_eq!(stored, computed);
}

#[test]
fn given_the_defaults_when_the_counters_are_read_then_area_1_is_one_ahead_of_area_0() {
    let image = image();
    let counters = [0, 1].map(|i| u16_at(area(&image, i), AREA_COUNTER));
    assert_eq!(counters, [0, 1]);
}

#[test]
fn given_the_defaults_when_the_newest_area_is_chosen_then_it_is_area_1() {
    assert_eq!(newest_index(&image()), Some(1));
}

#[test]
fn given_counters_when_the_newest_area_is_chosen_then_it_follows_the_one_ahead_rule() {
    // (counter0, counter1, expected newest area)
    let rows = [
        (0u16, 1u16, 1usize),
        (1, 0, 0),
        (5, 6, 1),
        (6, 5, 0),
        (0x7F, 0, 1),
        (0, 0, 0),
        (3, 9, 0),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(c0, c1, want)| newest_index(&with_counters(c0, c1)) != Some(want))
        .collect();
    assert_eq!(failures, Vec::<&(u16, u16, usize)>::new());
}

#[test]
fn given_a_corrupt_area_1_when_the_newest_area_is_chosen_then_area_0_is_used() {
    let mut image = image();
    image[AREA1 + 0x10] ^= 0xFF;
    assert_eq!(newest_index(&image), Some(0));
}

#[test]
fn given_a_corrupt_area_0_when_the_newest_area_is_chosen_then_area_1_is_used() {
    let mut image = image();
    image[AREA0 + 0x10] ^= 0xFF;
    assert_eq!(newest_index(&image), Some(1));
}

#[test]
fn given_two_corrupt_areas_when_the_newest_area_is_chosen_then_there_is_none() {
    let mut image = image();
    image[AREA0 + 0x10] ^= 0xFF;
    image[AREA1 + 0x10] ^= 0xFF;
    assert_eq!(newest_index(&image), None);
}

#[test]
fn given_a_short_image_when_the_newest_area_is_chosen_then_there_is_none() {
    assert_eq!(newest_user_settings(&[0u8; 16]), None);
}

#[test]
fn given_the_defaults_when_the_nickname_is_read_then_it_is_ninge_in_utf16_with_its_length() {
    let image = image();
    let area = area(&image, 0);
    assert_eq!(
        (
            text_of(area, 0x06, 0x1A),
            u16_at(area, 0x1A),
            u16_at(area, 0x06),
            u16_at(area, 0x06 + 10),
        ),
        ("ninge".to_string(), 5, u16::from(b'n'), 0)
    );
}

#[test]
fn given_a_nickname_longer_than_ten_units_when_built_then_it_is_cut_at_ten() {
    let settings = FirmwareSettings {
        nickname: "abcdefghijklmnop".to_string(),
        ..FirmwareSettings::default()
    };
    let image = build(&settings);
    assert_eq!(text_of(area(&image, 0), 0x06, 0x1A), "abcdefghij");
}

#[test]
fn given_a_surrogate_pair_at_the_cut_when_built_then_the_pair_is_not_split() {
    // Nine units, then a character that needs two: only nine fit.
    let settings = FirmwareSettings {
        nickname: format!("{}\u{1F600}", "a".repeat(9)),
        ..FirmwareSettings::default()
    };
    let image = build(&settings);
    assert_eq!(u16_at(area(&image, 0), 0x1A), 9);
}

#[test]
fn given_a_message_when_built_then_it_is_stored_with_its_length_and_cut_at_26() {
    let settings = FirmwareSettings {
        message: "m".repeat(40),
        ..FirmwareSettings::default()
    };
    let image = build(&settings);
    let area = area(&image, 0);
    assert_eq!(
        (u16_at(area, 0x50), text_of(area, 0x1C, 0x50).len()),
        (26, 26)
    );
}

#[test]
fn given_the_defaults_when_the_message_is_read_then_it_is_empty() {
    let image = image();
    let area = area(&image, 0);
    assert_eq!(
        (u16_at(area, 0x50), area[0x1C..0x1C + 52].to_vec()),
        (0, vec![0; 52])
    );
}

#[test]
fn given_each_language_when_built_then_the_low_three_flag_bits_hold_it() {
    let languages = [
        (Language::Japanese, 0),
        (Language::English, 1),
        (Language::French, 2),
        (Language::German, 3),
        (Language::Italian, 4),
        (Language::Spanish, 5),
    ];
    let failures: Vec<_> = languages
        .iter()
        .filter(|&&(language, bits)| {
            let image = build(&FirmwareSettings {
                language,
                ..FirmwareSettings::default()
            });
            u16_at(area(&image, 0), AREA_FLAGS) & 7 != bits
        })
        .collect();
    assert_eq!(failures, Vec::<&(Language, u16)>::new());
}

#[test]
fn given_the_defaults_when_the_flags_are_read_then_settings_are_okay_and_the_boot_menu_is_off() {
    let image = image();
    assert_eq!(u16_at(area(&image, 0), AREA_FLAGS), 0xEC41);
}

#[test]
fn given_the_boot_menu_enabled_when_built_then_bit_6_is_clear() {
    let image = build(&FirmwareSettings {
        boot_menu_disabled: false,
        ..FirmwareSettings::default()
    });
    assert_eq!(u16_at(area(&image, 0), AREA_FLAGS), 0xEC01);
}

#[test]
fn given_the_defaults_when_the_flags_are_read_then_the_lost_bit_is_clear_and_the_okay_bits_are_set()
{
    let flags = u16_at(area(&image(), 0), AREA_FLAGS);
    let okay = [10, 11, 13, 14, 15].map(|bit| flags >> bit & 1);
    assert_eq!((flags >> 9 & 1, okay), (0, [1; 5]));
}

#[test]
fn given_the_defaults_when_the_calibration_is_read_then_it_is_the_constant_in_both_areas() {
    let image = image();
    let [(ax1, ay1), (ax2, ay2)] = CALIBRATION.adc;
    let [(sx1, sy1), (sx2, sy2)] = CALIBRATION.screen;
    let read = |i| {
        let a = area(&image, i);
        (
            [
                u16_at(a, 0x58),
                u16_at(a, 0x5A),
                u16_at(a, 0x5E),
                u16_at(a, 0x60),
            ],
            [a[0x5C], a[0x5D], a[0x62], a[0x63]],
        )
    };
    let want = ([ax1, ay1, ax2, ay2], [sx1, sy1, sx2, sy2]);
    assert_eq!([read(0), read(1)], [want, want]);
}

#[test]
fn given_the_defaults_when_the_fixed_fields_are_read_then_they_have_the_values_of_the_note() {
    let image = image();
    let a = area(&image, 0);
    let rows: [(usize, &[u8]); 10] = [
        (0x00, &[5, 0]),
        (0x02, &[0, 1, 1, 0]),
        (0x52, &[0, 0, 0, 0, 0, 0]),
        (0x66, &[25, 0]),
        (0x68, &[0, 0, 0, 0]),
        (0x6C, &[0xFF; 4]),
        (0x74, &[0xFF; 8]),
        (0xF8, &[0xFF; 8]),
        (0x1A, &[5, 0]),
        (0x50, &[0, 0]),
    ];
    assert_eq!(differing(a, &rows), Vec::<(usize, &[u8])>::new());
}

#[test]
fn given_a_year_outside_the_range_when_built_then_it_saturates() {
    let years = [1999u16, 2000, 2255, 2256, 9999];
    let stored: Vec<u8> = years
        .iter()
        .map(|&year| {
            let image = build(&FirmwareSettings {
                year,
                ..FirmwareSettings::default()
            });
            area(&image, 0)[0x66]
        })
        .collect();
    assert_eq!(stored, [0, 0, 255, 255, 255]);
}

#[test]
fn given_a_favorite_color_above_15_when_built_then_the_low_four_bits_are_stored() {
    let image = build(&FirmwareSettings {
        favorite_color: 0x1F,
        ..FirmwareSettings::default()
    });
    assert_eq!(area(&image, 0)[0x02], 0x0F);
}

#[test]
fn given_the_areas_when_compared_then_only_the_counter_differs_because_the_crc_excludes_it() {
    let image = image();
    let (a0, a1) = (area(&image, 0), area(&image, 1));
    let differing: Vec<usize> = (0..USER_AREA_LEN).filter(|&i| a0[i] != a1[i]).collect();
    assert_eq!(differing, vec![0x70]);
}

#[test]
fn given_the_image_when_the_regions_outside_the_fields_are_read_then_they_are_erased_flash() {
    let image = image();
    let not_erased: Vec<usize> = (0x200..0x3_FE00).filter(|&i| image[i] != 0xFF).collect();
    assert_eq!(not_erased, Vec::<usize>::new());
}

proptest! {
    #[test]
    fn given_any_settings_when_built_then_both_areas_have_a_matching_crc_and_the_newest_is_area_1(
        nickname in ".{0,20}",
        message in ".{0,40}",
        color in any::<u8>(),
        month in any::<u8>(),
        day in any::<u8>(),
        year in any::<u16>(),
        boot in any::<bool>(),
        language in 0usize..6,
    ) {
        let languages = [
            Language::Japanese,
            Language::English,
            Language::French,
            Language::German,
            Language::Italian,
            Language::Spanish,
        ];
        let image = build(&FirmwareSettings {
            nickname,
            message,
            language: languages[language],
            favorite_color: color,
            birthday_month: month,
            birthday_day: day,
            year,
            boot_menu_disabled: boot,
        });
        let crcs = [0, 1].map(|i| (crc16(&area(&image, i)[..0x70]), u16_at(area(&image, i), AREA_CRC)));
        prop_assert_eq!(crcs[0].0, crcs[0].1);
        prop_assert_eq!(crcs[1].0, crcs[1].1);
        prop_assert_eq!(newest_index(&image), Some(1));
        prop_assert_eq!(image.len(), FIRMWARE_LEN);
    }
}
