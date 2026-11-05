// SPDX-License-Identifier: Apache-2.0
//! The ARM9 memory map through `Arm9View`. Expected values come from
//! `docs/hw/arm9-memory-map.md` and `docs/hw/vram-banks.md`, never from running the code.
//!
//! Scenarios live in harness functions that return what they observed (or the list of cases
//! that differed); a test body only asserts.

use super::*;
use crate::cpu::{Bus, Tcm};

const RAM: u32 = 0x0200_0000;
const RAM_PERIOD: u32 = 0x0040_0000;
const VRAMCNT_A: u32 = 0x0400_0240;
const VRAM_A: u32 = 0x0680_0000;
const VRAM_A_END: u32 = 0x0681_FFFF;
const ENABLE_MST0: u8 = 0x80;
const X: u32 = 0x1122_3344;
const Y: u32 = 0xA1B2_C3D4;

/// A TCM that is not there.
fn off() -> Tcm {
    Tcm {
        base: 0,
        size: 0,
        enabled: false,
        load_mode: false,
    }
}

/// An enabled TCM of virtual size `size` at `base`.
fn on(base: u32, size: u64) -> Tcm {
    Tcm {
        base,
        size,
        enabled: true,
        load_mode: false,
    }
}

/// An enabled TCM in load mode.
fn loading(base: u32, size: u64) -> Tcm {
    Tcm {
        load_mode: true,
        ..on(base, size)
    }
}

fn cfg(itcm: Tcm, dtcm: Tcm) -> TcmConfig {
    TcmConfig { itcm, dtcm }
}

fn plain() -> TcmConfig {
    cfg(off(), off())
}

fn with_vram() -> Memory {
    let mut m = Memory::new();
    Arm9View::new(&mut m, plain()).write8(VRAMCNT_A, ENABLE_MST0);
    m
}

// ---------------------------------------------------------------- main RAM

#[test]
fn given_a_word_written_to_main_ram_when_read_at_each_width_then_the_bytes_are_little_endian() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write32(RAM, X);
    let seen = (
        v.read32(RAM),
        v.read16(RAM),
        v.read16(RAM + 2),
        [
            v.read8(RAM),
            v.read8(RAM + 1),
            v.read8(RAM + 2),
            v.read8(RAM + 3),
        ],
    );
    assert_eq!(seen, (X, 0x3344, 0x1122, [0x44, 0x33, 0x22, 0x11]));
}

#[test]
fn given_narrow_writes_to_main_ram_when_read_as_a_word_then_each_lands_at_its_address() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write8(RAM, 0x11);
    v.write16(RAM + 2, 0x3344);
    v.write8(RAM + 1, 0x22);
    assert_eq!(v.read32(RAM), 0x3344_2211);
}

#[test]
fn given_a_word_write_at_an_address_ending_in_3_when_read_at_the_aligned_address_then_it_is_there()
{
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write32(RAM + 3, X);
    assert_eq!(
        (v.read32(RAM), v.read32(RAM + 1), v.read32(RAM + 4)),
        (X, X, 0)
    );
}

#[test]
fn given_a_halfword_at_an_odd_address_when_written_and_read_then_the_aligned_halfword_is_used() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write16(RAM + 0x11, 0xBEEF);
    assert_eq!(
        (
            v.read16(RAM + 0x10),
            v.read16(RAM + 0x11),
            v.read8(RAM + 0x10),
            v.read8(RAM + 0x11)
        ),
        (0xBEEF, 0xBEEF, 0xEF, 0xBE)
    );
}

#[test]
fn given_a_byte_write_at_an_odd_address_when_read_as_a_word_then_it_sits_at_that_byte() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write8(RAM + 0x21, 0x7A);
    assert_eq!(v.read32(RAM + 0x20), 0x0000_7A00);
}

#[test]
fn given_power_on_memory_when_main_ram_is_read_then_it_is_zero() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    let seen = [
        v.read32(RAM),
        v.read32(RAM + 0x1234),
        v.read32(RAM + RAM_PERIOD - 4),
        u32::from(v.read8(RAM + 0xFF_FFFF)),
    ];
    assert_eq!(seen, [0; 4]);
}

