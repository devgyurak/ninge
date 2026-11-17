// SPDX-License-Identifier: Apache-2.0
//! The cartridge slot's registers and the ROM read, from `docs/hw/cart-protocol.md` ("Registers",
//! "Commands", "Ownership", "Behavior"). The ROM is a synthetic image with a known pattern; the
//! scenarios go through helpers and a test body only asserts.

use super::{
    AUXSPICNT_IRQ, COMMAND_OFFSET, Cart, DATA_OFFSET, EXMEMCNT_OFFSET, ROMCTRL_BUSY,
    ROMCTRL_OFFSET, ROMCTRL_READY, ROMCTRL_RESB, block_bytes,
};
use crate::cart::protocol::chip_id;
use crate::io::device::{Checked, Device, Effect, Fired, Width, cx_with_rom};
use crate::io::dma::Trigger;
use crate::io::irq::{CARD_TRANSFER_IRQ, Requests};

const RESB: u32 = ROMCTRL_RESB;
const START: u32 = ROMCTRL_BUSY;
const SIZE_SHIFT: u32 = 24;
/// Bytes in the synthetic image: past the readable start at `8000h`, short of the 1 MiB chip.
const ROM_LEN: usize = 0x2_0000;
const SLOT_TO_ARM7: u32 = 1 << 11;

/// A byte of the image: not periodic over a block, so a wrong address shows.
fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 8) ^ (i >> 13)) as u8
}

/// The image: the pattern, with the header's device capacity (`014h`) at 0 so the chip is 1 MiB.
fn rom() -> Vec<u8> {
    let mut rom: Vec<u8> = (0..ROM_LEN).map(pattern).collect();
    rom[0x14] = 0;
    rom
}

fn write<const ARM9: bool>(cart: &mut Cart, offset: u32, width: Width, value: u32) -> Effect {
    Device::<ARM9>::write(cart, offset, width, value, cx_with_rom(0, &rom()))
}

fn write9(cart: &mut Cart, offset: u32, value: u32) -> Effect {
    write::<true>(cart, offset, Width::Word, value)
}

fn read<const ARM9: bool>(cart: &mut Cart, offset: u32, width: Width) -> (u32, Effect) {
    Device::<ARM9>::read(cart, offset, width, cx_with_rom(0, &rom()))
}

fn peek<const ARM9: bool>(cart: &Cart, offset: u32, width: Width) -> u32 {
    Device::<ARM9>::peek(cart, offset, width, cx_with_rom(0, &rom()))
}

fn romctrl<const ARM9: bool>(cart: &Cart) -> u32 {
    peek::<ARM9>(cart, ROMCTRL_OFFSET, Width::Word)
}

/// Writes `B7 aaaaaaaa 00 00 00` into the command registers as two words, and starts a transfer
/// of block size `size` as CPU `ARM9`.
fn start_read<const ARM9: bool>(cart: &mut Cart, address: u32, size: u32) -> Effect {
    let [a3, a2, a1, a0] = address.to_be_bytes();
    write::<ARM9>(
        cart,
        COMMAND_OFFSET,
        Width::Word,
        u32::from_le_bytes([0xB7, a3, a2, a1]),
    );
    write::<ARM9>(cart, COMMAND_OFFSET + 4, Width::Word, u32::from(a0));
    write::<ARM9>(
        cart,
        ROMCTRL_OFFSET,
        Width::Word,
        START | RESB | (size << SIZE_SHIFT),
    )
}

/// Starts an arbitrary first command byte with block size `size`, as the ARM9.
fn start_command(cart: &mut Cart, first: u8, size: u32) {
    write9(cart, COMMAND_OFFSET, u32::from(first));
    write9(cart, COMMAND_OFFSET + 4, 0);
    write9(cart, ROMCTRL_OFFSET, START | RESB | (size << SIZE_SHIFT));
}

/// Takes words from the data port as CPU `ARM9` until the transfer ends; the bytes, and the
/// effect of the last read. At most `limit` words, so a stuck transfer fails instead of hanging.
fn drain<const ARM9: bool>(cart: &mut Cart, limit: usize) -> (Vec<u8>, Effect) {
    let mut bytes = Vec::new();
    let mut effect = Effect::default();
    for _ in 0..limit {
        if romctrl::<ARM9>(cart) & ROMCTRL_READY == 0 {
            // The word's event: fire it at its time, if one is on its way.
            let Some(due) = cart.next_word_due() else {
                break;
            };
            let fired = Device::<ARM9>::slot_fire(cart, 0, due);
            effect = fired.effect;
            if romctrl::<ARM9>(cart) & ROMCTRL_READY == 0 {
                break;
            }
        }
        let (word, last) = read::<ARM9>(cart, DATA_OFFSET, Width::Word);
        bytes.extend(word.to_le_bytes());
        effect = last;
    }
    (bytes, effect)
}

