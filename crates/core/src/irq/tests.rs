// SPDX-License-Identifier: Apache-2.0
//! `InterruptController` on its own. Expected values come from `docs/hw/interrupts.md`
//! ("Registers", "IE and IF bits", "Behavior", "Edge cases"), never from running the code.
//!
//! Scenarios live in harness functions that return what they observed (or the list of cases
//! that differed); a test body only asserts.

use super::*;

/// The registers' offsets in the controller's register space, which starts at IME.
const IME: u32 = 0;
const IE: u32 = IE_ADDR - IME_ADDR;
const IF: u32 = IF_ADDR - IME_ADDR;

/// A byte write at `offset`.
fn write_byte<const ARM9: bool>(c: &mut InterruptController<ARM9>, offset: u32, value: u8) {
    c.write(offset, Width::Byte, u32::from(value), 0);
}

/// A byte read at `offset`.
fn read_byte<const ARM9: bool>(c: &InterruptController<ARM9>, offset: u32) -> u8 {
    c.peek(offset, Width::Byte, 0) as u8
}

/// The note's "IE and IF bits" table: (bit, exists on the ARM9, exists on the ARM7). Bits not
/// listed (14, 15, 25-31) exist on neither.
const BIT_TABLE: [(u32, bool, bool); 32] = [
    (0, true, true),
    (1, true, true),
    (2, true, true),
    (3, true, true),
    (4, true, true),
    (5, true, true),
    (6, true, true),
    (7, false, true),
    (8, true, true),
    (9, true, true),
    (10, true, true),
    (11, true, true),
    (12, true, true),
    (13, true, true),
    (14, false, false),
    (15, false, false),
    (16, true, true),
    (17, true, true),
    (18, true, true),
    (19, true, true),
    (20, true, true),
    (21, true, false),
    (22, false, true),
    (23, false, true),
    (24, false, true),
    (25, false, false),
    (26, false, false),
    (27, false, false),
    (28, false, false),
    (29, false, false),
    (30, false, false),
    (31, false, false),
];

/// Whether the note's table gives `bit` to the ARM9 (`ARM9`) or the ARM7.
fn exists<const ARM9: bool>(bit: u32) -> bool {
    BIT_TABLE
        .iter()
        .any(|&(b, on9, on7)| b == bit && if ARM9 { on9 } else { on7 })
}

/// Writes a 32-bit little-endian value one byte at a time at `addr`.
fn write_word<const ARM9: bool>(c: &mut InterruptController<ARM9>, addr: u32, value: u32) {
    value
        .to_le_bytes()
        .iter()
        .enumerate()
        .for_each(|(i, &b)| write_byte(c, addr + i as u32, b));
}

/// Reads a 32-bit little-endian value at `offset`, byte by byte.
fn read_word<const ARM9: bool>(c: &InterruptController<ARM9>, offset: u32) -> u32 {
    (0..4).fold(0, |value, i| {
        value | u32::from(read_byte(c, offset + i)) << (8 * i)
    })
}

// ---------------------------------------------------------------- reset

#[test]
fn given_new_controllers_when_read_then_ime_ie_and_if_are_all_zero_on_both_cpus() {
    let a9 = InterruptController::<true>::new();
    let a7 = InterruptController::<false>::new();
    let seen = [(a9.ime(), a9.ie(), a9.if_()), (a7.ime(), a7.ie(), a7.if_())];
    assert_eq!(seen, [(false, 0, 0), (false, 0, 0)]);
}

#[test]
fn given_new_controllers_when_the_line_is_read_then_it_is_low() {
    let lines = [
        InterruptController::<true>::new().line(),
        InterruptController::<false>::new().line(),
    ];
    assert_eq!(lines, [false, false]);
}

// ---------------------------------------------------------------- IME

#[test]
fn given_a_word_of_all_ones_written_to_ime_when_read_then_only_bit_0_is_kept() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IME, 0xFFFF_FFFF);
    assert_eq!((c.ime(), read_word(&c, IME)), (true, 1));
}

/// IME's state and its four bytes after byte writes of FF to bytes 1-3.
fn ime_after_high_byte_writes() -> (bool, Vec<u8>) {
    let mut c = InterruptController::<true>::new();
    (1..4).for_each(|i| write_byte(&mut c, IME + i, 0xFF));
    (c.ime(), (0..4).map(|i| read_byte(&c, IME + i)).collect())
}

#[test]
fn given_bytes_1_to_3_of_ime_written_with_ff_when_read_then_ime_is_still_off_and_they_read_0() {
    assert_eq!(ime_after_high_byte_writes(), (false, vec![0, 0, 0, 0]));
}

