// SPDX-License-Identifier: Apache-2.0
//! The cartridge slot's registers and the ROM read, from `docs/hw/cart-protocol.md` ("Registers",
//! "Commands", "Ownership", "Behavior"). The ROM is a synthetic image with a known pattern; the
//! scenarios go through helpers and a test body only asserts.

use super::{
    AUXSPICNT_IRQ, COMMAND_OFFSET, Cart, DATA_OFFSET, EXMEMCNT_OFFSET, ROMCTRL_BUSY,
    ROMCTRL_OFFSET, ROMCTRL_READY, ROMCTRL_RESB, block_bytes,
};
use crate::cart::protocol::chip_id;
use crate::io::device::{Checked, Device, Effect, Width, cx_with_rom};
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

fn rom() -> Vec<u8> {
    (0..ROM_LEN).map(pattern).collect()
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
            break;
        }
        let (word, last) = read::<ARM9>(cart, DATA_OFFSET, Width::Word);
        bytes.extend(word.to_le_bytes());
        effect = last;
    }
    (bytes, effect)
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
    assert_eq!(drain9(&mut cart), chip_id(ROM_LEN).to_le_bytes());
}

#[test]
fn given_the_chip_id_command_with_a_long_block_when_drained_then_the_id_repeats() {
    let mut cart = Cart::new();
    start_command(&mut cart, 0xB8, 1);
    let id = chip_id(ROM_LEN).to_le_bytes();
    assert_eq!(drain9(&mut cart), id.repeat(0x200 / 4));
}

#[test]
fn given_an_unknown_command_when_drained_then_every_word_is_ffff_ffff() {
    let mut cart = Cart::new();
    start_command(&mut cart, 0x9F, 7);
    assert_eq!(drain9(&mut cart), vec![0xFF; 4]);
}

#[test]
fn given_a_started_read_when_romctrl_is_read_then_busy_and_ready_are_set() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
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
fn given_a_block_size_of_0_when_started_then_the_transfer_is_done_at_once() {
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 0);
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
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 1);
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
fn given_the_irq_bit_set_when_a_block_of_size_0_starts_then_the_write_raises_if_bit_19() {
    let mut cart = with_irq();
    let effect = start_read::<true>(&mut cart, 0x8000, 0);
    assert_eq!(effect.requests.arm9, CARD_TRANSFER_IRQ);
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
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x9000, 1);
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
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x9000, 1);
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
    let mut cart = Cart::new();
    start_read::<true>(&mut cart, 0x8000, 7);
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