/// Writes `value` at `write_at`, then lists the read addresses that do not return it.
fn mirror_misses(write_at: u32, value: u32, read_at: &[u32]) -> Vec<u32> {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write32(write_at, value);
    read_at
        .iter()
        .copied()
        .filter(|&a| v.read32(a) != value)
        .collect()
}

#[test]
fn given_a_word_at_the_start_of_main_ram_when_read_through_the_mirrors_then_all_three_repeats_hold_it()
 {
    let misses = mirror_misses(
        RAM,
        X,
        &[RAM + RAM_PERIOD, RAM + 2 * RAM_PERIOD, RAM + 3 * RAM_PERIOD],
    );
    assert_eq!(misses, Vec::<u32>::new());
}

#[test]
fn given_a_word_at_the_end_of_the_first_4_mib_when_read_through_the_mirrors_then_every_repeat_holds_it()
 {
    let misses = mirror_misses(
        RAM + RAM_PERIOD - 4,
        X,
        &[
            RAM + 2 * RAM_PERIOD - 4,
            RAM + 3 * RAM_PERIOD - 4,
            0x02FF_FFFC,
        ],
    );
    assert_eq!(misses, Vec::<u32>::new());
}

#[test]
fn given_the_last_byte_before_0x0300_0000_when_written_then_it_reads_back_at_its_mirror_in_the_first_4_mib()
 {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write8(0x02FF_FFFF, 0x5A);
    assert_eq!(v.read8(0x023F_FFFF), 0x5A);
}

#[test]
fn given_a_write_at_the_first_address_after_the_mirror_block_when_main_ram_is_read_then_it_is_unchanged()
 {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write32(0x0300_0000, X);
    assert_eq!(
        (v.read32(0x0300_0000), v.read32(RAM), v.read32(0x02FF_FFFC)),
        (0, 0, 0)
    );
}

// ---------------------------------------------------------------- TCM

/// Base of the DTCM used over main RAM, and the physical sizes.
const DT: u32 = 0x0200_4000;
/// A base aligned to 64 KiB, for the DTCM regions larger than the physical size: GBATEK says the
/// base must be aligned to the size (`cp15.md`, "C9,C1,0 / C9,C1,1").
const DT64: u32 = 0x0201_0000;
const DTCM_PHYS: u64 = 0x4000;
const ITCM_PHYS: u64 = 0x8000;

#[test]
fn given_a_disabled_dtcm_when_written_and_read_then_main_ram_underneath_answers() {
    let mut m = Memory::new();
    let plain_read = {
        let mut v = Arm9View::new(&mut m, cfg(off(), off()));
        v.write32(DT, X);
        v.read32(DT)
    };
    // Enabling the (empty) DTCM hides the RAM word, so the first read really came from the RAM.
    let enabled_read = Arm9View::new(&mut m, cfg(off(), on(DT, DTCM_PHYS))).read32(DT);
    assert_eq!((plain_read, enabled_read), (X, 0));
}

/// (TCM before a write, TCM after, RAM underneath after the TCM is disabled).
fn dtcm_hides_ram() -> (u32, u32, u32) {
    let mut m = Memory::new();
    Arm9View::new(&mut m, plain()).write32(DT, Y);
    let (before, after) = {
        let mut v = Arm9View::new(&mut m, cfg(off(), on(DT, DTCM_PHYS)));
        let before = v.read32(DT);
        v.write32(DT, X);
        (before, v.read32(DT))
    };
    let under = Arm9View::new(&mut m, plain()).read32(DT);
    (before, after, under)
}

#[test]
fn given_an_enabled_dtcm_over_main_ram_when_written_then_the_tcm_answers_and_the_ram_is_untouched()
{
    assert_eq!(dtcm_hides_ram(), (0, X, Y));
}

/// (load in load mode, load after a store in load mode, load after leaving load mode,
/// load with the TCM disabled).
fn dtcm_load_mode() -> (u32, u32, u32, u32) {
    let mut m = Memory::new();
    Arm9View::new(&mut m, plain()).write32(DT, Y);
    let (a, b) = {
        let mut v = Arm9View::new(&mut m, cfg(off(), loading(DT, DTCM_PHYS)));
        let a = v.read32(DT);
        v.write32(DT, X);
        (a, v.read32(DT))
    };
    let c = Arm9View::new(&mut m, cfg(off(), on(DT, DTCM_PHYS))).read32(DT);
    let d = Arm9View::new(&mut m, plain()).read32(DT);
    (a, b, c, d)
}