#[test]
fn given_an_even_value_written_to_the_ime_low_byte_when_read_then_ime_is_off() {
    let mut c = InterruptController::<false>::new();
    write_byte(&mut c, IME, 0x01);
    write_byte(&mut c, IME, 0xFE);
    assert_eq!((c.ime(), read_byte(&c, IME)), (false, 0));
}

#[test]
fn given_ime_on_when_0_is_written_to_it_then_it_is_off() {
    let mut c = InterruptController::<false>::new();
    write_word(&mut c, IME, 1);
    write_word(&mut c, IME, 0);
    assert!(!c.ime());
}

// ---------------------------------------------------------------- IE

/// The bits that stay set after all ones are written to IE, minus those the table says exist.
fn ie_bit_differences<const ARM9: bool>() -> Vec<u32> {
    let mut c = InterruptController::<ARM9>::new();
    write_word(&mut c, IE, 0xFFFF_FFFF);
    (0..32)
        .filter(|&bit| (c.ie() >> bit & 1 == 1) != exists::<ARM9>(bit))
        .collect()
}

#[test]
fn given_all_ones_written_to_the_arm9_ie_when_read_then_exactly_the_table_bits_remain() {
    assert_eq!(ie_bit_differences::<true>(), Vec::<u32>::new());
}

#[test]
fn given_all_ones_written_to_the_arm7_ie_when_read_then_exactly_the_table_bits_remain() {
    assert_eq!(ie_bit_differences::<false>(), Vec::<u32>::new());
}

/// (bit, ie after writing only that bit) that differ from "kept iff the table says so".
fn single_bit_ie_failures<const ARM9: bool>() -> Vec<(u32, u32)> {
    (0..32)
        .filter_map(|bit| {
            let mut c = InterruptController::<ARM9>::new();
            write_word(&mut c, IE, 1 << bit);
            let want = if exists::<ARM9>(bit) { 1 << bit } else { 0 };
            (c.ie() != want).then_some((bit, c.ie()))
        })
        .collect()
}

#[test]
fn given_each_single_bit_written_to_ie_when_read_then_it_is_kept_only_where_the_cpu_has_it() {
    assert_eq!(
        (
            single_bit_ie_failures::<true>(),
            single_bit_ie_failures::<false>()
        ),
        (vec![], vec![])
    );
}

#[test]
fn given_ie_set_when_0_is_written_to_it_then_it_is_cleared_not_ored() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IE, 0xFFFF_FFFF);
    write_word(&mut c, IE, 0x0000_0001);
    assert_eq!(c.ie(), 1);
}

/// A CPU's four IE bytes after byte writes of FF to each.
fn ie_bytes_after_ff_writes<const ARM9: bool>() -> Vec<u8> {
    let mut c = InterruptController::<ARM9>::new();
    (0..4).for_each(|i| write_byte(&mut c, IE + i, 0xFF));
    (0..4).map(|i| read_byte(&c, IE + i)).collect()
}

#[test]
fn given_byte_writes_of_ff_to_each_ie_byte_when_read_then_each_byte_holds_its_cpus_valid_bits() {
    assert_eq!(
        (
            ie_bytes_after_ff_writes::<true>(),
            ie_bytes_after_ff_writes::<false>()
        ),
        (vec![0x7F, 0x3F, 0x3F, 0x00], vec![0xFF, 0x3F, 0xDF, 0x01])
    );
}

#[test]
fn given_a_byte_write_to_ie_when_read_then_only_that_byte_changed() {
    let mut c = InterruptController::<false>::new();
    write_word(&mut c, IE, 0x0000_0001);
    write_byte(&mut c, IE + 2, 0x04);
    assert_eq!(c.ie(), 0x0004_0001);
}

// ---------------------------------------------------------------- IF: request

#[test]
fn given_ime_off_and_ie_zero_when_every_bit_is_requested_then_if_holds_exactly_the_valid_bits() {
    let mut a9 = InterruptController::<true>::new();
    let mut a7 = InterruptController::<false>::new();
    a9.request(0xFFFF_FFFF);
    a7.request(0xFFFF_FFFF);
    assert_eq!((a9.if_(), a7.if_()), (0x003F_3F7F, 0x01DF_3FFF));
}

#[test]
fn given_requests_of_bits_that_do_not_exist_when_latched_then_if_stays_zero() {
    let mut a9 = InterruptController::<true>::new();
    let mut a7 = InterruptController::<false>::new();
    // Bit 7 exists only on the ARM7, bits 22-24 only on the ARM7, bit 21 only on the ARM9,
    // bits 14, 15 and 25-31 on neither.
    a9.request(1 << 7 | 1 << 14 | 1 << 22 | 1 << 24 | 1 << 31);
    a7.request(1 << 14 | 1 << 15 | 1 << 21 | 1 << 25 | 1 << 31);
    assert_eq!((a9.if_(), a7.if_()), (0, 0));
}

