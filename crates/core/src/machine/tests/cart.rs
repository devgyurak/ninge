// SPDX-License-Identifier: Apache-2.0
//! The cartridge slot reached through the CPUs' buses and the scheduler (`docs/hw/cart-protocol.md`):
//! blocks read from a synthetic image by polling and by DMA (ARM9 start mode 5, ARM7 start mode 2),
//! the time of the first word, and the card state direct boot leaves behind. Expected times come
//! from the note's "Timing"; expected bytes from the image's own pattern.

use super::dma::{DST, ENABLE, WORD, machine, peek_words, run_frames, set7, set9};
use super::*;
use crate::cart::test_image::Image;
use crate::cpu::Bus;
use crate::io::DeviceId;
use crate::scheduler::run_round;

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
const REPEAT: u32 = 1 << 25;
const SOURCE_FIXED: u32 = 2 << 23;
const CARD9: u32 = 5 << 27;
const CARD7: u32 = 2 << 28;

/// A byte of the data area: a pattern a wrong address shows.
fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(131) ^ (i >> 7)) as u8
}

/// A booted machine whose image has a patterned data area from `8000h` to its end (64 KiB).
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

/// The command words of `B7 aaaaaaaa 00 00 00`.
fn command(address: u32) -> (u32, u32) {
    let [a3, a2, a1, a0] = address.to_be_bytes();
    (u32::from_le_bytes([0xB7, a3, a2, a1]), u32::from(a0))
}

/// Writes the command and starts a 200h-byte read as the ARM9.
fn start9(machine: &mut Machine, address: u32) {
    let (low, high) = command(address);
    let mut bus = machine.bus9();
    bus.write32(COMMAND, low);
    bus.write32(COMMAND + 4, high);
    bus.write32(ROMCTRL, START | RESB | SIZE_200H);
}

/// The same as the ARM7, after the ARM9 handed it the slot.
fn start7(machine: &mut Machine, address: u32) {
    machine.bus9().write16(EXMEMCNT, SLOT_TO_ARM7);
    let (low, high) = command(address);
    let now = machine.arm7_clock;
    let mut bus = machine.bus7(now);
    bus.write32(COMMAND, low);
    bus.write32(COMMAND + 4, high);
    bus.write32(ROMCTRL, START | RESB | SIZE_200H);
}

/// ROMCTRL as the CPU that owns the slot reads it.
fn romctrl(machine: &Machine, arm9: bool) -> u32 {
    if arm9 {
        machine.inspect().arm9_peek32(ROMCTRL)
    } else {
        machine.inspect().arm7_peek32(machine.arm7_clock, ROMCTRL)
    }
}

/// Polls the data port as `arm9`: rounds run until a word is ready, then it is read, until the
/// transfer is done. At most `rounds` rounds in all, so a stuck transfer fails instead of hanging.
fn poll(machine: &mut Machine, arm9: bool, rounds: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for _ in 0..rounds {
        let status = romctrl(machine, arm9);
        if status & READY != 0 {
            let word = if arm9 {
                machine.bus9().read32(DATA)
            } else {
                let now = machine.arm7_clock;
                machine.bus7(now).read32(DATA)
            };
            bytes.extend(word.to_le_bytes());
        } else if status & START == 0 {
            break;
        } else {
            run_round(machine);
        }
    }
    bytes
}

/// Rounds until `done` holds, at most `rounds`.
fn run_while(machine: &mut Machine, rounds: u32, done: impl Fn(&Machine) -> bool) {
    for _ in 0..rounds {
        if done(machine) {
            break;
        }
        run_round(machine);
    }
}

