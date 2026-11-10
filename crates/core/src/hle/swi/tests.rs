// SPDX-License-Identifier: Apache-2.0
//! The copy, math, checksum and decompression SWIs, on a flat test bus. Expected values come from
//! `docs/hw/hle-swi.md` ("The copy, math and decompression SWIs", "Edge cases"), never from running
//! the code. The compressed inputs are written by hand below or built by the small encoder in
//! this file; nothing is taken from outside the repository.
//!
//! The bus ignores the address bits below the access width, as the memory system does
//! (`arm9-memory-map.md`, "Alignment"), and records every access with its width.

use super::copy::{cpu_fast_set, cpu_set};
use super::lz77::{Stream, decode, decode_vram, decode_wram};
use super::math::{divide, sqrt};
use super::misc::crc16_halfwords;
use crate::cpu::Bus;
use proptest::prelude::*;

const BASE: u32 = 0x0200_0000;
const SRC: u32 = BASE + 0x100;
const DST: u32 = BASE + 0x400;
/// Control bits: fill and 32-bit (`hle-swi.md`, "CpuSet").
const FILL: u32 = 1 << 24;
const WORD: u32 = 1 << 26;

/// 64 KiB of memory at `BASE`, mirrored; every access recorded as `(width in bytes, address)`.
struct FlatBus {
    mem: Vec<u8>,
    reads: Vec<(u8, u32)>,
    writes: Vec<(u8, u32)>,
}

impl FlatBus {
    fn new() -> Self {
        Self {
            mem: vec![0; 0x1_0000],
            reads: Vec::new(),
            writes: Vec::new(),
        }
    }

    fn at(addr: u32) -> usize {
        (addr & 0xFFFF) as usize
    }

    fn put(&mut self, addr: u32, bytes: &[u8]) {
        bytes
            .iter()
            .enumerate()
            .for_each(|(i, &b)| self.mem[Self::at(addr + i as u32)] = b);
    }

    fn get(&self, addr: u32, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| self.mem[Self::at(addr + i as u32)])
            .collect()
    }

    fn fill(&mut self, addr: u32, len: usize, value: u8) {
        self.put(addr, &vec![value; len]);
    }

    fn word(&self, addr: u32) -> u32 {
        u32::from_le_bytes(self.get(addr, 4).try_into().expect("four bytes"))
    }
}

impl Bus for FlatBus {
    fn fetch32(&mut self, _addr: u32) -> u32 {
        0
    }
    fn fetch16(&mut self, _addr: u32) -> u16 {
        0
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.reads.push((1, addr));
        self.mem[Self::at(addr)]
    }
    fn read16(&mut self, addr: u32) -> u16 {
        let addr = addr & !1;
        self.reads.push((2, addr));
        u16::from_le_bytes([self.mem[Self::at(addr)], self.mem[Self::at(addr + 1)]])
    }
    fn read32(&mut self, addr: u32) -> u32 {
        let addr = addr & !3;
        self.reads.push((4, addr));
        self.word(addr)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.writes.push((1, addr));
        self.mem[Self::at(addr)] = value;
    }
    fn write16(&mut self, addr: u32, value: u16) {
        let addr = addr & !1;
        self.writes.push((2, addr));
        self.put(addr, &value.to_le_bytes());
    }
    fn write32(&mut self, addr: u32, value: u32) {
        let addr = addr & !3;
        self.writes.push((4, addr));
        self.put(addr, &value.to_le_bytes());
    }
}

/// A bus with `words` at `SRC` and the destination area filled with `AAh`.
fn bus_with_words(words: &[u32]) -> FlatBus {
    let mut bus = FlatBus::new();
    words
        .iter()
        .enumerate()
        .for_each(|(i, &w)| bus.put(SRC + 4 * i as u32, &w.to_le_bytes()));
    bus.fill(DST, 0x100, 0xAA);
    bus
}

fn words_at(bus: &FlatBus, addr: u32, count: u32) -> Vec<u32> {
    (0..count).map(|i| bus.word(addr + 4 * i)).collect()
}

// ------------------------------------------------------------------------------------ Div