/// Fires the device's event at its time, as the scheduler does; the fired result.
fn fire(cart: &mut Cart) -> Fired {
    let due = cart.next_word_due().expect("an event is on its way");
    Device::<true>::slot_fire(cart, 0, due)
}

/// A started read of `size` as the ARM9 with its first word ready.
fn ready_read(address: u32, size: u32) -> Cart {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, address, size);
    fire(&mut cart);
    cart
}

fn drain9(cart: &mut Cart) -> Vec<u8> {
    drain::<true>(cart, 0x2000).0
}

/// A cart whose AUXSPICNT has the transfer-ready IRQ on, owned by the ARM9.
fn with_irq() -> Cart {
    let mut cart = Cart::new();
    write9(&mut cart, 0, u32::from(AUXSPICNT_IRQ));
    cart
}

/// The block sizes 1-7 whose transfer did not hand out `block_bytes(size)` bytes.
fn sizes_with_wrong_length() -> Vec<u32> {
    (1..=7)
        .filter(|&size| {
            let mut cart = Cart::new();
            start_read::<true>(&mut cart, 0x8000, size);
            drain9(&mut cart).len() as u32 != block_bytes(size)
        })
        .collect()
}

#[test]
fn given_each_block_size_when_a_read_is_drained_then_the_bytes_match_the_size() {
    assert_eq!(sizes_with_wrong_length(), Vec::<u32>::new());
}

#[test]
fn given_a_read_of_an_aligned_block_when_drained_then_the_bytes_are_the_roms() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x9200, 1);
    assert_eq!(drain9(&mut cart), rom()[0x9200..0x9400]);
}

#[test]
fn given_an_unaligned_address_when_drained_then_the_bytes_start_at_that_address() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x9203, 7);
    assert_eq!(drain9(&mut cart), rom()[0x9203..0x9207]);
}

#[test]
fn given_an_address_below_8000h_when_drained_then_it_reads_from_8000h_plus_the_low_9_bits() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x0000_0123, 7);
    assert_eq!(drain9(&mut cart), rom()[0x8123..0x8127]);
}

#[test]
fn given_a_read_that_crosses_a_4k_boundary_when_drained_then_it_wraps_inside_the_block() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x9FF8, 1);
    let mut want = rom()[0x9FF8..0xA000].to_vec();
    want.extend_from_slice(&rom()[0x9000..0x9000 + 0x200 - 8]);
    assert_eq!(drain9(&mut cart), want);
}

#[test]
fn given_an_address_past_the_image_when_drained_then_the_bytes_are_ff() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, ROM_LEN as u32, 7);
    assert_eq!(drain9(&mut cart), vec![0xFF; 4]);
}

#[test]
fn given_an_address_past_the_chip_when_drained_then_it_mirrors_into_the_chip() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x0010_9000, 7);
    assert_eq!(drain9(&mut cart), rom()[0x9000..0x9004]);
}

#[test]
fn given_the_chip_id_command_when_a_word_is_read_then_it_is_the_id_of_the_image_size() {
    let mut cart = Cart::new();
    start_command(&mut cart, 0xB8, 7);
    assert_eq!(drain9(&mut cart), chip_id(&rom()).to_le_bytes());
}

#[test]
fn given_the_chip_id_command_with_a_long_block_when_drained_then_the_id_repeats() {
    let mut cart = Cart::new();
    start_command(&mut cart, 0xB8, 1);
    let id = chip_id(&rom()).to_le_bytes();
    assert_eq!(drain9(&mut cart), id.repeat(0x200 / 4));
}

#[test]
fn given_an_unknown_command_when_drained_then_every_word_is_ffff_ffff() {
    let mut cart = Cart::new();
    start_command(&mut cart, 0x9F, 7);
    assert_eq!(drain9(&mut cart), vec![0xFF; 4]);
}

#[test]
fn given_a_started_read_when_romctrl_is_read_then_busy_is_set_and_no_word_is_ready() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    assert_eq!(
        romctrl::<true>(&cart) & (ROMCTRL_BUSY | ROMCTRL_READY),
        ROMCTRL_BUSY
    );
}

#[test]
fn given_a_started_read_when_its_event_fires_then_busy_and_ready_are_set() {
    let cart = ready_read(0x8000, 1);
    assert_eq!(
        romctrl::<true>(&cart) & (ROMCTRL_BUSY | ROMCTRL_READY),
        ROMCTRL_BUSY | ROMCTRL_READY
    );
}

