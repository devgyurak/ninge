// SPDX-License-Identifier: Apache-2.0
//! The SPI bus and the firmware chip behind it, from `docs/hw/spi.md` and `docs/hw/firmware.md`.
//! The firmware is the synthetic image; scenarios go through helpers and a test body only asserts.

use super::{BUSY, Checked, ENABLE, HOLD, IRQ, SPICNT_MASK, Spi, WIDE};
use crate::cpu::Bus;
use crate::io::device::{Device, Effect, Width, cx_at};
use crate::io::irq::{Requests, SPI_IRQ};
use crate::io::map::{Owner, Port, decode};
use crate::machine::Machine;

const WREN: u8 = 0x06;
const RDSR: u8 = 0x05;
const RDID: u8 = 0x9F;
const READ: u8 = 0x03;
const FAST: u8 = 0x0B;
const PP: u8 = 0x02;
const SPICNT: u32 = 0;
const SPIDATA: u32 = 2;
/// SPICNT with the bus enabled and the firmware selected, at baud 0 and in 8-bit mode.
const FIRMWARE: u16 = ENABLE | (1 << 8);
/// Far more master cycles than any unit takes, so no helper write finds the bus busy.
const GAP: u64 = 10_000;
/// The first user-settings area and the start of the second (`firmware.md`, "The memory map").
const AREA0: u32 = 0x3_FE00;
const AREA1: u32 = 0x3_FF00;

fn write(spi: &mut Spi, now: u64, offset: u32, width: Width, value: u32) -> Effect {
    Device::<false>::write(spi, offset, width, value, cx_at(now))
}

fn peek(spi: &Spi, now: u64, offset: u32, width: Width) -> u32 {
    Device::<false>::peek(spi, offset, width, cx_at(now))
}

/// Sends `bytes` as one session at `cnt` (hold on for every byte but the last, as software clears
/// it before the last unit), `GAP` apart, and returns the reply of each.
fn session(spi: &mut Spi, cnt: u16, bytes: &[u8]) -> Vec<u8> {
    let last = bytes.len() - 1;
    bytes
        .iter()
        .enumerate()
        .map(|(i, &byte)| {
            // After the previous unit, whenever the session began.
            let now = spi.done_at() + GAP;
            let hold = if i == last { 0 } else { HOLD };
            write(spi, now, SPICNT, Width::Half, u32::from(cnt | hold));
            write(spi, now, SPIDATA, Width::Byte, u32::from(byte));
            peek(spi, now + GAP - 1, SPIDATA, Width::Byte) as u8
        })
        .collect()
}

/// SPICNT's size bit for an 8-bit (`false`) or 16-bit unit.
fn size_bit(wide: bool) -> u16 {
    match wide {
        true => WIDE,
        false => 0,
    }
}

/// Sends each session through the ARM7's bus, hold on for every byte but the last, one unit 200
/// master cycles after the other (a unit takes 128 at baud 0).
fn send_through_the_bus(machine: &mut Machine, sessions: &[&[u8]]) {
    let units = sessions.iter().flat_map(|bytes| {
        let last = bytes.len() - 1;
        bytes
            .iter()
            .enumerate()
            .map(move |(i, &byte)| (byte, [HOLD, 0][usize::from(i == last)]))
    });
    units.enumerate().for_each(|(n, (byte, hold))| {
        let mut bus = machine.bus7(n as u64 * 200);
        bus.write16(0x0400_01C0, FIRMWARE | hold);
        bus.write8(0x0400_01C2, byte);
    });
}

/// A command with a 24-bit address and then `tail`, as one session; the replies after the address.
fn command(spi: &mut Spi, cmd: u8, address: u32, tail: &[u8]) -> Vec<u8> {
    let [_, a2, a1, a0] = address.to_be_bytes();
    let bytes: Vec<u8> = [&[cmd, a2, a1, a0][..], tail].concat();
    session(spi, FIRMWARE, &bytes)[4..].to_vec()
}

#[test]
fn given_the_firmware_selected_when_rdid_is_sent_then_the_chip_answers_20_40_12() {
    let mut spi = Spi::new();
    let replies = session(&mut spi, FIRMWARE, &[RDID, 0, 0, 0]);
    assert_eq!(&replies[1..], &[0x20, 0x40, 0x12]);
}