/// Rows: (numerator, denominator, quotient, remainder, absolute quotient).
fn div_failures(rows: &[(i32, i32, i32, i32, u32)]) -> Vec<(i32, i32, (i32, i32, u32))> {
    rows.iter()
        .filter(|&&(n, d, q, r, a)| divide(n, d) != (q, r, a))
        .map(|&(n, d, ..)| (n, d, divide(n, d)))
        .collect()
}

#[test]
fn given_signed_operands_when_dividing_then_the_quotient_truncates_toward_zero_and_the_remainder_follows_the_numerator()
 {
    let rows = [
        (7, 2, 3, 1, 3),
        (-7, 2, -3, -1, 3),
        (7, -2, -3, 1, 3),
        (-7, -2, 3, -1, 3),
        (0, 5, 0, 0, 0),
        (6, 3, 2, 0, 2),
        (1, 7, 0, 1, 0),
        (-1, 7, 0, -1, 0),
    ];
    assert_eq!(div_failures(&rows), vec![]);
}

#[test]
fn given_the_most_negative_numerator_when_dividing_by_one_then_the_absolute_quotient_is_80000000h()
{
    assert_eq!(divide(i32::MIN, 1), (i32::MIN, 0, 0x8000_0000));
}

// TODO(hw-unverified): the zero denominator and the overflow case are the model's
// (`hle-swi.md`, "Unverified").
#[test]
fn given_a_zero_denominator_when_dividing_then_the_result_is_plus_or_minus_one_as_the_note_models_unverified()
 {
    let rows = [(5, 0, 1, 5, 1), (-5, 0, -1, -5, 1), (0, 0, 1, 0, 1)];
    assert_eq!(div_failures(&rows), vec![]);
}

#[test]
fn given_the_overflowing_division_when_dividing_then_the_result_wraps_as_the_note_models_unverified()
 {
    assert_eq!(divide(i32::MIN, -1), (i32::MIN, 0, 0x8000_0000));
}

proptest! {
    #[test]
    fn given_any_nonzero_denominator_when_dividing_then_quotient_times_denominator_plus_remainder_is_the_numerator(
        n in any::<i32>(),
        d in any::<i32>().prop_filter("not zero or the overflow", |&d| d != 0 && d != -1),
    ) {
        let (q, r, a) = divide(n, d);
        prop_assert_eq!(i64::from(q) * i64::from(d) + i64::from(r), i64::from(n));
        prop_assert!(i64::from(r).abs() < i64::from(d).abs());
        prop_assert!(r == 0 || (r < 0) == (n < 0));
        prop_assert_eq!(a, q.unsigned_abs());
    }
}

// ----------------------------------------------------------------------------------- Sqrt

#[test]
fn given_known_values_when_taking_the_square_root_then_the_floor_is_returned() {
    let rows = [
        (0, 0),
        (1, 1),
        (3, 1),
        (4, 2),
        (15, 3),
        (16, 4),
        (1_000_000, 1000),
        (999_999, 999),
        (0xFFFE_0001, 0xFFFF),
        (0xFFFE_0000, 0xFFFE),
        (u32::MAX, 0xFFFF),
    ];
    let failures: Vec<_> = rows.iter().filter(|&&(n, r)| sqrt(n) != r).collect();
    assert_eq!(failures, Vec::<&(u32, u32)>::new());
}

proptest! {
    #[test]
    fn given_any_value_when_taking_the_square_root_then_the_result_squared_brackets_it(n in any::<u32>()) {
        let r = u64::from(sqrt(n));
        prop_assert!(r * r <= u64::from(n));
        prop_assert!(u64::from(n) < (r + 1) * (r + 1));
    }
}

// ------------------------------------------------------------------------------- CpuSet

#[test]
fn given_a_16_bit_copy_when_run_then_halfwords_move_and_nothing_else_is_written() {
    let mut bus = bus_with_words(&[0x2222_1111, 0x4444_3333]);
    cpu_set(&mut bus, SRC, DST, 3);
    assert_eq!(
        (
            words_at(&bus, DST, 2),
            bus.writes.iter().map(|w| w.0).collect::<Vec<_>>()
        ),
        (vec![0x2222_1111, 0xAAAA_3333], vec![2, 2, 2])
    );
}

