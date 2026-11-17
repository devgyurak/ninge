// SPDX-License-Identifier: Apache-2.0
//! The save chips, from `docs/hw/auxspi-saves.md`. Every chip is synthetic and starts erased
//! (`FFh`); a scenario goes through helpers, and a test body only asserts.

use super::{SaveChip, SaveError, SaveType};

const WREN: u8 = 0x06;
const WRDI: u8 = 0x04;
const RDSR: u8 = 0x05;
const WRSR: u8 = 0x01;
const RDID: u8 = 0x9F;
const READ: u8 = 0x03;
const FAST: u8 = 0x0B;
const WRITE: u8 = 0x02;
const PW: u8 = 0x0A;
const PE: u8 = 0xDB;
const SE: u8 = 0xD8;

/// Sends `bytes` as one session, the chip held selected until the last, and returns the replies.
fn send(chip: &mut SaveChip, bytes: &[u8]) -> Vec<u8> {
    let last = bytes.len() - 1;
    bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| chip.transfer(b, i != last))
        .collect()
}

/// The address bytes of `addr`, `width` of them, most significant first.
fn addr(addr: u32, width: usize) -> Vec<u8> {
    addr.to_be_bytes()[4 - width..].to_vec()
}

/// A command with its address and then `tail`, as one session.
fn command(chip: &mut SaveChip, cmd: u8, address: &[u8], tail: &[u8]) -> Vec<u8> {
    let bytes: Vec<u8> = [&[cmd][..], address, tail].concat();
    let replies = send(chip, &bytes);
    replies[1 + address.len()..].to_vec()
}

fn wren(chip: &mut SaveChip) {
    send(chip, &[WREN]);
}

/// Reads `n` bytes at `at` with the plain read command and an address of `width` bytes.
fn read(chip: &mut SaveChip, at: u32, width: usize, n: usize) -> Vec<u8> {
    command(chip, READ, &addr(at, width), &vec![0; n])
}

/// WREN, then writes `data` at `at` with command `cmd`.
fn write(chip: &mut SaveChip, cmd: u8, at: u32, width: usize, data: &[u8]) {
    wren(chip);
    command(chip, cmd, &addr(at, width), data);
}

fn status(chip: &mut SaveChip) -> u8 {
    send(chip, &[RDSR, 0])[1]
}

fn ids(chip: &mut SaveChip) -> Vec<u8> {
    send(chip, &[RDID, 0, 0, 0, 0])[1..].to_vec()
}

fn blank(kind: SaveType) -> SaveChip {
    SaveChip::blank(kind)
}

/// Every chip with the address width its commands take.
const CHIPS: [(SaveType, usize); 10] = [
    (SaveType::Eeprom512, 1),
    (SaveType::Eeprom8K, 2),
    (SaveType::Eeprom64K, 2),
    (SaveType::Eeprom128K, 3),
    (SaveType::Flash256K, 3),
    (SaveType::Flash512K, 3),
    (SaveType::Flash1M, 3),
    (SaveType::Flash8M, 3),
    (SaveType::Fram8K, 2),
    (SaveType::Fram32K, 2),
];

/// Chips where a write then a read at a few addresses does not give the data back.
fn round_trip_failures() -> Vec<SaveType> {
    CHIPS
        .iter()
        .filter(|&&(kind, width)| {
            let mut chip = blank(kind);
            write(&mut chip, WRITE, 0x12, width, &[0xA5, 0x5A, 0x3C]);
            read(&mut chip, 0x12, width, 3) != [0xA5, 0x5A, 0x3C]
        })
        .map(|&(kind, _)| kind)
        .collect()
}

#[test]
fn given_each_chip_when_bytes_are_written_then_a_read_gives_them_back() {
    assert_eq!(round_trip_failures(), Vec::<SaveType>::new());
}

#[test]
fn given_the_512_byte_eeprom_when_the_upper_half_is_written_then_only_its_read_sees_it() {
    let mut chip = blank(SaveType::Eeprom512);
    write(&mut chip, PW, 0x05, 1, &[0xAA]);
    assert_eq!(
        (
            command(&mut chip, FAST, &[0x05], &[0]),
            read(&mut chip, 0x05, 1, 1)
        ),
        (vec![0xAA], vec![0xFF])
    );
}

#[test]
fn given_a_chip_when_it_is_written_without_write_enable_then_the_data_is_ignored() {
    let mut chip = blank(SaveType::Eeprom8K);
    command(&mut chip, WRITE, &addr(4, 2), &[0x11]);
    assert_eq!(read(&mut chip, 4, 2, 1), vec![0xFF]);
}

