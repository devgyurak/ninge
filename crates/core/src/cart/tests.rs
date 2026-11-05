// SPDX-License-Identifier: Apache-2.0
//! Header parsing and the CRC. Expected values come from `docs/hw/cart-header.md`; the CRC
//! reference is written from the note's algorithm in `test_image`, never taken from `crc16`.
//!
//! Scenarios go through helpers; a test body only asserts.

use super::test_image::{Image, reference_crc16, words};
use super::{Binary, BinarySpec, Field, HEADER_LEN, Header, HeaderError, crc16, parse};
use proptest::prelude::*;

const MAIN_RAM_END: u32 = 0x023B_FE00;
const WRAM_END: u32 = 0x0380_7E00;

/// Parses an image after `change` edited the valid one.
fn parsed(change: impl FnOnce(&mut Image)) -> Result<Header, HeaderError> {
    let mut image = Image::valid();
    change(&mut image);
    parse(&image.build())
}

fn out_of_range(field: Field, value: u32) -> Result<Header, HeaderError> {
    Err(HeaderError::FieldOutOfRange { field, value })
}

/// Whether the parse is accepted (the header itself is not compared).
fn accepted(change: impl FnOnce(&mut Image)) -> bool {
    parsed(change).is_ok()
}

/// Lengths in `range` whose result is not `expected(len)`.
fn wrong_for_lengths(
    range: std::ops::RangeInclusive<usize>,
    expected: impl Fn(usize) -> bool,
) -> Vec<usize> {
    let image = Image::valid().build();
    range
        .filter(|&len| expected(len) != parse(&image[..len]).is_ok())
        .collect()
}

/// Lengths below the header size whose error is not `ImageTooShort { len }`.
fn short_lengths_without_the_right_error() -> Vec<usize> {
    let image = Image::valid().build();
    (0..HEADER_LEN)
        .filter(|&len| parse(&image[..len]) != Err(HeaderError::ImageTooShort { len }))
        .collect()
}

/// Lengths in 0..=0x200 where `parse` panics.
fn lengths_that_panic() -> Vec<usize> {
    let image = Image::valid().build();
    (0..=0x200)
        .filter(|&len| std::panic::catch_unwind(|| parse(&image[..len])).is_err())
        .collect()
}

// ---- CRC ----

#[test]
fn given_ascii_123456789_when_crc16_then_it_is_the_modbus_check_value_4b37() {
    assert_eq!(crc16(b"123456789"), 0x4B37);
}

#[test]
fn given_no_bytes_when_crc16_then_it_is_the_initial_value_ffff() {
    assert_eq!(crc16(&[]), 0xFFFF);
}

#[test]
fn given_one_zero_byte_when_crc16_then_it_is_40bf() {
    assert_eq!(crc16(&[0x00]), 0x40BF);
}

#[test]
fn given_bytes_1_2_3_4_when_crc16_then_it_is_2ba1() {
    assert_eq!(crc16(&[1, 2, 3, 4]), 0x2BA1);
}

proptest! {
    #[test]
    fn given_any_bytes_when_crc16_then_it_equals_the_bitwise_reference(
        data in proptest::collection::vec(any::<u8>(), 0..64)
    ) {
        prop_assert_eq!(crc16(&data), reference_crc16(&data));
    }
}

// ---- Fields ----

#[test]
fn given_valid_image_when_parsing_then_the_arm9_fields_are_read() {
    let header = parsed(|_| {}).expect("valid image");
    assert_eq!(
        header.arm9,
        BinarySpec {
            rom_offset: 0x4000,
            entry: 0x0200_0000,
            ram_address: 0x0200_0000,
            size: 0x100
        }
    );
}

#[test]
fn given_valid_image_when_parsing_then_the_arm7_fields_are_read() {
    let header = parsed(|_| {}).expect("valid image");
    assert_eq!(
        header.arm7,
        BinarySpec {
            rom_offset: 0x8000,
            entry: 0x0238_0000,
            ram_address: 0x0238_0000,
            size: 0x80
        }
    );
}