#[test]
fn given_a_32_bit_copy_when_run_then_words_move_in_order() {
    let mut bus = bus_with_words(&[1, 2, 3, 4]);
    cpu_set(&mut bus, SRC, DST, WORD | 3);
    assert_eq!(
        (
            words_at(&bus, DST, 4),
            bus.writes.iter().map(|w| w.0).collect::<Vec<_>>()
        ),
        (vec![1, 2, 3, 0xAAAA_AAAA], vec![4, 4, 4])
    );
}

#[test]
fn given_a_16_bit_fill_when_run_then_the_source_is_read_once_and_written_count_times() {
    let mut bus = bus_with_words(&[0x0000_BEEF, 0x1111_1111]);
    cpu_set(&mut bus, SRC, DST, FILL | 3);
    assert_eq!(
        (words_at(&bus, DST, 2), bus.reads.len(), bus.writes.len()),
        (vec![0xBEEF_BEEF, 0xAAAA_BEEF], 1, 3)
    );
}

#[test]
fn given_a_32_bit_fill_when_run_then_the_source_word_fills_count_words() {
    let mut bus = bus_with_words(&[0xDEAD_BEEF, 0x1111_1111]);
    cpu_set(&mut bus, SRC, DST, FILL | WORD | 3);
    assert_eq!(
        (words_at(&bus, DST, 4), bus.reads.len()),
        (
            vec![0xDEAD_BEEF; 3]
                .into_iter()
                .chain([0xAAAA_AAAA])
                .collect(),
            1
        )
    );
}

#[test]
fn given_a_count_of_zero_when_run_in_every_mode_then_the_bus_is_not_touched() {
    let touched: Vec<usize> = [0, FILL, WORD, FILL | WORD]
        .iter()
        .map(|&mode| {
            let mut bus = bus_with_words(&[1, 2]);
            cpu_set(&mut bus, SRC, DST, mode);
            cpu_fast_set(&mut bus, SRC, DST, mode);
            bus.reads.len() + bus.writes.len()
        })
        .collect();
    assert_eq!(touched, [0, 0, 0, 0]);
}

#[test]
fn given_control_bits_other_than_the_count_fill_and_width_when_run_then_they_are_ignored() {
    let mut bus = bus_with_words(&[0x0002_0001, 0x0004_0003]);
    // Bits 21-23, 25, 29 and 31 set (not 24 or 26): the count stays 2 and the copy is 16-bit.
    cpu_set(&mut bus, SRC, DST, 0xA2E0_0002);
    assert_eq!(
        (words_at(&bus, DST, 2), bus.writes.len()),
        (vec![0x0002_0001, 0xAAAA_AAAA], 2)
    );
}

#[test]
fn given_unaligned_addresses_when_copying_words_then_both_are_aligned_down() {
    let mut bus = bus_with_words(&[0x0403_0201, 0x0807_0605]);
    cpu_set(&mut bus, SRC + 1, DST + 3, WORD | 1);
    assert_eq!(words_at(&bus, DST, 2), vec![0x0403_0201, 0xAAAA_AAAA]);
}

#[test]
fn given_a_fast_copy_when_the_count_is_not_a_multiple_of_8_then_it_rounds_up_to_the_next_8_words() {
    let source: Vec<u32> = (1..=20).collect();
    let moved: Vec<u32> = [1, 8, 9, 16, 17]
        .iter()
        .map(|&count| {
            let mut bus = bus_with_words(&source);
            cpu_fast_set(&mut bus, SRC, DST, count);
            bus.writes.len() as u32
        })
        .collect();
    assert_eq!(moved, [8, 8, 16, 16, 24]);
}

#[test]
fn given_a_fast_copy_of_nine_words_when_run_then_sixteen_words_are_copied_and_the_17th_is_untouched()
 {
    let source: Vec<u32> = (1..=20).collect();
    let mut bus = bus_with_words(&source);
    cpu_fast_set(&mut bus, SRC, DST, 9);
    assert_eq!(
        (words_at(&bus, DST, 16), words_at(&bus, DST + 64, 1)),
        ((1..=16).collect::<Vec<u32>>(), vec![0xAAAA_AAAA])
    );
}