#[test]
fn given_a_drained_read_when_romctrl_is_read_then_busy_and_ready_are_clear() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    drain9(&mut cart);
    assert_eq!(romctrl::<true>(&cart) & (ROMCTRL_BUSY | ROMCTRL_READY), 0);
}

#[test]
fn given_a_block_size_of_0_when_its_event_fires_then_the_transfer_is_done() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 0);
    fire(&mut cart);
    assert_eq!(romctrl::<true>(&cart) & (ROMCTRL_BUSY | ROMCTRL_READY), 0);
}

#[test]
fn given_a_transfer_when_the_data_port_is_peeked_then_the_word_is_not_taken() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    let first = peek::<true>(&cart, DATA_OFFSET, Width::Word);
    let again = peek::<true>(&cart, DATA_OFFSET, Width::Word);
    assert_eq!((first, again, cart.remaining()), (again, first, 0x200));
}

#[test]
fn given_a_ready_word_when_a_byte_below_the_top_one_is_read_then_the_word_is_not_taken() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    read::<true>(&mut cart, DATA_OFFSET, Width::Byte);
    read::<true>(&mut cart, DATA_OFFSET + 2, Width::Byte);
    assert_eq!(cart.remaining(), 0x200);
}

#[test]
fn given_a_ready_word_when_the_top_byte_is_read_then_the_word_is_taken() {
    let mut cart = ready_read(0x8000, 1);
    read::<true>(&mut cart, DATA_OFFSET + 3, Width::Byte);
    assert_eq!(cart.remaining(), 0x200 - 4);
}

#[test]
fn given_a_finished_transfer_when_the_data_port_is_read_then_it_returns_the_last_word_taken() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 7);
    let (taken, _) = read::<true>(&mut cart, DATA_OFFSET, Width::Word);
    let (again, _) = read::<true>(&mut cart, DATA_OFFSET, Width::Word);
    assert_eq!(again, taken);
}

#[test]
fn given_a_new_cart_when_the_data_port_is_read_then_it_is_zero() {
    let mut cart = Cart::new();
    assert_eq!(read::<true>(&mut cart, DATA_OFFSET, Width::Word).0, 0);
}

#[test]
fn given_the_command_words_when_written_then_the_first_byte_is_at_the_lowest_address() {
    let mut cart = Cart::new();
    write9(&mut cart, COMMAND_OFFSET, 0x0302_01B7);
    write9(&mut cart, COMMAND_OFFSET + 4, 0x0000_0504);
    assert_eq!(cart.command(), [0xB7, 1, 2, 3, 4, 5, 0, 0]);
}

#[test]
fn given_command_bytes_written_singly_when_read_back_then_the_registers_read_zero() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, COMMAND_OFFSET + 2, Width::Byte, 0x7E);
    assert_eq!(
        (
            cart.command()[2],
            peek::<true>(&cart, COMMAND_OFFSET, Width::Word)
        ),
        (0x7E, 0)
    );
}

#[test]
fn given_the_arm9_owns_the_slot_when_the_arm7_writes_romctrl_then_nothing_starts() {
    let mut cart = Cart::new();
    start_read::<false>(&mut cart, 0x8000, 1);
    assert_eq!(romctrl::<true>(&cart), 0);
}

#[test]
fn given_the_arm9_owns_the_slot_when_the_arm7_reads_the_slot_registers_then_they_read_zero() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    let seven = [
        peek::<false>(&cart, ROMCTRL_OFFSET, Width::Word),
        peek::<false>(&cart, DATA_OFFSET, Width::Word),
        peek::<false>(&cart, 0, Width::Half),
    ];
    assert_eq!(seven, [0, 0, 0]);
}

#[test]
fn given_the_arm9_owns_the_slot_when_the_arm7_takes_from_the_data_port_then_the_word_stays() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    read::<false>(&mut cart, DATA_OFFSET, Width::Word);
    assert_eq!(cart.remaining(), 0x200);
}

#[test]
fn given_the_slot_handed_to_the_arm7_when_the_arm7_reads_a_block_then_it_gets_the_bytes() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, EXMEMCNT_OFFSET, Width::Half, SLOT_TO_ARM7);
    start_read::<false>(&mut cart, 0x9000, 1);
    assert_eq!(drain::<false>(&mut cart, 0x2000).0, rom()[0x9000..0x9200]);
}

#[test]
fn given_the_slot_handed_to_the_arm7_when_the_arm9_writes_romctrl_then_nothing_starts() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, EXMEMCNT_OFFSET, Width::Half, SLOT_TO_ARM7);
    start_read::<true>(&mut cart, 0x8000, 1);
    assert_eq!(romctrl::<false>(&cart), 0);
}