#[test]
fn given_a_read_across_the_two_areas_when_sent_then_area_0_ends_unused_and_area_1_starts_with_its_version()
 {
    let mut spi = Spi::new();
    let replies = command(&mut spi, READ, AREA1 - 2, &[0, 0, 0, 0]);
    assert_eq!(replies, [0xFF, 0xFF, 0x05, 0x00]);
}

#[test]
fn given_a_fast_read_when_sent_then_one_dummy_byte_follows_the_address_and_the_nickname_is_read() {
    let mut spi = Spi::new();
    // The nickname "ninge" in UTF-16 starts at +06h of the area.
    let replies = command(&mut spi, FAST, AREA0 + 6, &[0, 0, 0, 0, 0, 0]);
    assert_eq!(&replies[1..], &[b'n', 0, b'i', 0, b'n']);
}

#[test]
fn given_a_read_when_sent_then_data_starts_right_after_the_address() {
    let mut spi = Spi::new();
    let replies = command(&mut spi, READ, AREA0 + 6, &[0, 0, 0, 0]);
    assert_eq!(replies, [b'n', 0, b'i', 0]);
}

#[test]
fn given_a_chip_just_powered_when_rdsr_is_read_then_no_latch_is_set_and_after_wren_it_is() {
    let mut spi = Spi::new();
    let before = session(&mut spi, FIRMWARE, &[RDSR, 0])[1];
    session(&mut spi, FIRMWARE, &[WREN]);
    let after = session(&mut spi, FIRMWARE, &[RDSR, 0])[1];
    assert_eq!((before, after), (0x00, 0x02));
}

#[test]
fn given_wren_and_a_page_program_when_read_back_then_the_image_changed_there() {
    let mut spi = Spi::new();
    session(&mut spi, FIRMWARE, &[WREN]);
    session(&mut spi, FIRMWARE, &[PP, 0x01, 0x00, 0x00, 0xA5, 0x5A]);
    let read = command(&mut spi, READ, 0x1_0000, &[0, 0, 0]);
    assert_eq!(
        (read, spi.firmware().data()[0x1_0000..0x1_0003].to_vec()),
        (vec![0xA5, 0x5A, 0xFF], vec![0xA5, 0x5A, 0xFF])
    );
}

#[test]
fn given_a_page_program_with_no_wren_when_read_back_then_the_image_is_unchanged() {
    let mut spi = Spi::new();
    session(&mut spi, FIRMWARE, &[PP, 0x01, 0x00, 0x00, 0xA5]);
    assert_eq!(spi.firmware().data()[0x1_0000], 0xFF);
}

#[test]
fn given_hold_set_when_a_command_byte_is_sent_then_the_chip_stays_selected_and_with_hold_clear_it_is_not()
 {
    let mut held = Spi::new();
    write(
        &mut held,
        0,
        SPICNT,
        Width::Half,
        u32::from(FIRMWARE | HOLD),
    );
    write(&mut held, 0, SPIDATA, Width::Byte, u32::from(RDSR));
    let mut released = Spi::new();
    write(&mut released, 0, SPICNT, Width::Half, u32::from(FIRMWARE));
    write(&mut released, 0, SPIDATA, Width::Byte, u32::from(RDSR));
    assert_eq!(
        (held.firmware().selected(), released.firmware().selected()),
        (true, false)
    );
}

#[test]
fn given_a_held_session_when_the_next_byte_is_sent_then_it_belongs_to_the_same_command() {
    let mut spi = Spi::new();
    // RDSR repeats its status for every following byte while the chip stays selected.
    let replies = session(&mut spi, FIRMWARE, &[WREN]);
    let status = session(&mut spi, FIRMWARE, &[RDSR, 0, 0]);
    assert_eq!((replies.len(), &status[1..]), (1, &[0x02, 0x02][..]));
}

