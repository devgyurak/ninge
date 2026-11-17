// SPDX-License-Identifier: Apache-2.0
//! The cartridge slot reached through the CPUs' buses (`docs/hw/cart-protocol.md`): a block read
//! from a synthetic image, polled by the CPU that owns the slot, and the card state direct boot
//! leaves behind.

use super::*;
use crate::cart::test_image::Image;
use crate::cpu::Bus;

const AUXSPICNT: u32 = 0x0400_01A0;
const ROMCTRL: u32 = 0x0400_01A4;
const COMMAND: u32 = 0x0400_01A8;
const EXMEMCNT: u32 = 0x0400_0204;
const DATA: u32 = 0x0410_0010;

const START: u32 = 1 << 31;
const RESB: u32 = 1 << 29;
const READY: u32 = 1 << 23;
const SIZE_200H: u32 = 1 << 24;
const SLOT_TO_ARM7: u16 = 1 << 11;

/// A byte of the data area: a pattern a wrong address shows.
fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(131) ^ (i >> 7)) as u8
}

/// A valid image with a patterned data area from `8000h` to the end, `len` bytes long.
fn machine_with_data() -> Machine {
    let mut image = Image::valid();
    image.len = Some(0x1_0000);
    let mut rom = image.build();
    (0x8000..rom.len()).for_each(|i| rom[i] = pattern(i));
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("synthetic image");
    machine
}

fn expected(from: usize, len: usize) -> Vec<u8> {
    (from..from + len).map(pattern).collect()
}

/// A 200h-byte block at `address`, polled word by word through the ARM9's bus.
fn poll_arm9(machine: &mut Machine, address: u32) -> Vec<u8> {
    let mut bus = machine.bus9();
    bus.write32(
        COMMAND,
        u32::from_le_bytes([
            0xB7,
            (address >> 24) as u8,
            (address >> 16) as u8,
            (address >> 8) as u8,
        ]),
    );
    bus.write32(COMMAND + 4, address & 0xFF);
    bus.write32(ROMCTRL, START | RESB | SIZE_200H);
    let mut bytes = Vec::new();
    for _ in 0..0x80 {
        if bus.read32(ROMCTRL) & READY == 0 {
            break;
        }
        bytes.extend(bus.read32(DATA).to_le_bytes());
    }
    bytes
}

/// The same through the ARM7's bus, once the slot is handed over.
fn poll_arm7(machine: &mut Machine, address: u32) -> Vec<u8> {
    machine.bus9().write16(EXMEMCNT, SLOT_TO_ARM7);
    let mut bus = machine.bus7(0);
    bus.write32(
        COMMAND,
        u32::from_le_bytes([
            0xB7,
            (address >> 24) as u8,
            (address >> 16) as u8,
            (address >> 8) as u8,
        ]),
    );
    bus.write32(COMMAND + 4, address & 0xFF);
    bus.write32(ROMCTRL, START | RESB | SIZE_200H);
    let mut bytes = Vec::new();
    for _ in 0..0x80 {
        if bus.read32(ROMCTRL) & READY == 0 {
            break;
        }
        bytes.extend(bus.read32(DATA).to_le_bytes());
    }
    bytes
}

#[test]
fn given_a_booted_image_when_the_arm9_polls_a_block_then_it_reads_the_roms_bytes() {
    let mut machine = machine_with_data();
    assert_eq!(poll_arm9(&mut machine, 0x9000), expected(0x9000, 0x200));
}

#[test]
fn given_the_slot_handed_to_the_arm7_when_it_polls_a_block_then_it_reads_the_roms_bytes() {
    let mut machine = machine_with_data();
    assert_eq!(poll_arm7(&mut machine, 0xA400), expected(0xA400, 0x200));
}

#[test]
fn given_the_slot_handed_to_the_arm7_when_the_arm9_reads_romctrl_then_it_reads_zero() {
    let mut machine = machine_with_data();
    poll_arm7(&mut machine, 0x9000);
    assert_eq!(machine.inspect().arm9_peek32(ROMCTRL), 0);
}

#[test]
fn given_a_direct_boot_when_romctrl_and_auxspicnt_are_read_then_resb_is_set_and_nothing_else() {
    let machine = machine_with_data();
    assert_eq!(
        (
            machine.inspect().arm9_peek32(ROMCTRL),
            machine.inspect().arm9_peek16(AUXSPICNT),
            machine.inspect().arm9_peek16(EXMEMCNT)
        ),
        (RESB, 0, 0)
    );
}

#[test]
fn given_a_new_machine_when_the_slot_registers_are_read_then_they_read_zero() {
    let machine = Machine::new();
    assert_eq!(machine.inspect().arm9_peek32(ROMCTRL), 0);
}

#[test]
fn given_a_transfer_started_when_inspected_then_the_cart_shows_command_address_and_remaining() {
    let mut machine = machine_with_data();
    let mut bus = machine.bus9();
    bus.write32(COMMAND, 0x9000_00B7);
    bus.write32(COMMAND + 4, 0);
    bus.write32(ROMCTRL, START | RESB | SIZE_200H);
    let cart = machine.inspect().cart();
    assert_eq!(
        (
            cart.command()[0],
            cart.address(),
            cart.remaining(),
            cart.owner_is_arm9()
        ),
        (0xB7, 0x9000, 0x200, true)
    );
}

#[test]
fn given_a_transfer_in_flight_when_the_machine_is_saved_and_loaded_then_the_rest_matches_a_straight_run()
 {
    let mut machine = machine_with_data();
    {
        let mut bus = machine.bus9();
        bus.write32(COMMAND, 0x9000_00B7);
        bus.write32(COMMAND + 4, 0);
        bus.write32(ROMCTRL, START | RESB | SIZE_200H);
        bus.read32(DATA);
    }
    let state = machine.save_state();
    let mut loaded = Machine::new();
    loaded.load_rom(machine.rom.clone()).expect("image");
    loaded.load_state(&state).expect("loads");
    assert_eq!(rest_of_block(&mut loaded), rest_of_block(&mut machine));
}

/// The words still to be taken through the ARM9's bus.
fn rest_of_block(machine: &mut Machine) -> Vec<u8> {
    let mut bus = machine.bus9();
    let mut bytes = Vec::new();
    for _ in 0..0x80 {
        if bus.read32(ROMCTRL) & READY == 0 {
            break;
        }
        bytes.extend(bus.read32(DATA).to_le_bytes());
    }
    bytes
}
