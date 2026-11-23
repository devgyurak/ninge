// SPDX-License-Identifier: Apache-2.0
//! The values the BIOS and firmware leave in main RAM after direct boot (`docs/hw/direct-boot.md`,
//! "Boot-time values in main RAM"). Expected values come from the note's table and the synthetic
//! image's own header, never from running the boot code; the user-settings copy is compared with
//! what the ARM7 reads back over the SPI bus (`docs/hw/spi.md`).

use super::*;
use crate::cart::test_image::Image;
use crate::cpu::Bus;

const SPICNT: u32 = 0x0400_01C0;
const SPIDATA: u32 = 0x0400_01C2;
/// Enabled, firmware selected, baud 0.
const FIRMWARE: u16 = 0x8100;
const HOLD: u16 = 0x0800;
/// Master cycles between SPI units: more than the 128 a unit takes at baud 0.
const UNIT: u64 = 200;
const READ: u8 = 0x03;
/// The newest user-settings area: area 1, since the synthetic firmware numbers them 0 and 1.
const NEWEST_AREA: u32 = 0x3_FF00;
const USER_COPY: u32 = 0x027F_FC80;
const USER_COPY_LEN: u32 = 0x70;

fn booted(image: &Image) -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(image.build()).expect("synthetic image");
    machine
}

/// What `Machine::inspect` shows at `address`, `width` bytes (1, 2 or 4), through the ARM9.
fn peek(machine: &Machine, address: u32, width: u32) -> u32 {
    let view = machine.inspect();
    match width {
        1 => u32::from(view.arm9_peek8(address)),
        2 => u32::from(view.arm9_peek16(address)),
        _ => view.arm9_peek32(address),
    }
}

/// The rows `(address, width, expected)` whose memory differs.
fn differing(machine: &Machine, rows: &[(u32, u32, u32)]) -> Vec<(u32, u32, u32)> {
    rows.iter()
        .copied()
        .filter(|&(address, width, expected)| peek(machine, address, width) != expected)
        .collect()
}

/// A halfword of the image at `at`.
fn image_half(rom: &[u8], at: usize) -> u32 {
    u32::from(u16::from_le_bytes([rom[at], rom[at + 1]]))
}

/// The user-settings bytes the ARM7 reads over SPI at `address`: READ, the 24-bit address, then
/// `len` dummy writes, each unit `UNIT` cycles after the last.
fn spi_read(machine: &mut Machine, address: u32, len: u32) -> Vec<u8> {
    let [_, a2, a1, a0] = address.to_be_bytes();
    let dummies = std::iter::repeat_n(0u8, len as usize);
    let bytes: Vec<u8> = [READ, a2, a1, a0].into_iter().chain(dummies).collect();
    let last = bytes.len() - 1;
    let replies: Vec<u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, &byte)| {
            let now = i as u64 * UNIT;
            let hold = [HOLD, 0][usize::from(i == last)];
            let mut bus = machine.bus7(now);
            bus.write16(SPICNT, FIRMWARE | hold);
            bus.write8(SPIDATA, byte);
            machine.bus7(now + UNIT - 1).read8(SPIDATA)
        })
        .collect();
    replies[4..].to_vec()
}

/// Every address `rows` (and the chip IDs) cover, as byte addresses.
fn covered(rows: &[(u32, u32, u32)]) -> Vec<u32> {
    rows.iter()
        .flat_map(|&(address, width, _)| address..address + width)
        .collect()
}

/// The table of `direct-boot.md` for `Image::valid()`: (address, width, expected), the zero rows
/// included. The chip IDs are in `hle::tests`.
fn table(rom: &[u8]) -> Vec<(u32, u32, u32)> {
    let header_crc = image_half(rom, 0x15E);
    let secure_crc = image_half(rom, 0x6C);
    vec![
        (0x027F_F808, 2, header_crc),
        (0x027F_FC08, 2, header_crc),
        (0x027F_F80A, 2, secure_crc),
        (0x027F_FC0A, 2, secure_crc),
        // A valid image: the header CRC is okay, so the missing-or-bad flag is 0.
        (0x027F_F80C, 2, 0),
        (0x027F_FC0C, 2, 0),
        (0x027F_F80E, 2, 0),
        (0x027F_FC0E, 2, 0),
        (0x027F_F810, 2, 0xFFFF),
        (0x027F_F812, 2, 0),
        (0x027F_FC12, 2, 0),
        (0x027F_F814, 2, 0),
        (0x027F_FC14, 2, 0),
        (0x027F_F816, 2, 0),
        (0x027F_FC16, 1, 0),
        (0x027F_F818, 1, 0),
        (0x027F_FC17, 1, 0),
        (0x027F_F850, 2, 0x5835),
        (0x027F_FC10, 2, 0x5835),
        // `Image::valid()` puts the ARM7 at 0238_0000h.
        (0x027F_F860, 4, 0x0238_0000),
        (0x027F_F864, 4, 0),
        (0x027F_F868, 4, 0x3_FE00),
        (0x027F_F86C, 4, 0),
        (0x027F_F870, 4, 0),
        // Our firmware has no part 5, so its CRC field is zero.
        (0x027F_F874, 2, 0),
        (0x027F_F876, 2, 0),
        (0x027F_F880, 4, 7),
        (0x027F_F884, 4, 6),
        (0x027F_F890, 4, 0xB000_2A22),
        (0x027F_FC30, 4, 0),
        (0x027F_FC34, 4, 0),
        (0x027F_FC38, 4, 0),
        (0x027F_FC3C, 4, 0),
        (0x027F_FC40, 2, 1),
    ]
}