#[test]
fn given_bit_7_and_bit_21_requested_when_latched_then_each_lands_only_on_the_cpu_that_has_it() {
    let mut a9 = InterruptController::<true>::new();
    let mut a7 = InterruptController::<false>::new();
    a9.request(1 << 7 | 1 << 21);
    a7.request(1 << 7 | 1 << 21);
    assert_eq!((a9.if_(), a7.if_()), (1 << 21, 1 << 7));
}

#[test]
fn given_two_requests_when_latched_then_if_holds_both() {
    let mut c = InterruptController::<true>::new();
    c.request(1 << 3);
    c.request(1 << 16);
    assert_eq!(c.if_(), 1 << 3 | 1 << 16);
}

#[test]
fn given_a_latched_request_when_it_is_requested_again_then_if_is_unchanged() {
    let mut c = InterruptController::<true>::new();
    c.request(1 << 3);
    c.request(1 << 3);
    assert_eq!(c.if_(), 1 << 3);
}

#[test]
fn given_a_request_when_if_is_read_by_bytes_then_the_bytes_are_the_little_endian_parts() {
    let mut c = InterruptController::<true>::new();
    c.request(1 << 0 | 1 << 9 | 1 << 17 | 1 << 21);
    assert_eq!(read_word(&c, IF), 0x0022_0201);
}

// ---------------------------------------------------------------- IF: acknowledge

#[test]
fn given_if_bits_set_when_1_is_written_to_one_of_them_then_only_that_bit_clears() {
    let mut c = InterruptController::<true>::new();
    c.request(0b111);
    write_byte(&mut c, IF, 0x01);
    assert_eq!(c.if_(), 0b110);
}

#[test]
fn given_if_bits_set_when_0_is_written_then_nothing_changes() {
    let mut c = InterruptController::<true>::new();
    c.request(0x0003_0107);
    write_word(&mut c, IF, 0);
    assert_eq!(c.if_(), 0x0003_0107);
}

#[test]
fn given_if_clear_when_1_is_written_to_a_bit_then_it_does_not_become_set() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IF, 0xFFFF_FFFF);
    assert_eq!(c.if_(), 0);
}

#[test]
fn given_if_bits_in_every_byte_when_ff_is_written_to_one_byte_then_only_that_byte_clears() {
    let mut c = InterruptController::<true>::new();
    c.request(1 << 0 | 1 << 9 | 1 << 17 | 1 << 21);
    write_byte(&mut c, IF + 2, 0xFF);
    assert_eq!(c.if_(), 1 << 0 | 1 << 9);
}

#[test]
fn given_if_bits_in_every_byte_when_a_halfword_of_ones_is_written_then_only_that_half_clears() {
    let mut c = InterruptController::<true>::new();
    c.request(1 << 0 | 1 << 9 | 1 << 17 | 1 << 21);
    write_byte(&mut c, IF, 0xFF);
    write_byte(&mut c, IF + 1, 0xFF);
    assert_eq!(c.if_(), 1 << 17 | 1 << 21);
}

#[test]
fn given_if_bits_set_when_a_word_of_ones_is_written_then_all_clear() {
    let mut c = InterruptController::<false>::new();
    c.request(0xFFFF_FFFF);
    write_word(&mut c, IF, 0xFFFF_FFFF);
    assert_eq!(c.if_(), 0);
}

#[test]
fn given_a_write_to_if_when_the_other_registers_are_read_then_ime_and_ie_are_untouched() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IME, 1);
    write_word(&mut c, IE, 0x0000_0005);
    c.request(0b101);
    write_word(&mut c, IF, 0xFFFF_FFFF);
    assert_eq!((c.ime(), c.ie()), (true, 5));
}

// ---------------------------------------------------------------- the line

/// (ime, ie, requested, expected line) for the rule `IME = 1 and (IE & IF) != 0`.
const LINE_CASES: [(u32, u32, u32, bool); 8] = [
    (1, 0x1, 0x1, true),
    (0, 0x1, 0x1, false),
    (1, 0x0, 0x1, false),
    (1, 0x1, 0x0, false),
    (1, 0x2, 0x1, false),
    (1, 0x3, 0x2, true),
    (1, 0x0003_0000, 0x0001_0000, true),
    (0, 0x0, 0x0, false),
];

fn line_failures<const ARM9: bool>() -> Vec<(u32, u32, u32, bool)> {
    LINE_CASES
        .iter()
        .filter(|&&(ime, ie, req, want)| {
            let mut c = InterruptController::<ARM9>::new();
            write_word(&mut c, IME, ime);
            write_word(&mut c, IE, ie);
            c.request(req);
            c.line() != want
        })
        .copied()
        .collect()
}