#[test]
fn given_an_arm9_write_of_all_ones_when_exmemcnt_is_read_then_bits_8_to_10_and_12_read_zero() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, EXMEMCNT_OFFSET, Width::Half, 0xFFFF);
    assert_eq!(peek::<true>(&cart, EXMEMCNT_OFFSET, Width::Half), 0xE8FF);
}

#[test]
fn given_an_arm9_write_when_the_arm7_reads_exmemstat_then_it_sees_the_shared_bits_and_its_own_low_bits()
 {
    let mut cart = Cart::new();
    write::<true>(&mut cart, EXMEMCNT_OFFSET, Width::Half, 0xFFFF);
    assert_eq!(peek::<false>(&cart, EXMEMCNT_OFFSET, Width::Half), 0xE880);
}

#[test]
fn given_an_arm7_write_when_exmemcnt_is_read_then_only_its_own_low_bits_change() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, EXMEMCNT_OFFSET, Width::Half, 0x0801);
    write::<false>(&mut cart, EXMEMCNT_OFFSET, Width::Half, 0xFFFF);
    let (nine, seven) = (cart.exmemcnt9(), cart.exmemcnt7());
    assert_eq!((nine, seven), (0x0801, 0x087F));
}

#[test]
fn given_a_write_of_all_ones_when_auxspicnt_is_read_then_only_the_kept_bits_read_back() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, 0, Width::Half, 0xFFFF);
    assert_eq!(peek::<true>(&cart, 0, Width::Half), 0xE043);
}

#[test]
fn given_a_byte_write_to_each_half_of_auxspicnt_when_read_then_each_keeps_its_bits() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, 0, Width::Byte, 0xFF);
    write::<true>(&mut cart, 1, Width::Byte, 0xFF);
    assert_eq!(cart.auxspicnt(), 0xE043);
}

#[test]
fn given_the_irq_bit_set_when_a_transfer_ends_then_if_bit_19_is_raised_on_the_arm9() {
    let mut cart = with_irq();
    start_read::<true>(&mut cart, 0x8000, 7);
    let (_, effect) = drain::<true>(&mut cart, 4);
    assert_eq!(
        effect.requests,
        Requests {
            arm9: CARD_TRANSFER_IRQ,
            arm7: 0
        }
    );
}

#[test]
fn given_the_irq_bit_set_and_the_arm7_owning_when_a_transfer_ends_then_if_bit_19_is_raised_on_the_arm7()
 {
    let mut cart = with_irq();
    write::<true>(&mut cart, EXMEMCNT_OFFSET, Width::Half, SLOT_TO_ARM7);
    write::<false>(&mut cart, 0, Width::Half, u32::from(AUXSPICNT_IRQ));
    start_read::<false>(&mut cart, 0x8000, 7);
    let (_, effect) = drain::<false>(&mut cart, 4);
    assert_eq!(
        effect.requests,
        Requests {
            arm9: 0,
            arm7: CARD_TRANSFER_IRQ
        }
    );
}

#[test]
fn given_the_irq_bit_clear_when_a_transfer_ends_then_nothing_is_raised() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 7);
    let (_, effect) = drain::<true>(&mut cart, 4);
    assert_eq!(effect.requests, Requests::default());
}

#[test]
fn given_a_transfer_with_words_left_when_a_word_is_taken_then_no_irq_is_raised() {
    let mut cart = with_irq();
    start_read::<true>(&mut cart, 0x8000, 1);
    let (_, effect) = read::<true>(&mut cart, DATA_OFFSET, Width::Word);
    assert_eq!(effect.requests, Requests::default());
}

#[test]
fn given_the_irq_bit_set_when_a_block_of_size_0_ends_then_its_event_raises_if_bit_19() {
    let mut cart = with_irq();
    start_read::<true>(&mut cart, 0x8000, 0);
    assert_eq!(fire(&mut cart).effect.requests.arm9, CARD_TRANSFER_IRQ);
}

#[test]
fn given_resb_set_when_romctrl_is_written_without_it_then_it_stays_set() {
    let mut cart = Cart::new();
    write9(&mut cart, ROMCTRL_OFFSET, RESB);
    write9(&mut cart, ROMCTRL_OFFSET, 0);
    assert_eq!(romctrl::<true>(&cart), RESB);
}

#[test]
fn given_a_write_of_the_status_and_seed_bits_when_romctrl_is_read_then_they_read_zero() {
    let mut cart = Cart::new();
    write9(&mut cart, ROMCTRL_OFFSET, ROMCTRL_READY | (1 << 15));
    assert_eq!(romctrl::<true>(&cart), 0);
}