#[test]
fn given_a_dtcm_in_load_mode_when_stored_to_then_loads_read_the_ram_and_the_store_lands_in_the_tcm()
{
    assert_eq!(dtcm_load_mode(), (Y, Y, X, Y));
}

/// The ITCM sits over unmapped space at 0, so a load-mode load there reads 0.
/// (load in load mode after a store, load after leaving load mode.)
fn itcm_load_mode() -> (u32, u32) {
    let mut m = Memory::new();
    let a = {
        let mut v = Arm9View::new(&mut m, cfg(loading(0, ITCM_PHYS), off()));
        v.write32(0x10, X);
        v.read32(0x10)
    };
    let b = Arm9View::new(&mut m, cfg(on(0, ITCM_PHYS), off())).read32(0x10);
    (a, b)
}

#[test]
fn given_an_itcm_in_load_mode_when_stored_to_then_the_data_appears_once_load_mode_ends() {
    assert_eq!(itcm_load_mode(), (0, X));
}

/// Writes `X` at base + 8 in a DTCM of `size`, then lists the addresses (relative to the base)
/// that do not read `X`.
fn dtcm_mirror_misses(size: u64, offsets: &[u32]) -> Vec<u32> {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, cfg(off(), on(DT64, size)));
    v.write32(DT64 + 8, X);
    offsets
        .iter()
        .copied()
        .filter(|&o| v.read32(DT64 + o) != X)
        .collect()
}

#[test]
fn given_a_dtcm_virtual_size_of_four_times_the_physical_when_read_through_the_repeats_then_each_holds_the_word()
 {
    let misses = dtcm_mirror_misses(0x1_0000, &[0x4008, 0x8008, 0xC008]);
    assert_eq!(misses, Vec::<u32>::new());
}

#[test]
fn given_a_dtcm_virtual_size_of_four_times_the_physical_when_a_word_is_stored_then_the_neighbour_in_a_repeat_stays_zero()
 {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, cfg(off(), on(DT64, 0x1_0000)));
    v.write32(DT64 + 8, X);
    assert_eq!(v.read32(DT64 + 0x400C), 0);
}

#[test]
fn given_a_dtcm_virtual_size_below_the_physical_when_the_address_passes_the_region_then_the_ram_answers()
 {
    let mut m = Memory::new();
    let (inside, outside) = {
        let mut v = Arm9View::new(&mut m, cfg(off(), on(DT, 0x1000)));
        v.write32(DT + 0xFFC, X);
        v.write32(DT + 0x1000, Y);
        (v.read32(DT + 0xFFC), v.read32(DT + 0x1000))
    };
    let mut v = Arm9View::new(&mut m, plain());
    let ram = (v.read32(DT + 0xFFC), v.read32(DT + 0x1000));
    assert_eq!((inside, outside, ram), (X, Y, (0, Y)));
}

#[test]
fn given_an_itcm_overlapping_a_dtcm_when_stored_to_then_the_itcm_takes_the_word() {
    // The ITCM region is 64 MiB: it covers the DTCM at DT, where its own offset is 0x4000
    // (DT mod 32 KiB) while the DTCM's offset is 0.
    let both = cfg(on(0, 0x400_0000), on(DT, DTCM_PHYS));
    let mut m = Memory::new();
    Arm9View::new(&mut m, both).write32(DT, X);
    let dtcm_only = Arm9View::new(&mut m, cfg(off(), on(DT, DTCM_PHYS))).read32(DT);
    let itcm_only = Arm9View::new(&mut m, cfg(on(0, 0x400_0000), off())).read32(DT);
    assert_eq!((dtcm_only, itcm_only), (0, X));
}

#[test]
fn given_a_byte_store_to_an_enabled_dtcm_when_read_as_a_word_then_the_byte_is_there_and_the_ram_is_untouched()
 {
    let mut m = Memory::new();
    let tcm_word = {
        let mut v = Arm9View::new(&mut m, cfg(off(), on(DT, DTCM_PHYS)));
        v.write8(DT + 1, 0x5A);
        v.read32(DT)
    };
    let ram_word = Arm9View::new(&mut m, plain()).read32(DT);
    assert_eq!((tcm_word, ram_word), (0x0000_5A00, 0));
}

