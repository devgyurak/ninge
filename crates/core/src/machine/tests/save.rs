// SPDX-License-Identifier: Apache-2.0
//! The save chip reached through the ARM9's bus, and the host API around it
//! (`docs/hw/auxspi-saves.md`): a session over AUXSPIDATA, the save memory in a savestate, and the
//! determinism of the memory. Bytes go in as a CPU would send them, a byte time apart.

use super::dma::machine;
use super::*;
use crate::cart::{SaveError, SaveType};
use crate::cpu::Bus;

const AUXSPICNT: u32 = 0x0400_01A0;
const AUXSPIDATA: u32 = 0x0400_01A2;
const EXMEMCNT: u32 = 0x0400_0204;
/// Slot enable and SPI mode.
const SPI_ON: u16 = 0xA000;
const HOLD: u16 = 1 << 6;
const BUSY: u16 = 1 << 7;
const SLOT_TO_ARM7: u16 = 1 << 11;
/// Master cycles that pass a byte at the slowest baud, so a byte is always done.
const BYTE_TIME: u32 = 1024;

const WREN: u8 = 0x06;
const RDSR: u8 = 0x05;
const READ: u8 = 0x03;
const WRITE: u8 = 0x02;

/// A booted machine with a blank 8 KiB EEPROM fitted and the slot switched to SPI.
fn with_eeprom() -> Machine {
    let mut m = machine();
    m.insert_save(SaveType::Eeprom8K, vec![0xFF; 8192])
        .expect("8 KiB fits");
    m.bus9().write16(AUXSPICNT, SPI_ON);
    m
}

/// Lets a byte time pass on the ARM9's clock.
fn wait_byte_time(m: &mut Machine) {
    (0..BYTE_TIME).for_each(|_| m.step_arm9());
}

/// One SPI byte as the ARM9 sends it: written with hold as asked, and read once it is done.
fn byte(m: &mut Machine, value: u8, hold: bool) -> u8 {
    m.bus9()
        .write16(AUXSPICNT, SPI_ON | if hold { HOLD } else { 0 });
    m.bus9().write16(AUXSPIDATA, u16::from(value));
    wait_byte_time(m);
    m.bus9().read16(AUXSPIDATA) as u8
}

/// A whole session of `bytes`, hold on until the last; the replies.
fn session(m: &mut Machine, bytes: &[u8]) -> Vec<u8> {
    let last = bytes.len() - 1;
    bytes
        .iter()
        .enumerate()
        .map(|(i, &b)| byte(m, b, i != last))
        .collect()
}

/// WREN, then a write of `data` at `at` of the 8 KiB EEPROM.
fn put(m: &mut Machine, at: u16, data: &[u8]) {
    session(m, &[WREN]);
    let [hi, lo] = at.to_be_bytes();
    session(m, &[&[WRITE, hi, lo][..], data].concat());
}

fn get(m: &mut Machine, at: u16, n: usize) -> Vec<u8> {
    let [hi, lo] = at.to_be_bytes();
    session(m, &[&[READ, hi, lo][..], &vec![0; n]].concat())[3..].to_vec()
}

#[test]
fn given_a_booted_machine_when_no_save_is_inserted_then_there_is_no_chip_and_no_data() {
    let m = machine();
    assert_eq!(
        (m.save_type(), m.save_data().len(), m.save_generation()),
        (SaveType::None, 0, 0)
    );
}

#[test]
fn given_an_image_of_the_wrong_length_when_inserted_then_it_is_refused_and_no_chip_is_fitted() {
    let mut m = machine();
    let result = m.insert_save(SaveType::Eeprom8K, vec![0; 10]);
    assert_eq!(
        (result, m.save_type()),
        (
            Err(SaveError::BadLength {
                kind: SaveType::Eeprom8K,
                expected: 8192,
                found: 10
            }),
            SaveType::None
        )
    );
}

#[test]
fn given_an_inserted_image_when_the_save_data_is_read_then_it_is_that_image() {
    let mut m = machine();
    m.insert_save(SaveType::Eeprom512, vec![0x5A; 512])
        .expect("512 bytes fit");
    assert_eq!(
        (m.save_type(), m.save_data()),
        (SaveType::Eeprom512, &[0x5A; 512][..])
    );
}