#[test]
fn given_a_write_of_the_other_romctrl_fields_when_read_then_they_read_back() {
    let mut cart = Cart::new();
    write9(&mut cart, ROMCTRL_OFFSET, 0x1F7F_7FFF & !ROMCTRL_RESB);
    assert_eq!(
        romctrl::<true>(&cart),
        0x1F7F_7FFF & !ROMCTRL_RESB & !(1 << 15) & !ROMCTRL_READY
    );
}

#[test]
fn given_a_transfer_running_when_romctrl_is_written_again_with_start_then_it_does_not_restart() {
    let mut cart = ready_read(0x9000, 1);
    read::<true>(&mut cart, DATA_OFFSET, Width::Word);
    start_read::<true>(&mut cart, 0xA000, 1);
    assert_eq!((cart.address(), cart.remaining()), (0x9004, 0x200 - 4));
}

#[test]
fn given_a_started_read_when_inspected_then_address_and_remaining_follow_the_transfer() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x9004, 1);
    assert_eq!((cart.address(), cart.remaining()), (0x9004, 0x200));
}

#[test]
fn given_direct_boot_when_the_state_is_set_then_resb_is_released_and_the_arm9_owns_the_slot() {
    let mut cart = Cart::new();
    write9(&mut cart, 0, 0xFFFF);
    cart.direct_boot();
    assert_eq!(
        (
            cart.romctrl(),
            cart.auxspicnt(),
            cart.exmemcnt9(),
            cart.owner_is_arm9()
        ),
        (RESB, 0, 0, true)
    );
}

#[test]
fn given_a_transfer_in_flight_when_serialized_and_restored_then_the_rest_of_it_matches() {
    let mut cart = ready_read(0x9000, 1);
    read::<true>(&mut cart, DATA_OFFSET, Width::Word);
    let bytes = postcard::to_allocvec(&cart).expect("the slot serializes");
    let mut restored: Cart = postcard::from_bytes(&bytes).expect("the bytes parse");
    assert_eq!(drain9(&mut restored), rom()[0x9004..0x9200]);
}

#[test]
fn given_states_the_slot_can_reach_when_checked_then_they_are_well_formed() {
    let idle = Cart::new();
    let mut busy = Cart::new();
    start_read::<true>(&mut busy, 0x9FFC, 6);
    assert_eq!((idle.well_formed(0), busy.well_formed(0)), (true, true));
}

#[test]
fn given_a_ready_word_with_nothing_left_when_checked_then_it_is_not_well_formed() {
    let mut cart = ready_read(0x8000, 7);
    cart.left = 0;
    assert!(!cart.well_formed(0));
}

#[test]
fn given_a_position_outside_its_block_when_checked_then_it_is_not_well_formed() {
    let mut cart = Cart::new();
    cart.pos = 0x1000;
    assert!(!cart.well_formed(0));
}

#[test]
fn given_a_length_that_is_not_a_word_multiple_when_checked_then_it_is_not_well_formed() {
    let mut cart = Cart::new();
    cart.left = 3;
    assert!(!cart.well_formed(0));
}

// ------------------------------------------------------------------------------------ timing

const SLOW_CLOCK: u32 = 1 << 27;

/// A write of `value` at `offset` as the ARM9 at master time `now`.
fn write_at(cart: &mut Cart, now: u64, offset: u32, value: u32) -> Effect {
    Device::<true>::write(cart, offset, Width::Word, value, cx_with_rom(now, &rom()))
}

/// A read of the data port as the ARM9 at master time `now`.
fn take_at(cart: &mut Cart, now: u64) -> Effect {
    Device::<true>::read(cart, DATA_OFFSET, Width::Word, cx_with_rom(now, &rom())).1
}

/// Starts a read of `size` at master time `now` with the ROMCTRL bits `extra` (clock, gaps).
fn start_at(cart: &mut Cart, now: u64, size: u32, extra: u32) -> Effect {
    write_at(cart, now, COMMAND_OFFSET, 0x0000_00B7);
    write_at(cart, now, COMMAND_OFFSET + 4, 0);
    write_at(
        cart,
        now,
        ROMCTRL_OFFSET,
        START | RESB | extra | (size << SIZE_SHIFT),
    )
}

/// The cases (ROMCTRL bits, size, expected time of the first event) whose event time differs,
/// for a start at master time 1000.
fn wrong_first_event_times(cases: &[(u32, u32, u64)]) -> Vec<(u32, u32)> {
    cases
        .iter()
        .filter(|&&(extra, size, want)| {
            let mut cart = Cart::new();
            start_at(&mut cart, 1000, size, extra);
            cart.next_word_due() != Some(want)
        })
        .map(|&(extra, size, _)| (extra, size))
        .collect()
}