/// (data read, fetch) at DT after the RAM holds `Y` and a data store of `X` hit the DTCM.
fn dtcm_data_vs_fetch() -> (u32, u32) {
    let mut m = Memory::new();
    Arm9View::new(&mut m, plain()).write32(DT, Y);
    let mut v = Arm9View::new(&mut m, cfg(off(), on(DT, DTCM_PHYS)));
    v.write32(DT, X);
    (v.read32(DT), v.fetch32(DT))
}

#[test]
fn given_a_word_in_the_dtcm_and_another_in_the_ram_below_when_fetched_then_the_ram_word_is_returned()
 {
    assert_eq!(dtcm_data_vs_fetch(), (X, Y));
}

#[test]
fn given_an_enabled_itcm_when_a_stored_word_is_fetched_then_the_itcm_serves_it() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, cfg(on(0, ITCM_PHYS), off()));
    v.write32(0x40, X);
    assert_eq!(v.fetch32(0x40), X);
}

#[test]
fn given_an_enabled_itcm_when_a_stored_word_is_fetched_as_halfwords_then_each_half_is_served() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, cfg(on(0, ITCM_PHYS), off()));
    v.write32(0x40, X);
    assert_eq!((v.fetch16(0x40), v.fetch16(0x42)), (0x3344, 0x1122));
}

#[test]
fn given_an_itcm_in_load_mode_holding_a_word_when_fetched_then_the_memory_underneath_answers() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, cfg(loading(0, ITCM_PHYS), off()));
    v.write32(0x40, X);
    assert_eq!(v.fetch32(0x40), 0);
}

#[test]
fn given_a_dtcm_ending_at_the_top_of_the_address_space_when_the_last_word_is_accessed_then_it_and_its_bytes_read_back()
 {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, cfg(off(), on(0xFFFF_C000, DTCM_PHYS)));
    v.write32(0xFFFF_FFFC, X);
    assert_eq!(
        (
            v.read32(0xFFFF_FFFC),
            v.read8(0xFFFF_FFFF),
            v.read16(0xFFFF_FFFE)
        ),
        (X, 0x11, 0x1122)
    );
}

// ---------------------------------------------------------------- VRAMCNT_A

/// Writes each register value and lists the (value, decoded state) pairs that differ from
/// `expected`.
fn vramcnt_decode_failures(expected: &[(u8, VramBankState)]) -> Vec<(u8, VramBankState)> {
    expected
        .iter()
        .filter_map(|&(byte, want)| {
            let mut m = Memory::new();
            Arm9View::new(&mut m, plain()).write8(VRAMCNT_A, byte);
            let got = m.vram_a();
            (got != want).then_some((byte, got))
        })
        .collect()
}

fn state(enabled: bool, mst: u8, offset: u8) -> VramBankState {
    VramBankState {
        enabled,
        mst,
        offset,
    }
}

#[test]
fn given_vramcnt_a_values_when_written_then_enable_mst_and_offset_decode_from_bits_7_0_1_and_3_4() {
    let table = [
        (0x00, state(false, 0, 0)),
        (0x80, state(true, 0, 0)),
        (0x81, state(true, 1, 0)),
        (0x83, state(true, 3, 0)),
        (0x88, state(true, 0, 1)),
        (0x93, state(true, 3, 2)),
        (0x9B, state(true, 3, 3)),
        (0x03, state(false, 3, 0)),
        // Bit 2 is not used by bank A and is dropped.
        (0x84, state(true, 0, 0)),
        (0xFF, state(true, 3, 3)),
        (0x64, state(false, 0, 0)),
    ];
    assert_eq!(vramcnt_decode_failures(&table), vec![]);
}

#[test]
fn given_power_on_memory_when_vram_a_is_decoded_then_it_is_disabled() {
    assert_eq!(Memory::new().vram_a(), state(false, 0, 0));
}

#[test]
fn given_vramcnt_a_written_when_read_at_each_width_then_it_reads_zero() {
    let mut m = with_vram();
    let mut v = Arm9View::new(&mut m, plain());
    assert_eq!(
        (v.read8(VRAMCNT_A), v.read16(VRAMCNT_A), v.read32(VRAMCNT_A)),
        (0, 0, 0)
    );
}