#[test]
fn given_ime_ie_and_if_combinations_when_the_arm9_line_is_read_then_it_follows_ime_and_ie_and_if() {
    assert_eq!(line_failures::<true>(), vec![]);
}

#[test]
fn given_ime_ie_and_if_combinations_when_the_arm7_line_is_read_then_it_follows_ime_and_ie_and_if() {
    assert_eq!(line_failures::<false>(), vec![]);
}

#[test]
fn given_ime_off_and_a_request_when_if_is_read_then_it_holds_the_request_and_the_line_is_low() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IE, 1);
    c.request(1);
    assert_eq!((c.if_(), c.line()), (1, false));
}

#[test]
fn given_a_pending_enabled_request_when_ime_is_set_later_then_the_line_goes_high() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IE, 1);
    c.request(1);
    write_word(&mut c, IME, 1);
    assert_eq!((c.if_(), c.line()), (1, true));
}

#[test]
fn given_a_latched_request_when_ie_enables_it_later_then_the_line_goes_high() {
    let mut c = InterruptController::<false>::new();
    write_word(&mut c, IME, 1);
    c.request(1 << 8);
    let before = c.line();
    write_word(&mut c, IE, 1 << 8);
    assert_eq!((before, c.line()), (false, true));
}

#[test]
fn given_a_high_line_when_the_request_is_acknowledged_then_the_line_drops() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IME, 1);
    write_word(&mut c, IE, 1);
    c.request(1);
    let before = c.line();
    write_byte(&mut c, IF, 1);
    assert_eq!((before, c.line()), (true, false));
}

#[test]
fn given_a_request_for_a_bit_that_does_not_exist_when_ie_is_all_ones_then_the_line_stays_low() {
    let mut c = InterruptController::<true>::new();
    write_word(&mut c, IME, 1);
    write_word(&mut c, IE, 0xFFFF_FFFF);
    c.request(1 << 7);
    assert!(!c.line());
}

// ---------------------------------------------------------------- the hole between IME and IE

// Which addresses reach the controller at all is the I/O map's to say (`io.rs` tests); what is
// left to the controller is the word between IME (208-20B) and IE (210-213), inside its block.

/// The offsets of the word between IME and IE (`20C`-`20F`).
const HOLE: [u32; 4] = [IME + 4, IME + 5, IME + 6, IME + 7];

/// A controller with state in every register.
fn controller_with_state() -> InterruptController<false> {
    let mut c = InterruptController::<false>::new();
    write_word(&mut c, IME, 1);
    write_word(&mut c, IE, 0x0000_00FF);
    c.request(0x0000_0F0F);
    c
}

#[test]
fn given_the_word_between_ime_and_ie_when_read_then_each_byte_reads_0() {
    let c = controller_with_state();
    let nonzero: Vec<u32> = HOLE
        .iter()
        .copied()
        .filter(|&offset| read_byte(&c, offset) != 0)
        .collect();
    assert_eq!(nonzero, Vec::<u32>::new());
}

/// A controller with state, before and after byte writes of FF into the hole.
fn controllers_around_hole_writes() -> (InterruptController<false>, InterruptController<false>) {
    let mut c = controller_with_state();
    let before = c.clone();
    HOLE.iter()
        .for_each(|&offset| write_byte(&mut c, offset, 0xFF));
    (before, c)
}

#[test]
fn given_the_word_between_ime_and_ie_when_written_then_the_controller_is_unchanged() {
    let (before, after) = controllers_around_hole_writes();
    assert_eq!((after, before.if_()), (before, 0x0F0F));
}

#[test]
fn given_two_requests_when_merged_then_each_cpus_mask_is_ored() {
    let merged = Requests {
        arm9: 0b0011,
        arm7: 0b0100,
    }
    .merge(Requests {
        arm9: 0b0110,
        arm7: 0b1000,
    });
    assert_eq!(
        merged,
        Requests {
            arm9: 0b0111,
            arm7: 0b1100,
        }
    );
}

#[test]
fn given_a_mask_when_built_for_the_arm9_then_only_the_arm9_side_is_filled() {
    assert_eq!(
        Requests::for_cpu::<true>(TIMER0_IRQ),
        Requests {
            arm9: TIMER0_IRQ,
            arm7: 0,
        }
    );
}

#[test]
fn given_a_mask_when_built_for_the_arm7_then_only_the_arm7_side_is_filled() {
    assert_eq!(
        Requests::for_cpu::<false>(TIMER0_IRQ),
        Requests {
            arm9: 0,
            arm7: TIMER0_IRQ,
        }
    );
}