#[test]
fn given_eight_distinct_field_values_when_parsing_then_each_lands_in_its_own_field() {
    let header = parsed(|image| {
        image.arm9 = BinarySpec {
            rom_offset: 0x6000,
            entry: 0x0201_0040,
            ram_address: 0x0201_0000,
            size: 0x200,
        };
        image.arm7 = BinarySpec {
            rom_offset: 0x9000,
            entry: 0x037F_8100,
            ram_address: 0x037F_8000,
            size: 0x300,
        };
    })
    .expect("valid image");
    assert_eq!(
        (
            header.arm9.entry,
            header.arm9.ram_address,
            header.arm7.entry,
            header.arm7.ram_address
        ),
        (0x0201_0040, 0x0201_0000, 0x037F_8100, 0x037F_8000)
    );
}

#[test]
fn given_a_field_written_as_bytes_00_10_00_02_when_parsing_then_it_reads_little_endian() {
    let mut bytes = Image::valid().build();
    bytes[0x24..0x28].copy_from_slice(&[0x00, 0x10, 0x00, 0x02]);
    assert_eq!(parse(&bytes).expect("still valid").arm9.entry, 0x0200_1000);
}

#[test]
fn given_header_fields_written_with_words_helper_when_parsing_then_the_offsets_are_0x20_to_0x3c() {
    let mut bytes = Image::valid().build();
    let fields = words(&[
        0x4000,
        0x0200_0010,
        0x0200_0020,
        0x40,
        0x8000,
        0x0238_0010,
        0x0238_0020,
        0x20,
    ]);
    bytes[0x20..0x40].copy_from_slice(&fields);
    let header = parse(&bytes).expect("valid");
    assert_eq!(
        (
            header.arm9.entry,
            header.arm9.ram_address,
            header.arm9.size,
            header.arm7.entry,
            header.arm7.ram_address,
            header.arm7.size
        ),
        (
            0x0200_0010,
            0x0200_0020,
            0x40,
            0x0238_0010,
            0x0238_0020,
            0x20
        )
    );
}

#[test]
fn given_valid_image_when_parsing_then_crc_is_the_stored_value_and_crc_ok() {
    let image = Image::valid().build();
    let stored = u16::from_le_bytes([image[0x15E], image[0x15F]]);
    let header = parse(&image).expect("valid image");
    assert_eq!((header.crc, header.crc_ok), (stored, true));
}

#[test]
fn given_stored_crc_off_by_one_when_parsing_then_header_is_accepted_with_crc_ok_false() {
    let right = reference_crc16(&Image::valid().build()[..0x15E]);
    let header = parsed(|image| image.crc = Some(right ^ 1)).expect("a bad CRC still parses");
    assert_eq!((header.crc, header.crc_ok), (right ^ 1, false));
}

#[test]
fn given_a_flipped_byte_at_0x15d_when_parsing_then_crc_ok_is_false() {
    let mut bytes = Image::valid().build();
    bytes[0x15D] ^= 0x01;
    assert_eq!(parse(&bytes).map(|h| h.crc_ok), Ok(false));
}

#[test]
fn given_a_flipped_title_byte_when_parsing_then_crc_ok_is_false() {
    let mut bytes = Image::valid().build();
    bytes[0x00] ^= 0x80;
    assert_eq!(parse(&bytes).map(|h| h.crc_ok), Ok(false));
}

#[test]
fn given_a_flipped_byte_at_0x160_after_the_crc_when_parsing_then_crc_ok_stays_true() {
    let mut bytes = Image::valid().build();
    bytes[0x160] ^= 0xFF;
    assert_eq!(parse(&bytes).map(|h| h.crc_ok), Ok(true));
}

// ---- Image length ----

#[test]
fn given_every_length_below_0x170_when_parsing_then_the_error_is_image_too_short_with_that_length()
{
    assert_eq!(short_lengths_without_the_right_error(), Vec::<usize>::new());
}

#[test]
fn given_every_length_up_to_0x200_when_parsing_then_none_panics() {
    assert_eq!(lengths_that_panic(), Vec::<usize>::new());
}