#[test]
fn given_vramcnt_a_enabled_when_a_halfword_with_a_zero_low_byte_is_written_there_then_the_bank_is_disabled()
 {
    let mut m = with_vram();
    Arm9View::new(&mut m, plain()).write16(VRAMCNT_A, 0xFF00);
    assert_eq!(m.vram_a(), state(false, 0, 0));
}

#[test]
fn given_a_word_written_at_vramcnt_a_when_decoded_then_only_the_byte_at_that_address_counts() {
    let mut m = Memory::new();
    Arm9View::new(&mut m, plain()).write32(VRAMCNT_A, 0xFFFF_FF80);
    assert_eq!(m.vram_a(), state(true, 0, 0));
}

#[test]
fn given_vramcnt_a_enabled_when_the_next_byte_is_written_then_bank_a_is_unchanged() {
    let mut m = with_vram();
    Arm9View::new(&mut m, plain()).write8(VRAMCNT_A + 1, 0x00);
    assert_eq!(m.vram_a(), state(true, 0, 0));
}

#[test]
fn given_vram_a_enabled_with_mst_0_when_halfwords_and_words_are_stored_then_they_read_back() {
    let mut m = with_vram();
    let mut v = Arm9View::new(&mut m, plain());
    v.write16(VRAM_A, 0x1234);
    v.write32(VRAM_A_END - 3, Y);
    assert_eq!(
        (
            v.read16(VRAM_A),
            v.read32(VRAM_A_END - 3),
            v.read16(VRAM_A_END - 1)
        ),
        (0x1234, Y, 0xA1B2)
    );
}

#[test]
fn given_vram_a_enabled_when_the_first_and_last_bytes_are_stored_through_words_then_byte_reads_find_them()
 {
    let mut m = with_vram();
    let mut v = Arm9View::new(&mut m, plain());
    v.write32(VRAM_A, 0xAABB_CCDD);
    v.write32(VRAM_A_END - 3, X);
    assert_eq!((v.read8(VRAM_A), v.read8(VRAM_A_END)), (0xDD, 0x11));
}