#[test]
fn given_write_enable_when_a_write_session_ends_then_the_latch_is_cleared() {
    let mut chip = blank(SaveType::Eeprom8K);
    wren(&mut chip);
    let enabled = status(&mut chip);
    command(&mut chip, WRITE, &addr(0, 2), &[1]);
    assert_eq!((enabled, status(&mut chip)), (0x02, 0x00));
}

#[test]
fn given_write_enable_when_write_disable_is_sent_then_the_latch_is_cleared() {
    let mut chip = blank(SaveType::Flash256K);
    wren(&mut chip);
    send(&mut chip, &[WRDI]);
    assert_eq!(status(&mut chip), 0x00);
}

#[test]
fn given_an_8k_eeprom_when_a_write_runs_past_its_page_then_the_address_wraps_inside_the_page() {
    let mut chip = blank(SaveType::Eeprom8K);
    let data: Vec<u8> = (1..=33).collect();
    write(&mut chip, WRITE, 0, 2, &data);
    assert_eq!(
        (read(&mut chip, 0, 2, 1), read(&mut chip, 31, 2, 2)),
        (vec![33], vec![32, 0xFF])
    );
}

#[test]
fn given_a_fram_when_a_write_runs_past_32_bytes_then_the_next_bytes_follow() {
    let mut chip = blank(SaveType::Fram8K);
    let data: Vec<u8> = (1..=40).collect();
    write(&mut chip, WRITE, 0, 2, &data);
    assert_eq!(read(&mut chip, 30, 2, 4), vec![31, 32, 33, 34]);
}

#[test]
fn given_a_flash_when_pages_are_programmed_then_bits_only_clear_but_a_page_write_overwrites() {
    let mut chip = blank(SaveType::Flash256K);
    write(&mut chip, WRITE, 0, 3, &[0x0F]);
    write(&mut chip, WRITE, 0, 3, &[0xF0]);
    let programmed = read(&mut chip, 0, 3, 1);
    write(&mut chip, PW, 0, 3, &[0xF0]);
    assert_eq!(
        (programmed, read(&mut chip, 0, 3, 1)),
        (vec![0x00], vec![0xF0])
    );
}

#[test]
fn given_a_flash_page_when_a_page_write_sends_fewer_bytes_then_the_others_keep_their_value() {
    let mut chip = blank(SaveType::Flash256K);
    write(&mut chip, PW, 0, 3, &[1, 2, 3, 4]);
    write(&mut chip, PW, 1, 3, &[9]);
    assert_eq!(read(&mut chip, 0, 3, 4), vec![1, 9, 3, 4]);
}

#[test]
fn given_a_flash_when_a_page_is_erased_then_only_that_page_is_ff() {
    let mut chip = blank(SaveType::Flash256K);
    write(&mut chip, PW, 0, 3, &[1]);
    write(&mut chip, PW, 0x100, 3, &[2]);
    write(&mut chip, PW, 0x1FF, 3, &[3]);
    wren(&mut chip);
    send(&mut chip, &[PE, 0, 0x01, 0x80]);
    assert_eq!(
        (
            read(&mut chip, 0, 3, 1),
            read(&mut chip, 0x100, 3, 1),
            read(&mut chip, 0x1FF, 3, 1)
        ),
        (vec![1], vec![0xFF], vec![0xFF])
    );
}

#[test]
fn given_a_flash_when_a_sector_is_erased_then_its_65536_bytes_are_ff() {
    let mut chip = blank(SaveType::Flash512K);
    write(&mut chip, PW, 0xFFFF, 3, &[1]);
    write(&mut chip, PW, 0x1_0000, 3, &[2]);
    write(&mut chip, PW, 0x1_FFFF, 3, &[3]);
    wren(&mut chip);
    send(&mut chip, &[SE, 0x01, 0x23, 0x45]);
    assert_eq!(
        (
            read(&mut chip, 0xFFFF, 3, 1),
            read(&mut chip, 0x1_0000, 3, 1),
            read(&mut chip, 0x1_FFFF, 3, 1)
        ),
        (vec![1], vec![0xFF], vec![0xFF])
    );
}

#[test]
fn given_a_flash_when_an_erase_has_a_partial_address_then_nothing_changes_and_the_latch_stays() {
    let mut chip = blank(SaveType::Flash256K);
    write(&mut chip, PW, 0, 3, &[7]);
    wren(&mut chip);
    send(&mut chip, &[SE, 0x00]);
    assert_eq!(
        (read(&mut chip, 0, 3, 1), status(&mut chip)),
        (vec![7], 0x02)
    );
}

#[test]
fn given_a_flash_when_fast_read_is_used_then_a_dummy_byte_follows_the_address() {
    let mut chip = blank(SaveType::Flash256K);
    write(&mut chip, PW, 8, 3, &[0x42, 0x43]);
    assert_eq!(
        command(&mut chip, FAST, &addr(8, 3), &[0, 0, 0]),
        vec![0xFF, 0x42, 0x43]
    );
}