#[test]
fn given_clock_and_gap_settings_when_a_read_starts_then_the_first_word_is_due_after_command_gap_and_word()
 {
    // (8 command clocks + gap 1 + 4 for the word) card clocks of 10 (6.7 MHz) or 16 (4.2 MHz)
    // master cycles after the start (`cart-protocol.md`, "Timing").
    let cases = [
        (0, 1, 1000 + (8 + 4) * 10),
        (0x657, 1, 1000 + (8 + 0x657 + 4) * 10),
        (SLOW_CLOCK, 1, 1000 + (8 + 4) * 16),
        (SLOW_CLOCK | 0x10, 7, 1000 + (8 + 0x10 + 4) * 16),
        (0x1FFF, 1, 1000 + (8 + 0x1FFF + 4) * 10),
    ];
    assert_eq!(wrong_first_event_times(&cases), Vec::<(u32, u32)>::new());
}

#[test]
fn given_a_block_of_size_0_when_it_starts_then_its_event_is_due_after_command_and_gap_only() {
    let cases = [(5, 0, 1000 + (8 + 5) * 10), (SLOW_CLOCK, 0, 1000 + 8 * 16)];
    assert_eq!(wrong_first_event_times(&cases), Vec::<(u32, u32)>::new());
}

#[test]
fn given_a_start_when_it_returns_then_it_names_the_event_slot_to_replace() {
    let mut cart = Cart::new();
    assert_eq!(start_at(&mut cart, 0, 1, 0).reschedule, 1);
}

#[test]
fn given_a_started_read_when_the_event_slot_is_asked_then_it_is_the_next_words_due_time() {
    let mut cart = Cart::new();
    start_at(&mut cart, 1000, 1, 0);
    assert_eq!(Device::<true>::slot_due(&cart, 0), Some(1000 + 12 * 10));
}

#[test]
fn given_a_started_read_when_data_is_read_before_the_event_then_nothing_is_taken() {
    let mut cart = Cart::new();
    start_at(&mut cart, 0, 1, 0);
    take_at(&mut cart, 50);
    assert_eq!(cart.remaining(), 0x200);
}

#[test]
fn given_a_word_taken_when_romctrl_is_read_then_no_word_is_ready_until_the_event_fires() {
    let mut cart = ready_read(0x9000, 1);
    take_at(&mut cart, 500);
    assert_eq!(romctrl::<true>(&cart) & ROMCTRL_READY, 0);
}

/// The time of the event after `words` words were each taken at master time 5000 with gap 2 of 3.
fn due_after_taking(words: usize) -> Option<u64> {
    let mut cart = Cart::new();
    start_at(&mut cart, 0, 2, 3 << 16);
    for _ in 0..words {
        fire(&mut cart);
        take_at(&mut cart, 5000);
    }
    cart.next_word_due()
}

#[test]
fn given_words_taken_inside_a_200h_part_when_the_next_is_scheduled_then_it_is_one_word_later() {
    assert_eq!(due_after_taking(1), Some(5000 + 4 * 10));
    assert_eq!(due_after_taking(127), Some(5000 + 4 * 10));
}

#[test]
fn given_the_128th_word_taken_when_the_next_is_scheduled_then_gap_2_is_added() {
    assert_eq!(due_after_taking(128), Some(5000 + (4 + 3) * 10));
}

#[test]
fn given_a_word_taken_after_the_gap_when_the_next_is_scheduled_then_it_is_one_word_later_again() {
    assert_eq!(due_after_taking(129), Some(5000 + 4 * 10));
}

#[test]
fn given_the_last_word_taken_when_the_transfer_ends_then_no_event_is_left() {
    let mut cart = Cart::new();
    start_at(&mut cart, 0, 7, 0);
    fire(&mut cart);
    take_at(&mut cart, 100);
    assert_eq!(cart.next_word_due(), None);
}

#[test]
fn given_a_word_taken_with_more_left_when_it_returns_then_it_names_the_event_slot_to_replace() {
    let mut cart = ready_read(0x9000, 1);
    assert_eq!(take_at(&mut cart, 100).reschedule, 1);
}

#[test]
fn given_a_word_ready_when_the_event_fires_then_it_asks_for_the_owners_dma_channels() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    assert_eq!(
        fire(&mut cart).trigger,
        Some(Trigger::CardWord { arm9: true })
    );
}

#[test]
fn given_the_arm7_owns_the_slot_when_a_word_is_ready_then_the_request_is_for_the_arm7() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, EXMEMCNT_OFFSET, Width::Half, SLOT_TO_ARM7);
    start_read::<false>(&mut cart, 0x8000, 1);
    assert_eq!(
        fire(&mut cart).trigger,
        Some(Trigger::CardWord { arm9: false })
    );
}