#[test]
fn given_vram_a_enabled_when_the_addresses_just_outside_the_window_are_stored_then_nothing_is_mapped_there()
 {
    let mut m = with_vram();
    let mut v = Arm9View::new(&mut m, plain());
    v.write32(VRAM_A - 4, X);
    v.write32(VRAM_A_END + 1, Y);
    assert_eq!(
        (
            v.read32(VRAM_A - 4),
            v.read32(VRAM_A_END + 1),
            v.read32(VRAM_A),
            v.read32(VRAM_A_END - 3)
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn given_vram_a_enabled_when_a_byte_is_stored_then_the_bank_is_unchanged() {
    let mut m = with_vram();
    let mut v = Arm9View::new(&mut m, plain());
    v.write32(VRAM_A + 8, X);
    v.write8(VRAM_A + 8, 0xFF);
    v.write8(VRAM_A_END, 0xFF);
    assert_eq!((v.read32(VRAM_A + 8), v.read8(VRAM_A_END)), (X, 0));
}

/// Writes each control byte, stores a word at the window, then enables bank A properly and
/// lists the control bytes whose pre-enable read or later read is not zero.
fn unmapped_bank_failures(control: &[u8]) -> Vec<u8> {
    control
        .iter()
        .copied()
        .filter(|&byte| {
            let mut m = Memory::new();
            let mut v = Arm9View::new(&mut m, plain());
            v.write8(VRAMCNT_A, byte);
            v.write32(VRAM_A, X);
            let while_set = v.read32(VRAM_A);
            v.write8(VRAMCNT_A, ENABLE_MST0);
            let after_enable = v.read32(VRAM_A);
            while_set != 0 || after_enable != 0
        })
        .collect()
}

#[test]
fn given_a_bank_that_is_disabled_or_has_mst_other_than_0_when_stored_to_then_reads_give_zero_and_the_store_is_lost()
 {
    // 0x00 disabled; 0x01 and 0x03 disabled with MST; 0x81, 0x83, 0x87 enabled with MST != 0.
    assert_eq!(
        unmapped_bank_failures(&[0x00, 0x01, 0x03, 0x81, 0x83, 0x87, 0x9B]),
        Vec::<u8>::new()
    );
}

/// Checks `Memory::region` for each (address, region) pair; returns the pairs that differ.
fn region_failures(m: &Memory, expected: &[(u32, Region)]) -> Vec<(u32, Region)> {
    expected
        .iter()
        .filter_map(|&(addr, want)| {
            let got = m.region(addr);
            (got != want).then_some((addr, got))
        })
        .collect()
}

#[test]
fn given_power_on_memory_when_regions_are_looked_up_then_only_main_ram_is_mapped() {
    let table = [
        (0x0000_0000, Region::Unmapped),
        (RAM, Region::MainRam),
        (0x023F_FFFF, Region::MainRam),
        (0x0240_0000, Region::MainRam),
        (0x0280_0000, Region::MainRam),
        (0x02FF_FFFF, Region::MainRam),
        (0x0300_0000, Region::Unmapped),
        (VRAMCNT_A, Region::Unmapped),
        (VRAM_A, Region::Unmapped),
        (0xFFFF_0000, Region::Unmapped),
    ];
    assert_eq!(region_failures(&Memory::new(), &table), vec![]);
}

#[test]
fn given_vramcnt_a_enabled_when_regions_are_looked_up_then_exactly_the_128_kib_window_is_vram_a() {
    let table = [
        (0x067F_FFFF, Region::Unmapped),
        (VRAM_A, Region::VramA),
        (VRAM_A + 0x3FFF, Region::VramA),
        (VRAM_A + 0x4000, Region::VramA),
        (VRAM_A_END, Region::VramA),
        (0x0682_0000, Region::Unmapped),
        (RAM, Region::MainRam),
    ];
    assert_eq!(region_failures(&with_vram(), &table), vec![]);
}

#[test]
fn given_vramcnt_a_enabled_then_disabled_when_the_window_is_looked_up_then_it_is_unmapped_at_once()
{
    let mut m = with_vram();
    Arm9View::new(&mut m, plain()).write8(VRAMCNT_A, 0x00);
    let table = [(VRAM_A, Region::Unmapped), (VRAM_A_END, Region::Unmapped)];
    assert_eq!(region_failures(&m, &table), vec![]);
}

#[test]
fn given_vramcnt_a_enabled_then_set_to_mst_1_when_the_window_is_looked_up_then_it_is_unmapped_at_once()
 {
    let mut m = with_vram();
    Arm9View::new(&mut m, plain()).write8(VRAMCNT_A, 0x81);
    assert_eq!(m.region(VRAM_A), Region::Unmapped);
}

// ---------------------------------------------------------------- unmapped

const UNMAPPED: [u32; 9] = [
    0x0000_0000,
    0x0300_0000,
    0x0400_0000,
    0x0500_0000,
    0x0600_0000,
    0x0700_0000,
    0x0800_0000,
    0xFFFF_0000,
    0xFFFF_FFFC,
];

/// Stores all-ones at every width to each unmapped address, then returns the addresses that
/// read non-zero at any width, plus a marker (`u32::MAX`) if main RAM changed.
fn unmapped_failures() -> Vec<u32> {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    let mut bad = Vec::new();
    for &a in &UNMAPPED {
        v.write32(a, u32::MAX);
        v.write16(a, u16::MAX);
        v.write8(a, u8::MAX);
        let nonzero = v.read32(a) != 0
            || v.read16(a) != 0
            || v.read8(a) != 0
            || v.fetch32(a) != 0
            || v.fetch16(a) != 0;
        if nonzero {
            bad.push(a);
        }
    }
    if v.read32(RAM) != 0 || v.read32(0x02FF_FFFC) != 0 {
        bad.push(u32::MAX);
    }
    bad
}

#[test]
fn given_unmapped_addresses_when_written_and_read_at_every_width_then_reads_give_zero_and_nothing_leaks()
 {
    assert_eq!(unmapped_failures(), Vec::<u32>::new());
}

#[test]
fn given_the_last_byte_of_the_address_space_when_read_and_written_then_it_reads_zero() {
    let mut m = Memory::new();
    let mut v = Arm9View::new(&mut m, plain());
    v.write8(0xFFFF_FFFF, 0xFF);
    v.write16(0xFFFF_FFFF, 0xFFFF);
    v.write32(0xFFFF_FFFF, u32::MAX);
    assert_eq!(
        (
            v.read8(0xFFFF_FFFF),
            v.read16(0xFFFF_FFFF),
            v.read32(0xFFFF_FFFF)
        ),
        (0, 0, 0)
    );
}