/// Chips whose RDSR at power-on is not `expected`.
fn status_failures() -> Vec<SaveType> {
    let table = [
        (SaveType::None, 0xFF),
        (SaveType::Eeprom512, 0xF0),
        (SaveType::Eeprom8K, 0x00),
        (SaveType::Eeprom64K, 0x00),
        (SaveType::Eeprom128K, 0x00),
        (SaveType::Fram8K, 0x00),
        (SaveType::Fram32K, 0x00),
        (SaveType::Flash256K, 0x00),
        (SaveType::Flash8M, 0x00),
    ];
    table
        .iter()
        .filter(|&&(kind, expected)| status(&mut blank(kind)) != expected)
        .map(|&(kind, _)| kind)
        .collect()
}

#[test]
fn given_each_chip_at_power_on_when_the_status_is_read_then_it_has_its_fixed_bits() {
    assert_eq!(status_failures(), Vec::<SaveType>::new());
}

#[test]
fn given_the_512_byte_eeprom_when_write_enable_is_set_then_its_fixed_ones_stay() {
    let mut chip = blank(SaveType::Eeprom512);
    wren(&mut chip);
    assert_eq!(status(&mut chip), 0xF2);
}

/// Chips whose RDID is not what the table says.
fn id_failures() -> Vec<SaveType> {
    let none = vec![0xFF; 4];
    let table = [
        (SaveType::Flash256K, vec![0x20, 0x40, 0x12, 0xFF]),
        (SaveType::Flash512K, vec![0x20, 0x40, 0x13, 0xFF]),
        (SaveType::Flash1M, vec![0x20, 0x40, 0x14, 0xFF]),
        (SaveType::Flash8M, vec![0xC2, 0x20, 0x17, 0xFF]),
        (SaveType::Eeprom8K, none.clone()),
        (SaveType::Eeprom512, none.clone()),
        (SaveType::Fram32K, none),
    ];
    table
        .iter()
        .filter(|(kind, expected)| ids(&mut blank(*kind)) != *expected)
        .map(|(kind, _)| *kind)
        .collect()
}

#[test]
fn given_each_chip_when_the_id_is_read_then_flash_answers_its_capacity_and_the_rest_ff() {
    assert_eq!(id_failures(), Vec::<SaveType>::new());
}

#[test]
fn given_an_8k_eeprom_when_an_address_is_past_its_size_then_it_mirrors() {
    let mut chip = blank(SaveType::Eeprom8K);
    write(&mut chip, WRITE, 5, 2, &[0x77]);
    assert_eq!(read(&mut chip, 0x2005, 2, 1), vec![0x77]);
}

#[test]
fn given_a_256k_flash_when_an_address_is_past_its_size_then_it_mirrors() {
    let mut chip = blank(SaveType::Flash256K);
    write(&mut chip, PW, 0x4_0005, 3, &[0x66]);
    assert_eq!(read(&mut chip, 5, 3, 1), vec![0x66]);
}

#[test]
fn given_the_last_byte_of_a_chip_when_a_read_continues_then_it_wraps_to_the_first() {
    let mut chip = blank(SaveType::Eeprom8K);
    write(&mut chip, WRITE, 0, 2, &[0x31]);
    write(&mut chip, WRITE, 0x1FFF, 2, &[0x32]);
    assert_eq!(read(&mut chip, 0x1FFF, 2, 2), vec![0x32, 0x31]);
}

#[test]
fn given_write_protect_all_when_a_write_follows_then_the_memory_is_unchanged() {
    let mut chip = blank(SaveType::Eeprom8K);
    wren(&mut chip);
    send(&mut chip, &[WRSR, 0x0C]);
    write(&mut chip, WRITE, 0, 2, &[0x99]);
    assert_eq!(
        (status(&mut chip), read(&mut chip, 0, 2, 1)),
        (0x0C, vec![0xFF])
    );
}

#[test]
fn given_write_protect_upper_quarter_when_writing_both_sides_then_only_the_upper_is_refused() {
    let mut chip = blank(SaveType::Eeprom8K);
    wren(&mut chip);
    send(&mut chip, &[WRSR, 0x04]);
    write(&mut chip, WRITE, 0x1000, 2, &[1]);
    write(&mut chip, WRITE, 0x1800, 2, &[2]);
    assert_eq!(
        (read(&mut chip, 0x1000, 2, 1), read(&mut chip, 0x1800, 2, 1)),
        (vec![1], vec![0xFF])
    );
}

#[test]
fn given_the_status_write_disable_bit_when_set_by_wrsr_then_the_status_shows_it() {
    let mut chip = blank(SaveType::Fram8K);
    wren(&mut chip);
    send(&mut chip, &[WRSR, 0x80]);
    assert_eq!(status(&mut chip), 0x80);
}