#[test]
fn given_the_bus_disabled_when_a_byte_is_written_then_nothing_happens() {
    let mut spi = Spi::new();
    write(
        &mut spi,
        0,
        SPICNT,
        Width::Half,
        u32::from(FIRMWARE & !ENABLE),
    );
    let effect = write(&mut spi, 0, SPIDATA, Width::Byte, u32::from(RDID));
    assert_eq!(
        (
            effect,
            spi.firmware().selected(),
            spi.busy(1),
            spi.data(),
            spi.done_at()
        ),
        (Effect::default(), false, false, 0, 0)
    );
}

#[test]
fn given_each_baud_rate_and_size_when_a_byte_is_written_then_busy_lasts_the_note_duration() {
    // (baud, 16-bit, master cycles): 8-bit units take 128, 256, 512, 1024; 16-bit twice that.
    let rows: [(u16, bool, u64); 8] = [
        (0, false, 128),
        (1, false, 256),
        (2, false, 512),
        (3, false, 1024),
        (0, true, 256),
        (1, true, 512),
        (2, true, 1024),
        (3, true, 2048),
    ];
    let failures: Vec<_> = rows
        .iter()
        .filter(|&&(baud, wide, cycles)| {
            let mut spi = Spi::new();
            let cnt = FIRMWARE | baud | size_bit(wide);
            write(&mut spi, 1000, SPICNT, Width::Half, u32::from(cnt));
            write(&mut spi, 1000, SPIDATA, Width::Byte, u32::from(RDSR));
            let busy = |now| peek(&spi, now, SPICNT, Width::Half) & u32::from(BUSY) != 0;
            !(busy(1000) && busy(1000 + cycles - 1) && !busy(1000 + cycles))
        })
        .collect();
    assert_eq!(failures, Vec::<&(u16, bool, u64)>::new());
}

#[test]
fn given_a_byte_written_while_busy_when_the_unit_ends_then_the_second_byte_was_dropped() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Half, u32::from(FIRMWARE | HOLD));
    write(&mut spi, 0, SPIDATA, Width::Byte, u32::from(READ));
    // 100 < 128 cycles: still on the wire.
    write(&mut spi, 100, SPIDATA, Width::Byte, 0x00);
    assert_eq!(
        (spi.firmware().command(), spi.firmware().address()),
        (Some(READ), 0)
    );
}

#[test]
fn given_the_wide_mode_when_rdid_is_sent_then_the_second_byte_is_kept_in_spidata() {
    let mut spi = Spi::new();
    // The byte goes out, then a zero; the reply to that zero is the chip's first ID byte.
    write(&mut spi, 0, SPICNT, Width::Half, u32::from(FIRMWARE | WIDE));
    write(&mut spi, 0, SPIDATA, Width::Byte, u32::from(RDID));
    assert_eq!(peek(&spi, GAP, SPIDATA, Width::Half), 0x20);
}

#[test]
fn given_bit_14_set_when_a_byte_is_written_then_the_event_is_due_at_the_end_and_raises_if_bit_23() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Half, u32::from(FIRMWARE | IRQ));
    let effect = write(&mut spi, 1000, SPIDATA, Width::Byte, u32::from(RDSR));
    let due = Device::<false>::slot_due(&spi, 0);
    let fired = Device::<false>::slot_fire(&mut spi, 0, 1128);
    assert_eq!(
        (
            effect.reschedule,
            effect.requests,
            due,
            fired.effect.requests,
            Device::<false>::slot_due(&spi, 0)
        ),
        (
            1,
            Requests::default(),
            Some(1128),
            Requests::for_cpu::<false>(SPI_IRQ),
            None
        )
    );
}

#[test]
fn given_bit_14_clear_when_a_byte_is_written_then_no_event_is_queued_and_nothing_is_raised() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Half, u32::from(FIRMWARE));
    let effect = write(&mut spi, 1000, SPIDATA, Width::Byte, u32::from(RDSR));
    let fired = Device::<false>::slot_fire(&mut spi, 0, 1128);
    assert_eq!(
        (effect, Device::<false>::slot_due(&spi, 0), fired.effect),
        (Effect::default(), None, Effect::default())
    );
}