#[test]
fn given_a_block_of_size_0_when_its_event_fires_then_no_dma_request_is_made() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 0);
    assert_eq!(fire(&mut cart).trigger, None);
}

#[test]
fn given_no_event_on_its_way_when_the_slot_fires_then_nothing_happens() {
    let mut cart = Cart::new();
    assert_eq!(
        Device::<true>::slot_fire(&mut cart, 0, 10),
        Fired::default()
    );
}

#[test]
fn given_a_transfer_waiting_for_its_event_when_checked_without_one_then_it_is_not_well_formed() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
    cart.due = None;
    assert!(!cart.well_formed(0));
}

#[test]
fn given_a_transfer_with_its_event_pending_when_serialized_and_restored_then_the_event_time_survives()
 {
    let mut cart = Cart::new();
    start_at(&mut cart, 1000, 1, 0);
    let bytes = postcard::to_allocvec(&cart).expect("the slot serializes");
    let restored: Cart = postcard::from_bytes(&bytes).expect("the bytes parse");
    assert_eq!(restored.next_word_due(), Some(1120));
}

// ----------------------------------------------------------------------------- waiting word

#[test]
fn given_a_ready_word_when_asked_who_it_waits_for_then_only_the_owner_has_one() {
    let cart = ready_read(0x9000, 1);
    assert_eq!(
        (cart.word_waiting::<true>(), cart.word_waiting::<false>()),
        (true, false)
    );
}

#[test]
fn given_no_word_ready_when_asked_who_it_waits_for_then_neither_cpu_has_one() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x9000, 1);
    assert_eq!(
        (cart.word_waiting::<true>(), cart.word_waiting::<false>()),
        (false, false)
    );
}

#[test]
fn given_an_idle_slot_with_words_left_when_checked_then_it_is_not_well_formed() {
    let mut cart = Cart::new();
    cart.left = 4;
    cart.total = 4;
    assert!(!cart.well_formed(0));
}

#[test]
fn given_no_word_ready_when_the_data_port_is_read_then_it_returns_the_last_word_taken() {
    let mut cart = ready_read(0x9000, 1);
    let (taken, _) = read::<true>(&mut cart, DATA_OFFSET, Width::Word);
    let (idle, _) = read::<true>(&mut cart, DATA_OFFSET, Width::Word);
    assert_eq!(idle, taken);
}

// ------------------------------------------------------------------------------------ SPI side

/// AUXSPICNT value: enabled, SPI mode, with `low` bits (baud, hold) on top.
fn spi_cnt(low: u16) -> u32 {
    u32::from(0xA000 | low)
}

/// A cart with a blank 8 KiB EEPROM fitted and AUXSPICNT set to `low` (see [`spi_cnt`]).
fn spi_cart(low: u16) -> Cart {
    let mut cart = Cart::new();
    cart.insert_save(crate::cart::SaveType::Eeprom8K, vec![0xFF; 8192])
        .expect("8 KiB fits");
    write::<true>(&mut cart, 0, Width::Half, spi_cnt(low));
    cart
}

fn spi_write_at<const ARM9: bool>(
    cart: &mut Cart,
    now: u64,
    offset: u32,
    width: Width,
    value: u32,
) {
    Device::<ARM9>::write(cart, offset, width, value, cx_with_rom(now, &[]));
}

fn spi_peek_at<const ARM9: bool>(cart: &Cart, now: u64, offset: u32, width: Width) -> u32 {
    Device::<ARM9>::peek(cart, offset, width, cx_with_rom(now, &[]))
}

const BUSY_BIT: u32 = 1 << 7;
const SPI_DATA: u32 = 2;
const SPI_START: u64 = 1000;

/// The baud rates whose busy bit does not drop exactly `128 << baud` master cycles after a byte.
fn busy_time_failures() -> Vec<u16> {
    (0..4u16)
        .filter(|&baud| {
            let mut cart = spi_cart(baud);
            spi_write_at::<true>(&mut cart, SPI_START, SPI_DATA, Width::Byte, 0x05);
            let end = SPI_START + (128 << baud);
            let before = spi_peek_at::<true>(&cart, end - 1, 0, Width::Half) & BUSY_BIT;
            let at = spi_peek_at::<true>(&cart, end, 0, Width::Half) & BUSY_BIT;
            (before, at) != (BUSY_BIT, 0)
        })
        .collect()
}

#[test]
fn given_each_baud_rate_when_a_byte_is_written_then_busy_drops_after_its_byte_time() {
    assert_eq!(busy_time_failures(), Vec::<u16>::new());
}