#[test]
fn given_wrsr_without_write_enable_when_sent_then_the_status_is_unchanged() {
    let mut chip = blank(SaveType::Eeprom8K);
    send(&mut chip, &[WRSR, 0x0C]);
    assert_eq!(status(&mut chip), 0x00);
}

#[test]
fn given_a_flash_when_wrsr_is_sent_then_it_is_ignored() {
    let mut chip = blank(SaveType::Flash256K);
    wren(&mut chip);
    send(&mut chip, &[WRSR, 0x0C]);
    assert_eq!(status(&mut chip), 0x02);
}

#[test]
fn given_an_unknown_command_when_bytes_follow_then_the_chip_answers_ff_and_changes_nothing() {
    let mut chip = blank(SaveType::Eeprom8K);
    wren(&mut chip);
    let replies = send(&mut chip, &[0x77, 0x00, 0x00, 0x00, 0x12]);
    assert_eq!(
        (replies, read(&mut chip, 0, 2, 1), chip.generation()),
        (vec![0xFF; 5], vec![0xFF], 0)
    );
}

#[test]
fn given_no_chip_when_it_is_read_then_every_byte_is_ff() {
    let mut chip = SaveChip::default();
    write(&mut chip, WRITE, 0, 2, &[1]);
    assert_eq!(read(&mut chip, 0, 2, 3), vec![0xFF; 3]);
}

#[test]
fn given_hold_set_when_a_byte_is_sent_then_the_chip_stays_selected_until_a_byte_without_it() {
    let mut chip = blank(SaveType::Eeprom8K);
    chip.transfer(READ, true);
    let held = (chip.selected(), chip.command());
    chip.transfer(0, false);
    assert_eq!((held, chip.selected()), ((true, Some(READ)), false));
}

#[test]
fn given_a_write_when_a_byte_changes_then_the_generation_rises_but_a_read_or_same_value_does_not() {
    let mut chip = blank(SaveType::Eeprom8K);
    write(&mut chip, WRITE, 0, 2, &[0x10]);
    let changed = chip.generation();
    read(&mut chip, 0, 2, 4);
    write(&mut chip, WRITE, 0, 2, &[0x10]);
    assert_eq!((changed, chip.generation()), (1, 1));
}

#[test]
fn given_an_image_of_the_wrong_length_when_a_chip_is_made_then_it_is_refused() {
    assert_eq!(
        SaveChip::new(SaveType::Eeprom8K, vec![0; 100]),
        Err(SaveError::BadLength {
            kind: SaveType::Eeprom8K,
            expected: 8192,
            found: 100
        })
    );
}

#[test]
fn given_an_image_of_the_right_length_when_a_chip_is_made_then_the_memory_is_the_image() {
    let chip = SaveChip::new(SaveType::Eeprom512, vec![7; 512]).expect("512 bytes fit");
    assert_eq!(chip.data(), &[7; 512][..]);
}

/// Types whose name does not parse back to themselves.
fn name_failures() -> Vec<SaveType> {
    CHIPS
        .iter()
        .map(|&(kind, _)| kind)
        .chain([SaveType::None])
        .filter(|kind| SaveType::from_name(kind.name()) != Some(*kind))
        .collect()
}

#[test]
fn given_each_type_when_its_name_is_parsed_then_the_type_comes_back() {
    assert_eq!(name_failures(), Vec::<SaveType>::new());
}

/// File sizes that do not map to the type the note names.
fn file_len_failures() -> Vec<usize> {
    let table = [
        (512, Some(SaveType::Eeprom512)),
        (8 * 1024, Some(SaveType::Eeprom8K)),
        (32 * 1024, Some(SaveType::Fram32K)),
        (64 * 1024, Some(SaveType::Eeprom64K)),
        (128 * 1024, Some(SaveType::Eeprom128K)),
        (256 * 1024, Some(SaveType::Flash256K)),
        (512 * 1024, Some(SaveType::Flash512K)),
        (1024 * 1024, Some(SaveType::Flash1M)),
        (8 * 1024 * 1024, Some(SaveType::Flash8M)),
        (0, None),
        (1000, None),
    ];
    table
        .iter()
        .filter(|&&(len, expected)| SaveType::from_file_len(len) != expected)
        .map(|&(len, _)| len)
        .collect()
}

#[test]
fn given_a_file_size_when_the_type_is_looked_up_then_only_a_chip_size_maps_and_8k_is_eeprom() {
    assert_eq!(file_len_failures(), Vec::<usize>::new());
}

#[test]
fn given_an_unknown_name_when_it_is_parsed_then_there_is_no_type() {
    assert_eq!(SaveType::from_name("flash-3m"), None);
}