#[test]
fn given_a_fast_fill_of_one_word_when_run_then_eight_words_are_filled_from_one_read() {
    let mut bus = bus_with_words(&[0x1234_5678, 0]);
    cpu_fast_set(&mut bus, SRC, DST, FILL | 1);
    assert_eq!(
        (words_at(&bus, DST, 9), bus.reads.len()),
        (
            vec![0x1234_5678; 8]
                .into_iter()
                .chain([0xAAAA_AAAA])
                .collect(),
            1
        )
    );
}

#[test]
fn given_the_width_bit_when_the_fast_set_runs_then_the_units_are_always_words() {
    let mut bus = bus_with_words(&[7, 8]);
    cpu_fast_set(&mut bus, SRC, DST, 1);
    assert_eq!(
        bus.writes.iter().map(|w| w.0).collect::<Vec<_>>(),
        vec![4; 8]
    );
}

// ------------------------------------------------------------------------------- GetCRC16

/// CRC-16/MODBUS, written bit by bit from the note, continued from `crc`.
fn reference_crc(mut crc: u16, bytes: &[u8]) -> u16 {
    bytes.iter().for_each(|&byte| {
        crc ^= u16::from(byte);
        (0..8).for_each(|_| crc = (crc >> 1) ^ (0xA001 * (crc & 1)));
    });
    crc
}

#[test]
fn given_an_even_length_when_taking_the_crc_then_it_matches_crc16_modbus_over_the_bytes() {
    let mut bus = FlatBus::new();
    bus.put(SRC, b"12345678");
    assert_eq!(
        crc16_halfwords(&mut bus, 0xFFFF, SRC, 8),
        reference_crc(0xFFFF, b"12345678")
    );
}

#[test]
fn given_a_split_buffer_when_the_crc_continues_from_the_first_part_then_it_equals_the_whole() {
    let mut bus = FlatBus::new();
    bus.put(SRC, b"12345678");
    let first = crc16_halfwords(&mut bus, 0xFFFF, SRC, 4);
    assert_eq!(
        crc16_halfwords(&mut bus, first, SRC + 4, 4),
        reference_crc(0xFFFF, b"12345678")
    );
}

#[test]
fn given_an_odd_length_when_taking_the_crc_then_the_last_byte_is_left_out() {
    let mut bus = FlatBus::new();
    bus.put(SRC, b"1234567");
    assert_eq!(
        crc16_halfwords(&mut bus, 0xFFFF, SRC, 7),
        reference_crc(0xFFFF, b"123456")
    );
}

#[test]
fn given_a_zero_length_when_taking_the_crc_then_the_initial_value_comes_back_and_nothing_is_read() {
    let mut bus = FlatBus::new();
    assert_eq!(
        (crc16_halfwords(&mut bus, 0x1234, SRC, 0), bus.reads.len()),
        (0x1234, 0)
    );
}

#[test]
fn given_the_crc_when_reading_the_buffer_then_it_is_read_as_halfwords() {
    let mut bus = FlatBus::new();
    bus.put(SRC, b"12345678");
    let _ = crc16_halfwords(&mut bus, 0xFFFF, SRC, 8);
    assert_eq!(
        bus.reads.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![2, 2, 2, 2]
    );
}

// ---------------------------------------------------------------------------------- LZ77

/// A stream over a vector: input from `data` (zero past its end), output collected in a vector; a
/// distance that reaches before the start reads 0.
struct VecStream<'a> {
    input: std::iter::Copied<std::slice::Iter<'a, u8>>,
    out: Vec<u8>,
}

impl Stream for VecStream<'_> {
    fn next(&mut self) -> u8 {
        self.input.next().unwrap_or(0)
    }
    fn put(&mut self, byte: u8) {
        self.out.push(byte);
    }
    fn back(&mut self, distance: u32) -> u8 {
        self.out
            .len()
            .checked_sub(distance as usize)
            .map_or(0, |i| self.out[i])
    }
}

fn decoded(size: u32, data: &[u8]) -> Vec<u8> {
    let mut stream = VecStream {
        input: data.iter().copied(),
        out: Vec::new(),
    };
    decode(size, &mut stream);
    stream.out
}