#[test]
fn given_an_unfired_interrupt_when_the_next_byte_starts_then_the_old_request_is_raised_now() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Half, u32::from(FIRMWARE | IRQ));
    write(&mut spi, 0, SPIDATA, Width::Byte, u32::from(RDSR));
    // The first unit ended at 128, but its event has not fired when the next write comes.
    let effect = write(&mut spi, 500, SPIDATA, Width::Byte, 0);
    assert_eq!(
        (
            effect.requests,
            effect.reschedule,
            Device::<false>::slot_due(&spi, 0)
        ),
        (Requests::for_cpu::<false>(SPI_IRQ), 1, Some(628))
    );
}

#[test]
fn given_a_write_of_all_ones_when_spicnt_is_read_then_only_the_kept_bits_read_back() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Half, 0xFFFF);
    assert_eq!(
        (peek(&spi, 0, SPICNT, Width::Half), SPICNT_MASK),
        (0xCF03, 0xCF03)
    );
}

#[test]
fn given_a_byte_write_to_each_half_of_spicnt_when_read_then_each_keeps_its_bits() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Byte, 0xFF);
    write(&mut spi, 0, SPICNT + 1, Width::Byte, 0xFF);
    assert_eq!(spi.spicnt(), 0xCF03);
}

#[test]
fn given_a_write_of_the_busy_bit_when_read_then_it_reads_clear_while_idle() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Half, u32::from(BUSY));
    assert_eq!(peek(&spi, 0, SPICNT, Width::Half), 0);
}

#[test]
fn given_a_transfer_when_spidata_is_read_then_the_high_byte_reads_zero_and_a_high_byte_write_does_nothing()
 {
    let mut spi = Spi::new();
    session(&mut spi, FIRMWARE, &[RDID, 0]);
    write(&mut spi, 2 * GAP, SPIDATA + 1, Width::Byte, 0xFF);
    assert_eq!(
        (
            peek(&spi, 3 * GAP, SPIDATA, Width::Half),
            spi.firmware().selected()
        ),
        (0x20, false)
    );
}

#[test]
fn given_a_halfword_write_over_spidata_when_made_then_its_low_byte_starts_the_transfer() {
    let mut spi = Spi::new();
    write(&mut spi, 0, SPICNT, Width::Half, u32::from(FIRMWARE | HOLD));
    write(&mut spi, 0, SPIDATA, Width::Half, 0xAB00 | u32::from(READ));
    assert_eq!(spi.firmware().command(), Some(READ));
}

#[test]
fn given_a_word_write_over_both_registers_when_made_then_spicnt_is_set_before_the_byte_is_sent() {
    let mut spi = Spi::new();
    write(
        &mut spi,
        0,
        SPICNT,
        Width::Word,
        u32::from(RDSR) << 16 | u32::from(FIRMWARE | HOLD),
    );
    assert_eq!(
        (spi.spicnt(), spi.firmware().command()),
        (FIRMWARE | HOLD, Some(RDSR))
    );
}

#[test]
fn given_the_other_devices_when_a_byte_is_sent_then_the_line_floats_high_and_the_firmware_is_untouched()
 {
    // Selects 0 (power), 2 (touch) and 3 (reserved).
    let answers: Vec<(u16, u8, bool)> = [0u16, 2, 3]
        .iter()
        .map(|&device| {
            let mut spi = Spi::new();
            let cnt = ENABLE | device << 8;
            write(&mut spi, 0, SPICNT, Width::Half, u32::from(cnt | HOLD));
            write(&mut spi, 0, SPIDATA, Width::Byte, u32::from(RDID));
            (device, spi.data(), spi.firmware().selected())
        })
        .collect();
    assert_eq!(
        answers,
        [(0, 0xFF, false), (2, 0xFF, false), (3, 0xFF, false)]
    );
}

#[test]
fn given_a_fresh_bus_when_inspected_then_it_is_idle_with_the_firmware_chip_fitted() {
    let spi = Spi::new();
    assert_eq!(
        (
            spi.spicnt(),
            spi.device(),
            spi.data(),
            spi.busy(0),
            spi.done_at(),
            spi.firmware().data().len(),
            spi.firmware().command(),
        ),
        (0, 0, 0, false, 0, 0x4_0000, None)
    );
}