#[test]
fn given_a_chip_when_the_arm9_writes_a_page_then_a_read_returns_it_and_the_memory_changed() {
    let mut m = with_eeprom();
    put(&mut m, 0x20, &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(
        (
            get(&mut m, 0x20, 4),
            m.save_data()[0x20..0x24].to_vec(),
            m.save_generation()
        ),
        (
            vec![0xDE, 0xAD, 0xBE, 0xEF],
            vec![0xDE, 0xAD, 0xBE, 0xEF],
            4
        )
    );
}

#[test]
fn given_a_byte_just_written_when_auxspicnt_is_read_then_busy_is_set_until_the_byte_time_passes() {
    let mut m = with_eeprom();
    m.bus9().write16(AUXSPIDATA, u16::from(RDSR));
    let during = m.bus9().read16(AUXSPICNT) & BUSY;
    wait_byte_time(&mut m);
    let after = m.bus9().read16(AUXSPICNT) & BUSY;
    assert_eq!((during, after), (BUSY, 0));
}

#[test]
fn given_a_written_save_when_the_state_is_loaded_onto_a_fresh_machine_then_the_save_is_restored() {
    let mut a = with_eeprom();
    put(&mut a, 0, &[1, 2, 3]);
    let state = a.save_state();
    let mut b = machine();
    b.load_state(&state).expect("same ROM");
    assert_eq!(
        (b.save_type(), b.save_data().to_vec()),
        (SaveType::Eeprom8K, a.save_data().to_vec())
    );
}

#[test]
fn given_a_state_load_when_the_host_reads_the_generation_then_it_is_higher_than_before() {
    let a = with_eeprom();
    let state = a.save_state();
    let mut b = machine();
    let before = b.save_generation();
    b.load_state(&state).expect("same ROM");
    assert_eq!(b.save_generation(), before + 1);
}

#[test]
fn given_a_saved_state_when_loaded_and_continued_then_it_matches_an_uninterrupted_run() {
    let mut run = with_eeprom();
    put(&mut run, 0x40, &[9, 8, 7]);
    let state = run.save_state();
    let mut resumed = machine();
    resumed.load_state(&state).expect("same ROM");
    put(&mut run, 0x50, &[6, 5]);
    put(&mut resumed, 0x50, &[6, 5]);
    assert_eq!(
        (resumed.save_data().to_vec(), resumed.save_state()),
        (run.save_data().to_vec(), run.save_state())
    );
}

#[test]
fn given_the_same_input_twice_when_run_then_the_save_bytes_and_the_state_are_identical() {
    let mut a = with_eeprom();
    let mut b = with_eeprom();
    put(&mut a, 0x10, &[0xAB; 9]);
    put(&mut b, 0x10, &[0xAB; 9]);
    assert_eq!(
        (a.save_data().to_vec(), a.save_state()),
        (b.save_data().to_vec(), b.save_state())
    );
}

#[test]
fn given_a_fitted_chip_when_a_rom_is_loaded_again_then_the_machine_is_reset_to_no_chip() {
    let mut m = with_eeprom();
    let rom = m.rom().to_vec();
    m.load_rom(rom).expect("same ROM");
    assert_eq!(m.save_type(), SaveType::None);
}

#[test]
fn given_the_arm7_owns_the_slot_when_the_arm9_sends_a_byte_then_the_chip_is_not_selected() {
    let mut m = with_eeprom();
    m.bus9().write16(EXMEMCNT, SLOT_TO_ARM7);
    byte(&mut m, READ, true);
    assert!(!m.io.cart().save().selected());
}

#[test]
fn given_a_byte_on_the_wire_at_the_slowest_baud_when_the_state_is_saved_then_it_loads() {
    let mut m = with_eeprom();
    m.bus9().write16(AUXSPICNT, SPI_ON | 3);
    m.bus9().write16(AUXSPIDATA, u16::from(RDSR));
    let state = m.save_state();
    let mut loaded = machine();
    assert_eq!(loaded.load_state(&state), Ok(()));
}

#[test]
fn given_the_arm7_owns_the_slot_when_a_byte_is_on_the_wire_then_the_state_saved_then_loads() {
    let mut m = with_eeprom();
    m.bus9().write16(EXMEMCNT, SLOT_TO_ARM7);
    let at = m.inspect().arm7_clock();
    m.bus7(at).write16(AUXSPICNT, SPI_ON | 3);
    m.bus7(at).write16(AUXSPIDATA, u16::from(RDSR));
    let state = m.save_state();
    let mut loaded = machine();
    // The ARM7's byte really is on the wire (its busy bit is set), and the state still loads.
    assert_eq!(
        (loaded.load_state(&state), m.io.cart().spi_busy(at + 1)),
        (Ok(()), true)
    );
}