#[test]
fn given_a_valid_image_cut_between_0x170_and_0x200_when_parsing_then_it_is_refused() {
    // Binaries sit at 0x4000 and up, so a prefix this short cannot hold them.
    assert_eq!(
        wrong_for_lengths(HEADER_LEN..=0x200, |_| false),
        Vec::<usize>::new()
    );
}

#[test]
fn given_an_image_of_exactly_the_binaries_end_when_parsing_then_it_is_accepted() {
    assert!(accepted(|image| image.len = Some(0x8000 + 0x80)));
}

#[test]
fn given_an_image_one_byte_short_of_the_arm7_end_when_parsing_then_past_image_end() {
    let result = parsed(|image| image.len = Some(0x8000 + 0x80 - 1));
    assert_eq!(
        result,
        Err(HeaderError::PastImageEnd {
            binary: Binary::Arm7,
            end: 0x8080,
            len: 0x807F
        })
    );
}

#[test]
fn given_an_image_one_byte_short_of_the_arm9_end_when_parsing_then_past_image_end() {
    // The ARM9 binary ends after the ARM7's, so only it runs past.
    let result = parsed(|image| {
        image.arm9.size = 0x5000;
        image.len = Some(0x8FFF);
    });
    assert_eq!(
        result,
        Err(HeaderError::PastImageEnd {
            binary: Binary::Arm9,
            end: 0x9000,
            len: 0x8FFF
        })
    );
}

#[test]
fn given_arm9_binary_ending_exactly_at_the_image_end_when_parsing_then_it_is_accepted() {
    assert!(accepted(|image| {
        image.arm9.size = 0x5000;
        image.len = Some(0x9000);
    }));
}

#[test]
fn given_arm7_offset_ffffffff_when_parsing_then_the_end_is_widened_and_past_image_end() {
    let result = parsed(|image| {
        image.arm7.rom_offset = 0xFFFF_FFFF;
        image.len = Some(0x4100);
    });
    assert_eq!(
        result,
        Err(HeaderError::PastImageEnd {
            binary: Binary::Arm7,
            end: 0x1_0000_007F,
            len: 0x4100
        })
    );
}

// ---- ARM9 ROM offset ----

#[test]
fn given_arm9_offset_0x3000_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm9.rom_offset = 0x3000),
        out_of_range(Field::Arm9RomOffset, 0x3000)
    );
}

#[test]
fn given_arm9_offset_0x4000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm9.rom_offset = 0x4000));
}

#[test]
fn given_arm9_offset_0x4001_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm9.rom_offset = 0x4001),
        out_of_range(Field::Arm9RomOffset, 0x4001)
    );
}

#[test]
fn given_arm9_offset_0x4800_not_a_multiple_of_0x1000_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm9.rom_offset = 0x4800),
        out_of_range(Field::Arm9RomOffset, 0x4800)
    );
}

#[test]
fn given_arm9_offset_0x5000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm9.rom_offset = 0x5000));
}

#[test]
fn given_arm9_offset_ffffffff_when_parsing_then_field_out_of_range_without_wrapping() {
    assert_eq!(
        parsed(|i| i.arm9.rom_offset = 0xFFFF_FFFF),
        out_of_range(Field::Arm9RomOffset, 0xFFFF_FFFF)
    );
}

// ---- ARM7 ROM offset ----

#[test]
fn given_arm7_offset_0x7fff_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.rom_offset = 0x7FFF),
        out_of_range(Field::Arm7RomOffset, 0x7FFF)
    );
}

#[test]
fn given_arm7_offset_0x8000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.rom_offset = 0x8000));
}

#[test]
fn given_unaligned_arm7_offset_0x8001_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.rom_offset = 0x8001));
}

// ---- ARM9 entry and RAM address ----

#[test]
fn given_arm9_entry_0x01ffffff_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm9.entry = 0x01FF_FFFF),
        out_of_range(Field::Arm9Entry, 0x01FF_FFFF)
    );
}

#[test]
fn given_arm9_entry_0x02000000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm9.entry = 0x0200_0000));
}

#[test]
fn given_arm9_entry_0x023bfe00_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm9.entry = MAIN_RAM_END));
}