#[test]
fn given_a_byte_written_when_auxspidata_is_read_then_it_holds_the_chips_reply() {
    let mut cart = spi_cart(0x40);
    spi_write_at::<true>(&mut cart, 0, SPI_DATA, Width::Byte, 0x05);
    spi_write_at::<true>(&mut cart, 1000, SPI_DATA, Width::Byte, 0x00);
    assert_eq!(
        spi_peek_at::<true>(&cart, 2000, SPI_DATA, Width::Half),
        0x00
    );
}

#[test]
fn given_a_byte_written_while_busy_when_the_first_ends_then_it_was_dropped() {
    let mut cart = spi_cart(0);
    spi_write_at::<true>(&mut cart, 0, SPI_DATA, Width::Byte, 0x06);
    spi_write_at::<true>(&mut cart, 10, SPI_DATA, Width::Byte, 0x05);
    assert!(!cart.save().selected());
}

#[test]
fn given_hold_set_when_a_byte_is_written_then_the_chip_stays_selected_and_otherwise_it_is_not() {
    let mut held = spi_cart(0x40);
    spi_write_at::<true>(&mut held, 0, SPI_DATA, Width::Byte, 0x05);
    let mut released = spi_cart(0);
    spi_write_at::<true>(&mut released, 0, SPI_DATA, Width::Byte, 0x05);
    assert_eq!(
        (held.save().selected(), released.save().selected()),
        (true, false)
    );
}

#[test]
fn given_the_slot_not_in_spi_mode_when_a_byte_is_written_then_nothing_happens() {
    let mut cart = spi_cart(0);
    spi_write_at::<true>(&mut cart, 0, 0, Width::Half, 0x8040);
    spi_write_at::<true>(&mut cart, 10, SPI_DATA, Width::Byte, 0x05);
    assert_eq!((cart.save().selected(), cart.spi_busy(11)), (false, false));
}

#[test]
fn given_the_slot_not_enabled_when_a_byte_is_written_then_nothing_happens() {
    let mut cart = spi_cart(0);
    spi_write_at::<true>(&mut cart, 0, 0, Width::Half, 0x2040);
    spi_write_at::<true>(&mut cart, 10, SPI_DATA, Width::Byte, 0x05);
    assert_eq!((cart.save().selected(), cart.spi_busy(11)), (false, false));
}

#[test]
fn given_a_halfword_write_to_auxspidata_when_read_back_then_only_the_low_byte_was_sent() {
    let mut cart = spi_cart(0x40);
    spi_write_at::<true>(&mut cart, 0, SPI_DATA, Width::Half, 0xAB05);
    assert_eq!(cart.save().command(), Some(0x05));
}

#[test]
fn given_a_byte_write_to_the_high_half_of_auxspidata_when_sent_then_no_transfer_starts() {
    let mut cart = spi_cart(0x40);
    spi_write_at::<true>(&mut cart, 0, SPI_DATA + 1, Width::Byte, 0x05);
    assert_eq!((cart.save().selected(), cart.spi_busy(1)), (false, false));
}

#[test]
fn given_a_word_write_to_auxspicnt_when_its_upper_half_is_data_then_the_transfer_uses_the_new_cnt()
{
    let mut cart = Cart::new();
    cart.insert_save(crate::cart::SaveType::Eeprom8K, vec![0xFF; 8192])
        .expect("8 KiB fits");
    spi_write_at::<true>(&mut cart, 0, 0, Width::Word, spi_cnt(0x40) | 0x05 << 16);
    assert_eq!(
        (cart.save().command(), cart.save().selected()),
        (Some(0x05), true)
    );
}

#[test]
fn given_auxspicnt_written_with_busy_set_when_read_then_the_busy_bit_comes_from_the_time_only() {
    let mut cart = Cart::new();
    write::<true>(&mut cart, 0, Width::Half, 0xFFFF);
    assert_eq!(spi_peek_at::<true>(&cart, 0, 0, Width::Half) & BUSY_BIT, 0);
}

#[test]
fn given_the_arm7_does_not_own_the_slot_when_it_sends_a_byte_then_it_reads_zero_and_nothing_starts()
{
    let mut cart = spi_cart(0x40);
    spi_write_at::<false>(&mut cart, 0, SPI_DATA, Width::Byte, 0x05);
    assert_eq!(
        (
            cart.save().selected(),
            spi_peek_at::<false>(&cart, 10, 0, Width::Half),
            spi_peek_at::<false>(&cart, 10, SPI_DATA, Width::Half)
        ),
        (false, 0, 0)
    );
}

#[test]
fn given_a_busy_time_far_in_the_future_when_the_state_is_checked_then_it_is_not_well_formed() {
    let mut cart = spi_cart(0);
    cart.spi_done = u64::MAX;
    assert!(!cart.well_formed(0));
}