/// The pending events of the card slot's device as `(time, arm9, slot)`.
fn card_events(machine: &Machine) -> Vec<(u64, bool, u8)> {
    machine
        .inspect()
        .scheduler()
        .pending()
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Slot {
                device: DeviceId::Cart,
                arm9,
                slot,
            } => Some((event.time, arm9, slot)),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------------- polling

#[test]
fn given_a_booted_image_when_the_arm9_polls_a_block_then_it_reads_the_roms_bytes() {
    let mut machine = machine_with_data();
    start9(&mut machine, 0x9000);
    assert_eq!(poll(&mut machine, true, 20_000), expected(0x9000, 0x200));
}

#[test]
fn given_the_slot_handed_to_the_arm7_when_it_polls_a_block_then_it_reads_the_roms_bytes() {
    let mut machine = machine_with_data();
    start7(&mut machine, 0xA400);
    assert_eq!(poll(&mut machine, false, 20_000), expected(0xA400, 0x200));
}

#[test]
fn given_the_slot_handed_to_the_arm7_when_the_arm9_reads_romctrl_then_it_reads_zero() {
    let mut machine = machine_with_data();
    start7(&mut machine, 0x9000);
    assert_eq!(machine.inspect().arm9_peek32(ROMCTRL), 0);
}

#[test]
fn given_a_started_read_when_nothing_has_run_then_one_card_event_is_queued_for_the_first_word() {
    // 8 command clocks + gap 1 (0) + 4 for the word, 10 master cycles each, from time 0; the event
    // is the ARM9's whichever CPU started it (`scheduler.md`).
    let mut machine = machine_with_data();
    start9(&mut machine, 0x9000);
    assert_eq!(card_events(&machine), vec![(120, true, 0)]);
}

#[test]
fn given_the_arm7_starts_a_read_when_the_event_is_queued_then_it_is_the_same_single_event() {
    let mut machine = machine_with_data();
    start7(&mut machine, 0x9000);
    assert_eq!(card_events(&machine).len(), 1);
}

#[test]
fn given_a_word_taken_when_the_next_is_due_then_one_event_waits_four_card_clocks_later() {
    let mut machine = machine_with_data();
    start9(&mut machine, 0x9000);
    run_while(&mut machine, 20_000, |m| romctrl(m, true) & READY != 0);
    let taken_at = machine.inspect().scheduler().now();
    machine.bus9().read32(DATA);
    assert_eq!(card_events(&machine), vec![(taken_at + 40, true, 0)]);
}

#[test]
fn given_a_started_read_when_the_first_word_time_has_passed_then_the_word_is_ready() {
    let mut machine = machine_with_data();
    start9(&mut machine, 0x9000);
    run_while(&mut machine, 20_000, |m| {
        m.inspect().scheduler().now() >= 120
    });
    assert_eq!(romctrl(&machine, true) & READY, READY);
}

#[test]
fn given_a_started_read_when_nothing_has_run_then_no_word_is_ready() {
    let mut machine = machine_with_data();
    start9(&mut machine, 0x9000);
    assert_eq!(romctrl(&machine, true) & READY, 0);
}

// ------------------------------------------------------------------------------- DMA

/// The ARM9's channel 0 reading the data port (fixed) into `DST`, one word for each request.
fn arm9_card_channel(machine: &mut Machine, extra: u32, count: u32) {
    set9(
        machine,
        0,
        DATA,
        DST,
        ENABLE | WORD | SOURCE_FIXED | CARD9 | extra | count,
    );
}

#[test]
fn given_an_arm9_repeating_channel_of_one_unit_when_a_block_is_read_then_dma_moves_the_whole_block()
{
    // GBATEK's recipe: fixed source at the data port, length 1, 32-bit, repeat on, start mode 5.
    let mut machine = machine_with_data();
    arm9_card_channel(&mut machine, REPEAT, 1);
    start9(&mut machine, 0x9000);
    run_while(&mut machine, 20_000, |m| romctrl(m, true) & START == 0);
    let want: Vec<u32> = expected(0x9000, 0x200)
        .chunks(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect();
    assert_eq!(peek_words(&machine, DST, 128), want);
}

#[test]
fn given_an_arm9_channel_of_128_units_when_a_block_is_read_then_one_unit_moves_for_each_word() {
    let mut machine = machine_with_data();
    arm9_card_channel(&mut machine, 0, 128);
    start9(&mut machine, 0xA000);
    run_while(&mut machine, 20_000, |m| romctrl(m, true) & START == 0);
    let first = u32::from_le_bytes([
        pattern(0xA000),
        pattern(0xA001),
        pattern(0xA002),
        pattern(0xA003),
    ]);
    let last = u32::from_le_bytes([
        pattern(0xA1FC),
        pattern(0xA1FD),
        pattern(0xA1FE),
        pattern(0xA1FF),
    ]);
    assert_eq!(
        (
            peek_words(&machine, DST, 1)[0],
            peek_words(&machine, DST + 127 * 4, 1)[0],
            machine.inspect().dma9().control(0) >> 31
        ),
        (first, last, 0)
    );
}

#[test]
fn given_an_arm9_channel_of_one_unit_without_repeat_when_a_block_is_read_then_only_the_first_word_moves()
 {
    let mut machine = machine_with_data();
    arm9_card_channel(&mut machine, 0, 1);
    start9(&mut machine, 0x9000);
    run_frames(&mut machine, 1);
    let first = u32::from_le_bytes([
        pattern(0x9000),
        pattern(0x9001),
        pattern(0x9002),
        pattern(0x9003),
    ]);
    assert_eq!(
        (
            peek_words(&machine, DST, 2),
            machine.inspect().dma9().control(0) >> 31
        ),
        (vec![first, 0], 0)
    );
}

#[test]
fn given_an_arm7_repeating_channel_when_the_arm7_reads_a_block_then_dma_moves_the_whole_block() {
    let mut machine = machine_with_data();
    machine.bus9().write16(EXMEMCNT, SLOT_TO_ARM7);
    set7(
        &mut machine,
        0,
        0,
        DATA,
        DST,
        ENABLE | WORD | SOURCE_FIXED | REPEAT | CARD7 | 1,
    );
    start7(&mut machine, 0x9400);
    run_while(&mut machine, 20_000, |m| romctrl(m, false) & START == 0);
    let want: Vec<u32> = expected(0x9400, 0x200)
        .chunks(4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect();
    assert_eq!(peek_words(&machine, DST, 128), want);
}

#[test]
fn given_the_arm7_owns_the_slot_when_an_arm9_card_channel_waits_then_it_never_starts() {
    let mut machine = machine_with_data();
    arm9_card_channel(&mut machine, REPEAT, 1);
    start7(&mut machine, 0x9000);
    run_while(&mut machine, 20_000, |m| {
        m.inspect().scheduler().now() > 5_000
    });
    assert_eq!(peek_words(&machine, DST, 1), vec![0]);
}

#[test]
fn given_a_dma_block_read_with_the_irq_bit_when_it_ends_then_if_bit_19_is_requested_on_the_owner() {
    let mut machine = machine_with_data();
    machine.bus9().write16(AUXSPICNT, 1 << 14);
    arm9_card_channel(&mut machine, REPEAT, 1);
    start9(&mut machine, 0x9000);
    run_while(&mut machine, 20_000, |m| romctrl(m, true) & START == 0);
    assert_eq!(machine.inspect().irq9().if_() & (1 << 19), 1 << 19);
}

// ------------------------------------------------------------------ save, load, determinism

/// A machine part-way through a DMA block read: the first words moved, the rest not.
fn mid_transfer() -> Machine {
    let mut machine = machine_with_data();
    arm9_card_channel(&mut machine, REPEAT, 1);
    start9(&mut machine, 0x9000);
    run_while(&mut machine, 20_000, |m| {
        m.inspect().scheduler().now() >= 1_500
    });
    machine
}

#[test]
fn given_a_transfer_in_flight_when_saved_loaded_and_continued_then_it_matches_a_straight_run() {
    let mut straight = mid_transfer();
    let mut resumed = Machine::new();
    resumed.load_rom(straight.rom().to_vec()).expect("image");
    resumed
        .load_state(&mid_transfer().save_state())
        .expect("the state loads");
    run_while(&mut straight, 20_000, |m| romctrl(m, true) & START == 0);
    run_while(&mut resumed, 20_000, |m| romctrl(m, true) & START == 0);
    run_frames(&mut straight, 1);
    run_frames(&mut resumed, 1);
    assert_eq!(resumed.save_state(), straight.save_state());
}

#[test]
fn given_the_same_image_and_setup_when_a_dma_block_read_runs_twice_then_the_savestates_are_identical()
 {
    let finished = || {
        let mut machine = mid_transfer();
        run_while(&mut machine, 20_000, |m| romctrl(m, true) & START == 0);
        machine.save_state()
    };
    assert_eq!(finished(), finished());
}

#[test]
fn given_a_machine_loaded_from_a_state_when_the_queue_is_rebuilt_then_the_card_event_is_where_it_was()
 {
    let original = mid_transfer();
    let mut loaded = Machine::new();
    loaded.load_rom(original.rom().to_vec()).expect("image");
    loaded.load_state(&original.save_state()).expect("loads");
    assert_eq!(card_events(&loaded), card_events(&original));
}

// -------------------------------------------------------------------------- direct boot

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
    start9(&mut machine, 0x9000);
    let cart = machine.inspect().cart();
    assert_eq!(
        (
            cart.current_command(),
            cart.address(),
            cart.remaining(),
            cart.owner_is_arm9()
        ),
        (0xB7, 0x9000, 0x200, true)
    );
}

#[test]
fn given_a_started_transfer_when_inspected_then_the_cart_shows_when_its_next_word_is_due() {
    let mut machine = machine_with_data();
    start9(&mut machine, 0x9000);
    assert_eq!(machine.inspect().cart().next_word_due(), Some(120));
}

#[test]
fn given_the_default_dma_machine_when_a_frame_runs_with_the_image_untouched_then_no_card_event_exists()
 {
    let mut idle = machine();
    run_frames(&mut idle, 1);
    assert_eq!(card_events(&idle), vec![]);
}