#[test]
fn given_arm9_entry_0x023bfe01_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm9.entry = 0x023B_FE01),
        out_of_range(Field::Arm9Entry, 0x023B_FE01)
    );
}

#[test]
fn given_arm9_ram_address_0x01ffffff_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm9.ram_address = 0x01FF_FFFF),
        out_of_range(Field::Arm9RamAddress, 0x01FF_FFFF)
    );
}

#[test]
fn given_arm9_ram_address_0x023bfe01_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm9.ram_address = 0x023B_FE01),
        out_of_range(Field::Arm9RamAddress, 0x023B_FE01)
    );
}

#[test]
fn given_arm9_ram_address_0x023bfe00_when_parsing_then_the_binary_runs_past_the_load_area() {
    let result = parsed(|i| i.arm9.ram_address = MAIN_RAM_END);
    assert_eq!(
        result,
        Err(HeaderError::PastLoadArea {
            binary: Binary::Arm9,
            end: 0x023B_FF00,
            limit: MAIN_RAM_END
        })
    );
}

// ---- ARM9 size and load area ----

#[test]
fn given_arm9_size_zero_when_parsing_then_zero_size() {
    assert_eq!(
        parsed(|i| i.arm9.size = 0),
        Err(HeaderError::ZeroSize {
            binary: Binary::Arm9
        })
    );
}

#[test]
fn given_arm9_size_0x3bfe00_at_ram_0x02000000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm9.size = 0x3B_FE00));
}

#[test]
fn given_arm9_size_0x3bfe01_when_parsing_then_field_out_of_range() {
    // The size limit is a field rule (cart-header.md "Allowed values"); the load-area sum also
    // fails here, and the note names the field, so FieldOutOfRange is expected.
    assert_eq!(
        parsed(|i| i.arm9.size = 0x3B_FE01),
        out_of_range(Field::Arm9Size, 0x3B_FE01)
    );
}

#[test]
fn given_arm9_size_ffffffff_when_parsing_then_field_out_of_range_without_overflow() {
    assert_eq!(
        parsed(|i| i.arm9.size = 0xFFFF_FFFF),
        out_of_range(Field::Arm9Size, 0xFFFF_FFFF)
    );
}

#[test]
fn given_arm9_binary_ending_exactly_at_0x023bfe00_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm9.ram_address = MAIN_RAM_END - 0x100));
}

#[test]
fn given_arm9_binary_ending_one_byte_past_0x023bfe00_when_parsing_then_past_load_area() {
    let result = parsed(|i| i.arm9.ram_address = MAIN_RAM_END - 0xFF);
    assert_eq!(
        result,
        Err(HeaderError::PastLoadArea {
            binary: Binary::Arm9,
            end: 0x023B_FE01,
            limit: MAIN_RAM_END
        })
    );
}

// ---- ARM7 entry and RAM address ----

#[test]
fn given_arm7_entry_0x01ffffff_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.entry = 0x01FF_FFFF),
        out_of_range(Field::Arm7Entry, 0x01FF_FFFF)
    );
}

#[test]
fn given_arm7_entry_0x023bfe01_between_the_ranges_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.entry = 0x023B_FE01),
        out_of_range(Field::Arm7Entry, 0x023B_FE01)
    );
}

#[test]
fn given_arm7_entry_0x037f7fff_just_below_the_wram_range_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.entry = 0x037F_7FFF),
        out_of_range(Field::Arm7Entry, 0x037F_7FFF)
    );
}

#[test]
fn given_arm7_entry_0x037f8000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.entry = 0x037F_8000));
}

#[test]
fn given_arm7_entry_0x03807e00_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.entry = WRAM_END));
}

#[test]
fn given_arm7_entry_0x03807e01_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.entry = 0x0380_7E01),
        out_of_range(Field::Arm7Entry, 0x0380_7E01)
    );
}

#[test]
fn given_arm7_entry_0x023bfe00_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.entry = MAIN_RAM_END));
}

#[test]
fn given_arm7_ram_address_0x01ffffff_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.ram_address = 0x01FF_FFFF),
        out_of_range(Field::Arm7RamAddress, 0x01FF_FFFF)
    );
}