#[test]
fn given_a_fresh_bus_when_checked_then_it_is_well_formed() {
    assert!(Spi::new().well_formed(0));
}

#[test]
fn given_state_a_register_cannot_reach_when_checked_then_it_is_not_well_formed() {
    let mut stray_bits = Spi::new();
    stray_bits.spicnt = 0x0010;
    let mut far_future = Spi::new();
    far_future.done_at = 1_000_000;
    let mut orphan_event = Spi::new();
    orphan_event.irq_due = Some(5);
    let mut short_image = Spi::new();
    short_image.firmware =
        crate::cart::save::SaveChip::blank(crate::cart::save::SaveType::Flash512K);
    let verdicts =
        [stray_bits, far_future, orphan_event, short_image].map(|spi| spi.well_formed(0));
    assert_eq!(verdicts, [false; 4]);
}

#[test]
fn given_the_same_input_when_run_twice_then_the_state_is_identical() {
    let run = || {
        let mut spi = Spi::new();
        session(&mut spi, FIRMWARE | IRQ, &[WREN]);
        command(&mut spi, READ, AREA0, &[0, 0]);
        spi
    };
    assert_eq!(run(), run());
}

// -------------------------------------------------------------------- the maps, through a machine

#[test]
fn given_the_two_cpus_when_the_address_is_decoded_then_only_the_arm7_reaches_the_device() {
    assert_eq!(
        (
            decode::<true>(0x0400_01C0),
            decode::<false>(0x0400_01C0),
            decode::<false>(0x0400_01C2)
        ),
        (
            None,
            Some(Owner::Io(Port::Spi, 0)),
            Some(Owner::Io(Port::Spi, 2))
        )
    );
}

#[test]
fn given_the_arm9_when_it_writes_and_reads_the_registers_then_it_reads_zero_and_the_arm7_is_unaffected()
 {
    let mut machine = Machine::new();
    machine.bus9().write16(0x0400_01C0, 0xFFFF);
    let arm9 = machine.bus9().read16(0x0400_01C0);
    let arm7 = machine.bus7(0).read16(0x0400_01C0);
    assert_eq!((arm9, arm7), (0, 0));
}

#[test]
fn given_the_arm7_when_it_writes_spicnt_then_it_reads_back_and_the_arm9_still_reads_zero() {
    let mut machine = Machine::new();
    machine.bus7(0).write16(0x0400_01C0, FIRMWARE);
    let arm7 = machine.bus7(0).read16(0x0400_01C0);
    let arm9 = machine.bus9().read16(0x0400_01C0);
    assert_eq!((arm7, arm9), (FIRMWARE, 0));
}

#[test]
fn given_the_arm7_sends_a_byte_through_the_bus_when_inspected_then_the_view_shows_the_session() {
    let mut machine = Machine::new();
    machine.bus7(0).write16(0x0400_01C0, FIRMWARE | HOLD);
    machine.bus7(0).write8(0x0400_01C2, RDSR);
    let inspect = machine.inspect();
    let spi = inspect.spi();
    assert_eq!(
        (
            spi.spicnt(),
            spi.device(),
            spi.busy(0),
            spi.firmware().command(),
            spi.firmware().status(),
        ),
        (FIRMWARE | HOLD, 1, true, Some(RDSR), 0)
    );
}

#[test]
fn given_a_written_firmware_when_a_rom_is_loaded_then_the_image_is_rebuilt_from_the_settings() {
    let mut machine = Machine::new();
    // WREN, then PP of one byte at 10000h.
    send_through_the_bus(&mut machine, &[&[WREN], &[PP, 0x01, 0x00, 0x00, 0xA5]]);
    let written = machine.inspect().spi().firmware().data()[0x1_0000];
    machine
        .load_rom(crate::cart::test_image::Image::valid().build())
        .expect("synthetic image");
    let rebuilt = machine.inspect().spi().firmware().data().to_vec();
    assert_eq!(
        (
            written,
            rebuilt == crate::spi::build(&crate::spi::FirmwareSettings::default())
        ),
        (0xA5, true)
    );
}