#[test]
fn given_a_valid_image_when_booted_then_each_value_of_the_table_is_in_main_ram() {
    let image = Image::valid();
    let machine = booted(&image);
    assert_eq!(
        differing(&machine, &table(&image.build())),
        Vec::<(u32, u32, u32)>::new()
    );
}

#[test]
fn given_a_header_with_a_wrong_crc_when_booted_then_the_bad_flag_is_set_in_both_places() {
    let mut image = Image::valid();
    image.crc = Some(0x1234);
    let machine = booted(&image);
    let rows = [
        (0x027F_F80C, 2, 1),
        (0x027F_FC0C, 2, 1),
        // The stored CRC is copied as it stands, wrong or not.
        (0x027F_F808, 2, 0x1234),
        (0x027F_FC08, 2, 0x1234),
    ];
    assert_eq!(differing(&machine, &rows), Vec::<(u32, u32, u32)>::new());
}

#[test]
fn given_a_booted_machine_when_the_newest_area_is_read_over_spi_then_the_ram_copy_equals_it() {
    let mut machine = booted(&Image::valid());
    let over_spi = spi_read(&mut machine, NEWEST_AREA, USER_COPY_LEN);
    let in_ram: Vec<u8> = (0..USER_COPY_LEN)
        .map(|i| machine.inspect().arm9_peek8(USER_COPY + i))
        .collect();
    assert_eq!(in_ram, over_spi);
}

#[test]
fn given_a_booted_machine_when_the_ram_copy_is_read_then_it_starts_with_the_settings_version_and_the_nickname()
 {
    let machine = booted(&Image::valid());
    // Version 5 (u16), then color, month, day, a zero byte, and the nickname "ninge" in UTF-16.
    let rows = [
        (USER_COPY, 2, 5),
        (USER_COPY + 6, 2, u32::from(b'n')),
        (USER_COPY + 8, 2, u32::from(b'i')),
        (USER_COPY + 0x1A, 2, 5),
    ];
    assert_eq!(differing(&machine, &rows), Vec::<(u32, u32, u32)>::new());
}

#[test]
fn given_a_booted_machine_when_the_ranges_are_scanned_then_every_byte_outside_the_table_is_zero() {
    let image = Image::valid();
    let machine = booted(&image);
    let mut written = covered(&table(&image.build()));
    // The chip IDs, and the 70h user-settings bytes.
    written.extend((0x027F_F800..0x027F_F808).chain(0x027F_FC00..0x027F_FC08));
    written.extend(USER_COPY..USER_COPY + USER_COPY_LEN);
    let ranges = [
        0x027F_F800..0x027F_F894,
        0x027F_FC00..0x027F_FC80,
        USER_COPY + USER_COPY_LEN..0x027F_FD00,
    ];
    let strays: Vec<u32> = ranges
        .into_iter()
        .flatten()
        .filter(|address| !written.contains(address))
        .filter(|&address| machine.inspect().arm9_peek8(address) != 0)
        .collect();
    assert_eq!(strays, Vec::<u32>::new());
}

#[test]
fn given_a_booted_machine_when_the_values_are_read_at_the_main_ram_mirror_then_they_are_the_same() {
    let machine = booted(&Image::valid());
    let rows = [(0x023F_FC40, 2, 1), (0x023F_F880, 4, 7)];
    assert_eq!(differing(&machine, &rows), Vec::<(u32, u32, u32)>::new());
}

#[test]
fn given_the_same_image_when_booted_twice_then_the_states_are_identical() {
    let a = booted(&Image::valid()).save_state();
    let b = booted(&Image::valid()).save_state();
    assert_eq!(a, b);
}