#[test]
fn given_arm7_ram_address_0x023bfe01_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.ram_address = 0x023B_FE01),
        out_of_range(Field::Arm7RamAddress, 0x023B_FE01)
    );
}

#[test]
fn given_arm7_ram_address_0x037f7fff_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.ram_address = 0x037F_7FFF),
        out_of_range(Field::Arm7RamAddress, 0x037F_7FFF)
    );
}

#[test]
fn given_arm7_ram_address_0x03807e01_when_parsing_then_field_out_of_range() {
    assert_eq!(
        parsed(|i| i.arm7.ram_address = 0x0380_7E01),
        out_of_range(Field::Arm7RamAddress, 0x0380_7E01)
    );
}

#[test]
fn given_arm7_ram_address_0x037f8000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.ram_address = 0x037F_8000));
}

#[test]
fn given_arm7_in_the_wram_range_when_parsing_then_the_spec_is_parsed_as_given() {
    let header = parsed(|i| {
        i.arm7.ram_address = 0x037F_8000;
        i.arm7.entry = 0x037F_8000;
    })
    .expect("accepted");
    assert_eq!(
        (header.arm7.ram_address, header.arm7.entry),
        (0x037F_8000, 0x037F_8000)
    );
}

// ---- ARM7 size and load area ----

#[test]
fn given_arm7_size_zero_when_parsing_then_zero_size() {
    assert_eq!(
        parsed(|i| i.arm7.size = 0),
        Err(HeaderError::ZeroSize {
            binary: Binary::Arm7
        })
    );
}

#[test]
fn given_arm7_size_0xfe00_in_the_wram_range_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| {
        i.arm7.ram_address = 0x037F_8000;
        i.arm7.size = 0xFE00;
    }));
}

#[test]
fn given_arm7_size_0xfe01_in_the_wram_range_when_parsing_then_field_out_of_range() {
    // The limit is chosen by the RAM address (cart-header.md "Allowed values").
    let result = parsed(|i| {
        i.arm7.ram_address = 0x037F_8000;
        i.arm7.size = 0xFE01;
    });
    assert_eq!(result, out_of_range(Field::Arm7Size, 0xFE01));
}

#[test]
fn given_arm7_size_0xfe01_in_main_ram_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.size = 0xFE01));
}

#[test]
fn given_arm7_size_0x3bfe00_at_ram_0x02000000_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| {
        i.arm7.ram_address = 0x0200_0000;
        i.arm7.size = 0x3B_FE00;
    }));
}

#[test]
fn given_arm7_size_0x3bfe01_in_main_ram_when_parsing_then_field_out_of_range() {
    let result = parsed(|i| {
        i.arm7.ram_address = 0x0200_0000;
        i.arm7.size = 0x3B_FE01;
    });
    assert_eq!(result, out_of_range(Field::Arm7Size, 0x3B_FE01));
}

#[test]
fn given_arm7_binary_ending_exactly_at_0x03807e00_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.ram_address = WRAM_END - 0x80));
}

#[test]
fn given_arm7_binary_ending_one_byte_past_0x03807e00_when_parsing_then_past_load_area() {
    let result = parsed(|i| i.arm7.ram_address = WRAM_END - 0x7F);
    assert_eq!(
        result,
        Err(HeaderError::PastLoadArea {
            binary: Binary::Arm7,
            end: 0x0380_7E01,
            limit: WRAM_END
        })
    );
}

#[test]
fn given_arm7_binary_ending_one_byte_past_0x023bfe00_when_parsing_then_past_load_area() {
    let result = parsed(|i| i.arm7.ram_address = MAIN_RAM_END - 0x7F);
    assert_eq!(
        result,
        Err(HeaderError::PastLoadArea {
            binary: Binary::Arm7,
            end: 0x023B_FE01,
            limit: MAIN_RAM_END
        })
    );
}

#[test]
fn given_arm7_binary_ending_exactly_at_0x023bfe00_when_parsing_then_it_is_accepted() {
    assert!(accepted(|i| i.arm7.ram_address = MAIN_RAM_END - 0x80));
}