/// The hand-written stream for "ABABABABAB": flags 0010_0000 (literal, literal, block), `A`, `B`,
/// then a block of length 8 (field 5) at distance 2 (field 1): bytes 50h 01h.
const OVERLAP: [u8; 6] = [0x20, 0x41, 0x42, 0x50, 0x01, 0x00];

#[test]
fn given_literals_only_when_decoding_then_the_bytes_come_out_in_order() {
    // Flag byte 0: eight literals.
    let data = [0x00, 1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(decoded(8, &data), vec![1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn given_a_block_overlapping_its_own_output_when_decoding_then_the_pattern_repeats() {
    assert_eq!(decoded(10, &OVERLAP), b"ABABABABAB".to_vec());
}

#[test]
fn given_a_block_at_distance_1_when_decoding_then_the_last_byte_repeats() {
    // Literal `Z`, then a block of length 5 (field 2) at distance 1 (field 0).
    assert_eq!(decoded(6, &[0x40, b'Z', 0x20, 0x00]), b"ZZZZZZ".to_vec());
}

#[test]
fn given_a_size_inside_a_block_when_decoding_then_the_block_is_cut_at_the_size() {
    assert_eq!(decoded(5, &OVERLAP), b"ABABA".to_vec());
}

#[test]
fn given_a_size_of_zero_when_decoding_then_nothing_is_output() {
    assert_eq!(decoded(0, &OVERLAP), Vec::<u8>::new());
}

#[test]
fn given_more_than_eight_items_when_decoding_then_the_next_flag_byte_is_read_after_the_eighth() {
    // Eight literals under flag 00h, then a flag byte 80h: a block (len 3, distance 8), then
    // seven further literal flags that are never used.
    let mut data = vec![0x00, 1, 2, 3, 4, 5, 6, 7, 8, 0x80, 0x00, 0x07];
    data.push(0);
    assert_eq!(decoded(11, &data), vec![1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3]);
}

#[test]
fn given_a_distance_reaching_before_the_start_when_decoding_then_the_missing_bytes_read_as_the_sink_gives()
 {
    // A first-item block, length 3, distance 1: the stream has nothing behind it and gives 0.
    assert_eq!(decoded(3, &[0x80, 0x00, 0x00]), vec![0, 0, 0]);
}

/// 4096 distinct-ish literals, 8 to a flag byte (flags 00h), then a block of length 3 at the
/// largest distance, 4096.
fn stream_with_the_largest_distance(literals: &[u8]) -> Vec<u8> {
    let mut data: Vec<u8> = literals
        .chunks(8)
        .flat_map(|group| std::iter::once(0x00).chain(group.iter().copied()))
        .collect();
    data.extend_from_slice(&[0x80, 0x0F, 0xFF]);
    data
}

#[test]
fn given_the_maximum_distance_when_decoding_then_the_field_is_12_bits_plus_one() {
    let literals: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    let out = decoded(4096 + 3, &stream_with_the_largest_distance(&literals));
    assert_eq!(out[4096..], literals[..3]);
}

/// A flag byte plus the items it governs (up to 8).
fn encode_group(group: &[Item]) -> Vec<u8> {
    let flags = group.iter().enumerate().fold(0u8, |f, (i, item)| {
        f | (u8::from(matches!(item, Item::Block(..))) << (7 - i))
    });
    let mut out = vec![flags];
    group.iter().for_each(|item| match *item {
        Item::Literal(b) => out.push(b),
        Item::Block(len, distance) => {
            out.push(((len - 3) << 4) as u8 | ((distance - 1) >> 8) as u8);
            out.push(((distance - 1) & 0xFF) as u8);
        }
    });
    out
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Literal(u8),
    Block(u32, u32),
}

/// A small encoder for the tests: greedy matches against the last 32 bytes.
fn encode(data: &[u8]) -> Vec<u8> {
    let mut items = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let best = (1..=32.min(at))
            .map(|distance| {
                let len = (0..18.min(data.len() - at))
                    .take_while(|&k| data[at + k] == data[at + k - distance])
                    .count();
                (len, distance)
            })
            .max();
        match best {
            Some((len, distance)) if len >= 3 => {
                items.push(Item::Block(len as u32, distance as u32));
                at += len;
            }
            _ => {
                items.push(Item::Literal(data[at]));
                at += 1;
            }
        }
    }
    items.chunks(8).flat_map(encode_group).collect()
}

proptest! {
    #[test]
    fn given_any_data_when_encoded_and_decoded_then_it_comes_back(
        data in proptest::collection::vec(0u8..4, 0..300),
    ) {
        prop_assert_eq!(decoded(data.len() as u32, &encode(&data)), data);
    }
}

fn bus_with_stream(size: u32, stream: &[u8]) -> FlatBus {
    let mut bus = FlatBus::new();
    bus.fill(DST, 0x100, 0xAA);
    bus.put(SRC, &((size << 8) | 0x10).to_le_bytes());
    bus.put(SRC + 4, stream);
    bus
}

#[test]
fn given_the_wram_form_when_decoding_then_every_output_byte_is_a_byte_write() {
    let mut bus = bus_with_stream(10, &OVERLAP);
    decode_wram(&mut bus, SRC, DST);
    assert_eq!(
        (
            bus.get(DST, 11),
            bus.writes.iter().all(|w| w.0 == 1),
            bus.writes.len()
        ),
        (b"ABABABABAB\xAA".to_vec(), true, 10)
    );
}

#[test]
fn given_the_vram_form_when_decoding_then_the_output_is_written_as_halfwords_low_byte_first() {
    let mut bus = bus_with_stream(10, &OVERLAP);
    decode_vram(&mut bus, SRC, DST);
    assert_eq!(
        (
            bus.get(DST, 11),
            bus.writes.iter().map(|w| w.0).collect::<Vec<_>>()
        ),
        (b"ABABABABAB\xAA".to_vec(), vec![2, 2, 2, 2, 2])
    );
}

#[test]
fn given_the_vram_form_with_a_block_at_distance_1_when_decoding_then_the_pending_byte_is_repeated()
{
    let mut bus = bus_with_stream(6, &[0x40, b'Z', 0x20, 0x00]);
    decode_vram(&mut bus, SRC, DST);
    assert_eq!(bus.get(DST, 7), b"ZZZZZZ\xAA".to_vec());
}

// TODO(hw-unverified): an odd size leaves the last byte of the 16-bit form unwritten
// (`hle-swi.md`, "Unverified").
#[test]
fn given_the_vram_form_with_an_odd_size_when_decoding_then_the_last_byte_is_not_written_as_the_note_models_unverified()
 {
    let mut bus = bus_with_stream(5, &OVERLAP);
    decode_vram(&mut bus, SRC, DST);
    assert_eq!(
        (bus.get(DST, 6), bus.writes.len()),
        (b"ABAB\xAA\xAA".to_vec(), 2)
    );
}

#[test]
fn given_the_wram_form_with_an_odd_size_when_decoding_then_the_last_byte_is_written() {
    let mut bus = bus_with_stream(5, &OVERLAP);
    decode_wram(&mut bus, SRC, DST);
    assert_eq!(bus.get(DST, 6), b"ABABA\xAA".to_vec());
}

#[test]
fn given_a_header_of_another_type_when_decoding_then_the_stream_is_decoded_anyway_as_the_note_models_unverified()
 {
    let mut bus = bus_with_stream(10, &OVERLAP);
    bus.put(SRC, &((10u32 << 8) | 0x20).to_le_bytes());
    decode_wram(&mut bus, SRC, DST);
    assert_eq!(bus.get(DST, 10), b"ABABABABAB".to_vec());
}

/// A stream that only counts its output, over an all-zero input, so a huge size costs no memory.
struct CountStream {
    count: u32,
}

impl Stream for CountStream {
    fn next(&mut self) -> u8 {
        0
    }
    fn put(&mut self, _byte: u8) {
        self.count += 1;
    }
    fn back(&mut self, _distance: u32) -> u8 {
        0
    }
}

#[test]
fn given_the_largest_size_when_the_source_is_all_zero_then_decoding_ends_after_that_many_bytes() {
    let mut stream = CountStream { count: 0 };
    decode(0x00FF_FFFF, &mut stream);
    assert_eq!(stream.count, 0x00FF_FFFF);
}
