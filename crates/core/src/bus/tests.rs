// SPDX-License-Identifier: Apache-2.0
//! The ARM9 memory map through `Arm9View`. Expected values come from
//! `docs/hw/arm9-memory-map.md` and `docs/hw/vram-banks.md`, never from running the code.
//!
//! Scenarios live in harness functions that return what they observed (or the list of cases
//! that differed); a test body only asserts.

use super::bench::Bench;
use super::*;
use crate::cpu::{Bus, Tcm};
use crate::io::irq::Requests;

const RAM: u32 = 0x0200_0000;
const RAM_PERIOD: u32 = 0x0040_0000;
const VRAMCNT_A: u32 = 0x0400_0240;
const VRAM_A: u32 = 0x0680_0000;
const VRAM_A_END: u32 = 0x0681_FFFF;
const PALETTE: u32 = 0x0500_0000;
const PALETTE_SIZE: u32 = 0x800;
const OAM: u32 = 0x0700_0000;
const OAM_SIZE: u32 = 0x800;
const BG_A: u32 = 0x0600_0000;
const BG_SLOT: u32 = 0x2_0000;
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

fn with_vram() -> Bench {
    let mut m = Bench::new();
    m.arm9(plain()).write8(VRAMCNT_A, ENABLE_MST0);
    m
}

// ---------------------------------------------------------------- main RAM

#[test]
fn given_a_word_written_to_main_ram_when_read_at_each_width_then_the_bytes_are_little_endian() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
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
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write8(RAM, 0x11);
    v.write16(RAM + 2, 0x3344);
    v.write8(RAM + 1, 0x22);
    assert_eq!(v.read32(RAM), 0x3344_2211);
}

#[test]
fn given_a_word_write_at_an_address_ending_in_3_when_read_at_the_aligned_address_then_it_is_there()
{
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(RAM + 3, X);
    assert_eq!(
        (v.read32(RAM), v.read32(RAM + 1), v.read32(RAM + 4)),
        (X, X, 0)
    );
}

#[test]
fn given_a_halfword_at_an_odd_address_when_written_and_read_then_the_aligned_halfword_is_used() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
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
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write8(RAM + 0x21, 0x7A);
    assert_eq!(v.read32(RAM + 0x20), 0x0000_7A00);
}

#[test]
fn given_power_on_memory_when_main_ram_is_read_then_it_is_zero() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
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
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
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
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write8(0x02FF_FFFF, 0x5A);
    assert_eq!(v.read8(0x023F_FFFF), 0x5A);
}

#[test]
fn given_a_write_at_the_first_address_after_the_mirror_block_when_main_ram_is_read_then_it_is_unchanged()
 {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    // 0300_0000h is shared WRAM since #22 (`wramcnt.md`); main RAM must not see the write.
    v.write32(0x0300_0000, X);
    assert_eq!((v.read32(RAM), v.read32(0x02FF_FFFC)), (0, 0));
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
    let mut m = Bench::new();
    let plain_read = {
        let mut v = m.arm9(cfg(off(), off()));
        v.write32(DT, X);
        v.read32(DT)
    };
    // Enabling the (empty) DTCM hides the RAM word, so the first read really came from the RAM.
    let enabled_read = m.arm9(cfg(off(), on(DT, DTCM_PHYS))).read32(DT);
    assert_eq!((plain_read, enabled_read), (X, 0));
}

/// (TCM before a write, TCM after, RAM underneath after the TCM is disabled).
fn dtcm_hides_ram() -> (u32, u32, u32) {
    let mut m = Bench::new();
    m.arm9(plain()).write32(DT, Y);
    let (before, after) = {
        let mut v = m.arm9(cfg(off(), on(DT, DTCM_PHYS)));
        let before = v.read32(DT);
        v.write32(DT, X);
        (before, v.read32(DT))
    };
    let under = m.arm9(plain()).read32(DT);
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
    let mut m = Bench::new();
    m.arm9(plain()).write32(DT, Y);
    let (a, b) = {
        let mut v = m.arm9(cfg(off(), loading(DT, DTCM_PHYS)));
        let a = v.read32(DT);
        v.write32(DT, X);
        (a, v.read32(DT))
    };
    let c = m.arm9(cfg(off(), on(DT, DTCM_PHYS))).read32(DT);
    let d = m.arm9(plain()).read32(DT);
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
    let mut m = Bench::new();
    let a = {
        let mut v = m.arm9(cfg(loading(0, ITCM_PHYS), off()));
        v.write32(0x10, X);
        v.read32(0x10)
    };
    let b = m.arm9(cfg(on(0, ITCM_PHYS), off())).read32(0x10);
    (a, b)
}

#[test]
fn given_an_itcm_in_load_mode_when_stored_to_then_the_data_appears_once_load_mode_ends() {
    assert_eq!(itcm_load_mode(), (0, X));
}

/// Writes `X` at base + 8 in a DTCM of `size`, then lists the addresses (relative to the base)
/// that do not read `X`.
fn dtcm_mirror_misses(size: u64, offsets: &[u32]) -> Vec<u32> {
    let mut m = Bench::new();
    let mut v = m.arm9(cfg(off(), on(DT64, size)));
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
    let mut m = Bench::new();
    let mut v = m.arm9(cfg(off(), on(DT64, 0x1_0000)));
    v.write32(DT64 + 8, X);
    assert_eq!(v.read32(DT64 + 0x400C), 0);
}

#[test]
fn given_a_dtcm_virtual_size_below_the_physical_when_the_address_passes_the_region_then_the_ram_answers()
 {
    let mut m = Bench::new();
    let (inside, outside) = {
        let mut v = m.arm9(cfg(off(), on(DT, 0x1000)));
        v.write32(DT + 0xFFC, X);
        v.write32(DT + 0x1000, Y);
        (v.read32(DT + 0xFFC), v.read32(DT + 0x1000))
    };
    let mut v = m.arm9(plain());
    let ram = (v.read32(DT + 0xFFC), v.read32(DT + 0x1000));
    assert_eq!((inside, outside, ram), (X, Y, (0, Y)));
}

#[test]
fn given_an_itcm_overlapping_a_dtcm_when_stored_to_then_the_itcm_takes_the_word() {
    // The ITCM region is 64 MiB: it covers the DTCM at DT, where its own offset is 0x4000
    // (DT mod 32 KiB) while the DTCM's offset is 0.
    let both = cfg(on(0, 0x400_0000), on(DT, DTCM_PHYS));
    let mut m = Bench::new();
    m.arm9(both).write32(DT, X);
    let dtcm_only = m.arm9(cfg(off(), on(DT, DTCM_PHYS))).read32(DT);
    let itcm_only = m.arm9(cfg(on(0, 0x400_0000), off())).read32(DT);
    assert_eq!((dtcm_only, itcm_only), (0, X));
}

#[test]
fn given_a_byte_store_to_an_enabled_dtcm_when_read_as_a_word_then_the_byte_is_there_and_the_ram_is_untouched()
 {
    let mut m = Bench::new();
    let tcm_word = {
        let mut v = m.arm9(cfg(off(), on(DT, DTCM_PHYS)));
        v.write8(DT + 1, 0x5A);
        v.read32(DT)
    };
    let ram_word = m.arm9(plain()).read32(DT);
    assert_eq!((tcm_word, ram_word), (0x0000_5A00, 0));
}

/// (data read, fetch) at DT after the RAM holds `Y` and a data store of `X` hit the DTCM.
fn dtcm_data_vs_fetch() -> (u32, u32) {
    let mut m = Bench::new();
    m.arm9(plain()).write32(DT, Y);
    let mut v = m.arm9(cfg(off(), on(DT, DTCM_PHYS)));
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
    let mut m = Bench::new();
    let mut v = m.arm9(cfg(on(0, ITCM_PHYS), off()));
    v.write32(0x40, X);
    assert_eq!(v.fetch32(0x40), X);
}

#[test]
fn given_an_enabled_itcm_when_a_stored_word_is_fetched_as_halfwords_then_each_half_is_served() {
    let mut m = Bench::new();
    let mut v = m.arm9(cfg(on(0, ITCM_PHYS), off()));
    v.write32(0x40, X);
    assert_eq!((v.fetch16(0x40), v.fetch16(0x42)), (0x3344, 0x1122));
}

#[test]
fn given_an_itcm_in_load_mode_holding_a_word_when_fetched_then_the_memory_underneath_answers() {
    let mut m = Bench::new();
    let mut v = m.arm9(cfg(loading(0, ITCM_PHYS), off()));
    v.write32(0x40, X);
    assert_eq!(v.fetch32(0x40), 0);
}

#[test]
fn given_a_dtcm_ending_at_the_top_of_the_address_space_when_the_last_word_is_accessed_then_it_and_its_bytes_read_back()
 {
    let mut m = Bench::new();
    let mut v = m.arm9(cfg(off(), on(0xFFFF_C000, DTCM_PHYS)));
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
            let mut m = Bench::new();
            m.arm9(plain()).write8(VRAMCNT_A, byte);
            let got = m.vram_a();
            (got != want).then_some((byte, got))
        })
        .collect()
}

fn state(enabled: bool, mst: u8, offset: u8, mapping: VramMapping) -> VramBankState {
    VramBankState {
        enabled,
        mst,
        offset,
        mapping,
    }
}

const OFF: VramMapping = VramMapping::Disabled;
const LCDC: VramMapping = VramMapping::Lcdc;

#[test]
fn given_vramcnt_a_values_when_written_then_enable_mst_and_offset_decode_from_bits_7_0_1_and_3_4() {
    let table = [
        (0x00, state(false, 0, 0, OFF)),
        (0x80, state(true, 0, 0, LCDC)),
        (0x81, state(true, 1, 0, VramMapping::BgA { offset: 0 })),
        (0x83, state(true, 3, 0, VramMapping::Texture { slot: 0 })),
        (0x88, state(true, 0, 1, LCDC)),
        (0x93, state(true, 3, 2, VramMapping::Texture { slot: 2 })),
        (0x9B, state(true, 3, 3, VramMapping::Texture { slot: 3 })),
        (0x03, state(false, 3, 0, OFF)),
        // Bit 2 is not used by bank A and is dropped.
        (0x84, state(true, 0, 0, LCDC)),
        (0xFF, state(true, 3, 3, VramMapping::Texture { slot: 3 })),
        (0x64, state(false, 0, 0, OFF)),
    ];
    assert_eq!(vramcnt_decode_failures(&table), vec![]);
}

#[test]
fn given_power_on_memory_when_vram_a_is_decoded_then_it_is_disabled() {
    assert_eq!(Bench::new().vram_a(), state(false, 0, 0, OFF));
}

#[test]
fn given_vramcnt_a_written_when_read_at_each_width_then_it_reads_zero() {
    let mut m = with_vram();
    let mut v = m.arm9(plain());
    assert_eq!(
        (v.read8(VRAMCNT_A), v.read16(VRAMCNT_A), v.read32(VRAMCNT_A)),
        (0, 0, 0)
    );
}

#[test]
fn given_vramcnt_a_enabled_when_a_halfword_with_a_zero_low_byte_is_written_there_then_the_bank_is_disabled()
 {
    let mut m = with_vram();
    m.arm9(plain()).write16(VRAMCNT_A, 0xFF00);
    assert_eq!(m.vram_a(), state(false, 0, 0, OFF));
}

#[test]
fn given_a_word_written_at_vramcnt_a_when_decoded_then_only_the_byte_at_that_address_counts() {
    let mut m = Bench::new();
    m.arm9(plain()).write32(VRAMCNT_A, 0xFFFF_FF80);
    assert_eq!(m.vram_a(), state(true, 0, 0, LCDC));
}

#[test]
fn given_vramcnt_a_enabled_when_the_next_byte_is_written_then_bank_a_is_unchanged() {
    let mut m = with_vram();
    m.arm9(plain()).write8(VRAMCNT_A + 1, 0x00);
    assert_eq!(m.vram_a(), state(true, 0, 0, LCDC));
}

#[test]
fn given_vram_a_enabled_with_mst_0_when_halfwords_and_words_are_stored_then_they_read_back() {
    let mut m = with_vram();
    let mut v = m.arm9(plain());
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
    let mut v = m.arm9(plain());
    v.write32(VRAM_A, 0xAABB_CCDD);
    v.write32(VRAM_A_END - 3, X);
    assert_eq!((v.read8(VRAM_A), v.read8(VRAM_A_END)), (0xDD, 0x11));
}

#[test]
fn given_vram_a_enabled_when_the_addresses_just_outside_the_window_are_stored_then_nothing_is_mapped_there()
 {
    let mut m = with_vram();
    let mut v = m.arm9(plain());
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
    let mut v = m.arm9(plain());
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
            let mut m = Bench::new();
            let mut v = m.arm9(plain());
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
fn region_failures(m: &Bench, expected: &[(u32, Region)]) -> Vec<(u32, Region)> {
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
        // Shared WRAM, owned by the ARM9 at power-on (`wramcnt.md`).
        (0x0300_0000, Region::SharedWram),
        // Palette RAM needs no control register (`palette-ram.md`, "Layout").
        (PALETTE, Region::Palette),
        (VRAMCNT_A, Region::Unmapped),
        (VRAM_A, Region::Unmapped),
        // The synthetic BIOS (`hle-swi.md`, "The BIOS area").
        (0xFFFF_0000, Region::Bios),
    ];
    assert_eq!(region_failures(&Bench::new(), &table), vec![]);
}

#[test]
fn given_vramcnt_a_enabled_when_regions_are_looked_up_then_exactly_the_128_kib_window_is_vram_a() {
    let table = [
        (0x067F_FFFF, Region::Unmapped),
        (VRAM_A, Region::Vram),
        (VRAM_A + 0x3FFF, Region::Vram),
        (VRAM_A + 0x4000, Region::Vram),
        (VRAM_A_END, Region::Vram),
        (0x0682_0000, Region::Unmapped),
        (RAM, Region::MainRam),
    ];
    assert_eq!(region_failures(&with_vram(), &table), vec![]);
}

#[test]
fn given_vramcnt_a_enabled_then_disabled_when_the_window_is_looked_up_then_it_is_unmapped_at_once()
{
    let mut m = with_vram();
    m.arm9(plain()).write8(VRAMCNT_A, 0x00);
    let table = [(VRAM_A, Region::Unmapped), (VRAM_A_END, Region::Unmapped)];
    assert_eq!(region_failures(&m, &table), vec![]);
}

#[test]
fn given_vramcnt_a_enabled_then_set_to_mst_1_when_the_window_is_looked_up_then_it_is_unmapped_at_once()
 {
    let mut m = with_vram();
    m.arm9(plain()).write8(VRAMCNT_A, 0x81);
    assert_eq!(m.region(VRAM_A), Region::Unmapped);
}

// ------------------------------------------------------ palette RAM (#28)
//
// Expected values come from `docs/hw/palette-ram.md` ("Layout", "Behavior", "Edge cases"), never
// from running the code. A test that pins a line of its "Unverified" section says so in its name.

#[test]
fn given_palette_ram_when_words_and_halfwords_are_stored_then_they_read_back_little_endian() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(PALETTE, X);
    v.write16(PALETTE + 4, 0xBEEF);
    assert_eq!(
        (
            v.read32(PALETTE),
            v.read16(PALETTE),
            v.read16(PALETTE + 2),
            v.read8(PALETTE),
            v.read8(PALETTE + 3),
            v.read16(PALETTE + 4)
        ),
        (X, 0x3344, 0x1122, 0x44, 0x11, 0xBEEF)
    );
}

#[test]
fn given_palette_ram_when_a_byte_is_stored_then_the_palette_is_unchanged() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(PALETTE + 8, X);
    v.write8(PALETTE + 8, 0xFF);
    v.write8(PALETTE + 0x7FF, 0xFF);
    assert_eq!((v.read32(PALETTE + 8), v.read8(PALETTE + 0x7FF)), (X, 0));
}

#[test]
fn given_power_on_memory_when_palette_ram_is_read_then_it_is_zero_as_the_note_models_unverified() {
    // TODO(hw-unverified): palette-ram.md "Unverified", the power-on contents.
    let m = Bench::new();
    assert_eq!(
        (
            m.read32(&plain(), PALETTE),
            m.read32(&plain(), PALETTE + 0x7FC)
        ),
        (0, 0)
    );
}

/// Stores `value` at `write_at`, and returns the addresses of `read_at` that do not read it.
fn palette_mirror_misses(write_at: u32, value: u32, read_at: &[u32]) -> Vec<u32> {
    let mut m = Bench::new();
    m.write32(&plain(), write_at, value);
    read_at
        .iter()
        .copied()
        .filter(|&at| m.read32(&plain(), at) != value)
        .collect()
}

#[test]
fn given_a_word_in_palette_ram_when_read_through_the_2_kib_mirrors_then_every_repeat_holds_it() {
    // TODO(hw-unverified): palette-ram.md "Unverified", the mirror through the 16 MiB block.
    let at = PALETTE + 0x10;
    let repeats = [
        at + PALETTE_SIZE,
        at + 2 * PALETTE_SIZE,
        at + 0x4000,
        at + 0x10_0000,
        0x05FF_F810,
    ];
    assert_eq!(palette_mirror_misses(at, X, &repeats), Vec::<u32>::new());
}

#[test]
fn given_a_word_stored_through_a_palette_mirror_when_read_at_the_first_repeat_then_it_is_there() {
    assert_eq!(
        palette_mirror_misses(0x05FF_F7FC, Y, &[PALETTE + 0x7FC]),
        Vec::<u32>::new()
    );
}

#[test]
fn given_a_word_stored_at_each_end_of_the_palette_block_when_the_neighbours_are_read_then_they_stay_zero()
 {
    // The first address after the 16 MiB block is the BG window, not palette RAM.
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(0x05FF_FFFC, X);
    v.write32(BG_A, Y);
    assert_eq!(
        (
            v.read32(0x05FF_FFFC),
            v.read32(PALETTE + 0x7FC),
            v.read32(BG_A)
        ),
        (X, X, 0)
    );
}

#[test]
fn given_engine_bs_half_of_the_palette_when_a_word_is_stored_then_engine_as_half_is_unchanged() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(PALETTE, X);
    v.write32(PALETTE + 0x400, Y);
    assert_eq!((v.read32(PALETTE), v.read32(PALETTE + 0x400)), (X, Y));
}

#[test]
fn given_palette_ram_when_the_arm7_stores_and_reads_then_nothing_is_there_for_it() {
    // TODO(hw-unverified): palette-ram.md "Unverified", the ARM7 reads 0 and drops writes.
    let mut m = Bench::new();
    m.write32_arm7(0, PALETTE, X);
    assert_eq!(
        (
            m.read32_arm7(0, PALETTE),
            m.read32(&plain(), PALETTE),
            m.region7(PALETTE)
        ),
        (0, 0, Region::Unmapped)
    );
}

#[test]
fn given_vramcnt_a_changed_when_the_palette_is_looked_up_then_it_is_still_palette_ram() {
    let mut m = with_vram();
    m.arm9(plain()).write8(VRAMCNT_A, 0x00);
    let table = [
        (PALETTE, Region::Palette),
        (0x05FF_FFFF, Region::Palette),
        (0x0600_0000, Region::Unmapped),
    ];
    assert_eq!(region_failures(&m, &table), vec![]);
}

// ------------------------------------------------------ OAM (#31)
//
// Expected values come from `docs/hw/obj.md` ("Registers and memory"), never from running the
// code. A test that pins a line of its "Unverified" section says so in its name.

#[test]
fn given_oam_when_words_and_halfwords_are_stored_then_they_read_back_little_endian() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(OAM, X);
    v.write16(OAM + 4, 0xBEEF);
    assert_eq!(
        (
            v.read32(OAM),
            v.read16(OAM),
            v.read16(OAM + 2),
            v.read8(OAM),
            v.read8(OAM + 3),
            v.read16(OAM + 4)
        ),
        (X, 0x3344, 0x1122, 0x44, 0x11, 0xBEEF)
    );
}

#[test]
fn given_oam_when_a_byte_is_stored_then_oam_is_unchanged() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(OAM + 8, X);
    v.write8(OAM + 8, 0xFF);
    v.write8(OAM + 0x7FF, 0xFF);
    assert_eq!((v.read32(OAM + 8), v.read8(OAM + 0x7FF)), (X, 0));
}

#[test]
fn given_power_on_memory_when_oam_is_read_then_it_is_zero_as_the_note_models_unverified() {
    // TODO(hw-unverified): obj.md "Unverified", the power-on contents.
    let m = Bench::new();
    assert_eq!(
        (m.read32(&plain(), OAM), m.read32(&plain(), OAM + 0x7FC)),
        (0, 0)
    );
}

/// Stores `value` at `write_at`, and returns the addresses of `read_at` that do not read it.
fn oam_mirror_misses(write_at: u32, value: u32, read_at: &[u32]) -> Vec<u32> {
    let mut m = Bench::new();
    m.write32(&plain(), write_at, value);
    read_at
        .iter()
        .copied()
        .filter(|&at| m.read32(&plain(), at) != value)
        .collect()
}

#[test]
fn given_a_word_in_oam_when_read_through_the_2_kib_mirrors_then_every_repeat_holds_it() {
    // TODO(hw-unverified): obj.md "Unverified", the mirror through the 16 MiB block.
    let at = OAM + 0x10;
    let repeats = [
        at + OAM_SIZE,
        at + 2 * OAM_SIZE,
        at + 0x4000,
        at + 0x10_0000,
        0x07FF_F810,
    ];
    assert_eq!(oam_mirror_misses(at, X, &repeats), Vec::<u32>::new());
}

#[test]
fn given_a_word_stored_through_an_oam_mirror_when_read_at_the_first_repeat_then_it_is_there() {
    assert_eq!(
        oam_mirror_misses(0x07FF_F7FC, Y, &[OAM + 0x7FC]),
        Vec::<u32>::new()
    );
}

#[test]
fn given_a_word_stored_at_the_end_of_the_oam_block_when_the_next_block_is_read_then_it_is_unmapped()
{
    // The first address after the 16 MiB block is the GBA slot, which is unmapped.
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(0x07FF_FFFC, X);
    v.write32(0x0800_0000, Y);
    assert_eq!(
        (
            v.read32(0x07FF_FFFC),
            v.read32(OAM + 0x7FC),
            v.read32(0x0800_0000)
        ),
        (X, X, 0)
    );
}

#[test]
fn given_engine_bs_half_of_oam_when_a_word_is_stored_then_engine_as_half_is_unchanged() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(OAM, X);
    v.write32(OAM + 0x400, Y);
    assert_eq!((v.read32(OAM), v.read32(OAM + 0x400)), (X, Y));
}

#[test]
fn given_oam_when_the_arm7_stores_and_reads_then_nothing_is_there_for_it() {
    // TODO(hw-unverified): obj.md "Unverified", the ARM7 reads 0 and drops writes.
    let mut m = Bench::new();
    m.write32_arm7(0, OAM, X);
    assert_eq!(
        (
            m.read32_arm7(0, OAM),
            m.read32(&plain(), OAM),
            m.region7(OAM)
        ),
        (0, 0, Region::Unmapped)
    );
}

#[test]
fn given_vramcnt_a_changed_when_oam_is_looked_up_then_it_is_still_oam() {
    let mut m = with_vram();
    m.arm9(plain()).write8(VRAMCNT_A, 0x00);
    let table = [
        (0x06FF_FFFF, Region::Unmapped),
        (OAM, Region::Oam),
        (0x07FF_FFFF, Region::Oam),
        (0x0800_0000, Region::Unmapped),
    ];
    assert_eq!(region_failures(&m, &table), vec![]);
}

// ------------------------------------------------------ VRAM bank A as engine A BG (#28)
//
// Expected values come from `docs/hw/vram-banks.md` ("Behavior", "Engine A BG VRAM (MST 1)",
// "Edge cases"), never from running the code.

/// A bank A that is enabled with MST 1 and the offset `ofs`.
fn with_bg_vram(ofs: u8) -> Bench {
    let mut m = Bench::new();
    m.arm9(plain())
        .write8(VRAMCNT_A, ENABLE_MST0 | 1 | (ofs << 3));
    m
}

#[test]
fn given_bank_a_with_mst_1_and_offset_0_when_regions_are_looked_up_then_exactly_the_first_128_kib_of_the_bg_window_is_vram_a()
 {
    let table = [
        (0x05FF_FFFF, Region::Palette),
        (BG_A, Region::Vram),
        (BG_A + 0x3FFF, Region::Vram),
        (BG_A + 0x4000, Region::Vram),
        (BG_A + BG_SLOT - 1, Region::Vram),
        (BG_A + BG_SLOT, Region::Unmapped),
        (VRAM_A, Region::Unmapped),
    ];
    assert_eq!(region_failures(&with_bg_vram(0), &table), vec![]);
}

/// The offsets whose BG window is not exactly the 128 KiB slot `ofs * 20000h` into the window.
fn bg_slot_failures() -> Vec<u8> {
    (0u8..4)
        .filter(|&ofs| {
            let m = with_bg_vram(ofs);
            let base = BG_A + u32::from(ofs) * BG_SLOT;
            let inside =
                m.region(base) == Region::Vram && m.region(base + BG_SLOT - 1) == Region::Vram;
            let outside = m.region(base.wrapping_sub(1)) != Region::Vram
                && m.region(base + BG_SLOT) != Region::Vram;
            !(inside && outside)
        })
        .collect()
}

#[test]
fn given_bank_a_with_mst_1_when_the_offset_is_each_value_then_the_window_is_the_slot_it_names() {
    assert_eq!(bg_slot_failures(), Vec::<u8>::new());
}

#[test]
fn given_bank_a_with_mst_1_when_words_and_halfwords_are_stored_then_they_read_back() {
    let mut m = with_bg_vram(0);
    let mut v = m.arm9(plain());
    v.write16(BG_A, 0x1234);
    v.write32(BG_A + BG_SLOT - 4, Y);
    assert_eq!(
        (
            v.read16(BG_A),
            v.read32(BG_A + BG_SLOT - 4),
            v.read8(BG_A + BG_SLOT - 1)
        ),
        (0x1234, Y, 0xA1)
    );
}

#[test]
fn given_bank_a_with_mst_1_when_a_byte_is_stored_then_the_bank_is_unchanged_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): vram-banks.md "Unverified", byte stores to BG VRAM.
    let mut m = with_bg_vram(0);
    let mut v = m.arm9(plain());
    v.write32(BG_A + 8, X);
    v.write8(BG_A + 8, 0xFF);
    assert_eq!(v.read32(BG_A + 8), X);
}

#[test]
fn given_a_word_stored_in_lcdc_mode_when_the_bank_moves_to_mst_1_then_the_same_bytes_are_in_the_bg_window()
 {
    let mut m = with_vram();
    m.write32(&plain(), VRAM_A + 0x24, X);
    m.write8(&plain(), VRAMCNT_A, 0x81);
    assert_eq!(m.read32(&plain(), BG_A + 0x24), X);
}

#[test]
fn given_a_word_stored_in_the_bg_window_when_the_bank_moves_back_to_lcdc_then_the_same_bytes_are_there()
 {
    let mut m = with_bg_vram(0);
    m.write32(&plain(), BG_A + 0x40, Y);
    m.write8(&plain(), VRAMCNT_A, 0x80);
    assert_eq!(m.read32(&plain(), VRAM_A + 0x40), Y);
}

#[test]
fn given_a_word_stored_at_offset_0_when_the_offset_moves_to_1_then_the_same_bytes_are_one_slot_up()
{
    let mut m = with_bg_vram(0);
    m.write32(&plain(), BG_A + 0x100, X);
    m.write8(&plain(), VRAMCNT_A, 0x89);
    assert_eq!(
        (
            m.read32(&plain(), BG_A + BG_SLOT + 0x100),
            m.read32(&plain(), BG_A + 0x100)
        ),
        (X, 0)
    );
}

#[test]
fn given_bank_a_with_mst_1_when_the_lcdc_window_is_read_then_nothing_is_there_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): vram-banks.md "Unverified", a bank with MST other than 0 at LCDC.
    let mut m = with_bg_vram(0);
    m.write32(&plain(), BG_A, X);
    assert_eq!(
        (m.read32(&plain(), VRAM_A), m.region(VRAM_A)),
        (0, Region::Unmapped)
    );
}

/// The control values after which the BG window still shows bank A.
fn bg_window_still_mapped(controls: &[u8]) -> Vec<u8> {
    controls
        .iter()
        .copied()
        .filter(|&control| {
            let mut m = with_bg_vram(0);
            m.write32(&plain(), BG_A, X);
            m.write8(&plain(), VRAMCNT_A, control);
            m.region(BG_A) != Region::Unmapped || m.read32(&plain(), BG_A) != 0
        })
        .collect()
}

#[test]
fn given_bank_a_with_mst_1_when_disabled_or_set_to_mst_2_or_3_then_the_bg_window_is_unmapped() {
    // 0x01 disabled with MST 1; 0x82, 0x83 and 0x8A enabled with the OBJ and texture mappings
    // (`TODO(phase4)`, `TODO(phase7)`).
    assert_eq!(
        bg_window_still_mapped(&[0x01, 0x82, 0x83, 0x8A]),
        Vec::<u8>::new()
    );
}

#[test]
fn given_bank_a_with_mst_1_when_the_arm7_reads_the_bg_window_then_it_is_not_there() {
    let mut m = with_bg_vram(0);
    m.write32(&plain(), BG_A, X);
    assert_eq!(
        (m.read32_arm7(0, BG_A), m.region7(BG_A)),
        (0, Region::Unmapped)
    );
}

// ---------------------------------------------------------------- unmapped

const UNMAPPED: [u32; 6] = [
    0x0000_0000,
    // An I/O port no register occupies; 0x0400_0000 is DISPCNT since #20.
    0x0400_0090,
    // The engine A BG window while no bank is mapped there; the palette RAM and OAM are always mapped.
    0x0600_0000,
    0x0800_0000,
    0xFFFF_0000,
    0xFFFF_FFFC,
];

/// Stores all-ones at every width to each unmapped address, then returns the addresses that
/// read non-zero at any width, plus a marker (`u32::MAX`) if main RAM changed.
fn unmapped_failures() -> Vec<u32> {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
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
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
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

// ------------------------------------------------------- shared WRAM and WRAMCNT (#22)
//
// Expected values come from `docs/hw/wramcnt.md` ("Registers", "Behavior", "Edge cases"), never
// from running the code. A test that pins a line of its "Unverified" section says so in its name.

const WRAM: u32 = 0x0300_0000;
const WRAM7_START: u32 = 0x0380_0000;
/// WRAMCNT, an ARM9 register (`wramcnt.md`, "Registers").
const WRAMCNT: u32 = 0x0400_0247;
/// WRAMSTAT, the ARM7's read-only view of it.
const WRAMSTAT: u32 = 0x0400_0241;
const MARK: u8 = 0x5A;

/// Which CPU's view of the memory an access goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Who {
    Nine,
    Seven,
}
use Who::{Nine, Seven};

fn read_as(m: &Bench, who: Who, addr: u32) -> u8 {
    match who {
        Nine => m.read8(&plain(), addr),
        Seven => m.read8_arm7(0, addr),
    }
}

fn write_as(m: &mut Bench, who: Who, addr: u32, value: u8) {
    match who {
        Nine => m.write8(&plain(), addr, value),
        Seven => m.write8_arm7(0, addr, value),
    }
}

/// WRAMCNT is written by the ARM9, as a byte at its own address.
fn set_mode(m: &mut Bench, mode: u8) {
    m.write8(&plain(), WRAMCNT, mode);
}

fn in_mode(mode: u8) -> Bench {
    let mut m = Bench::new();
    set_mode(&mut m, mode);
    m
}

/// One scenario: (mode, writer, write address, reader, read address, whether the reader sees
/// the marker). Each runs on a fresh memory, so a reader that sees the marker saw the same
/// physical byte.
type Visible = (u8, Who, u32, Who, u32, bool);

/// The cases that differ from the expected visibility.
fn visibility_failures(cases: &[Visible]) -> Vec<Visible> {
    cases
        .iter()
        .copied()
        .filter(|&(mode, writer, at, reader, from, seen)| {
            let mut m = in_mode(mode);
            write_as(&mut m, writer, at, MARK);
            (read_as(&m, reader, from) == MARK) != seen
        })
        .collect()
}

#[test]
fn given_mode_0_when_one_cpu_writes_then_only_the_arm9_and_the_arm7s_own_wram_see_it() {
    let cases = [
        // The ARM9 owns all 32 KiB and the window repeats it every 32 KiB.
        (0, Nine, WRAM, Nine, WRAM, true),
        (0, Nine, WRAM, Nine, WRAM + 0x8000, true),
        (0, Nine, WRAM + 0x4000, Nine, WRAM + 0xC000, true),
        (0, Nine, WRAM + 0x7FFF, Nine, WRAM + 0xFFFF, true),
        (0, Nine, WRAM + 0x7FFF, Nine, WRAM + 0x8000, false),
        (0, Nine, WRAM, Nine, 0x037F_8000, true),
        (0, Nine, WRAM, Nine, 0x03FF_8000, true),
        (0, Nine, WRAM, Nine, WRAM7_START, true),
        // The ARM7 owns none of it: it sees its own WRAM there instead.
        (0, Nine, WRAM, Seven, WRAM, false),
        (0, Nine, WRAM, Seven, WRAM7_START, false),
        (0, Seven, WRAM, Nine, WRAM, false),
        (0, Seven, WRAM7_START, Nine, WRAM, false),
        // 0300_0000h shows the ARM7's 64 KiB WRAM, which is also at 0380_0000h and repeats
        // every 64 KiB through both ranges.
        (0, Seven, WRAM, Seven, WRAM7_START, true),
        (0, Seven, WRAM7_START, Seven, WRAM, true),
        (0, Seven, WRAM, Seven, WRAM + 0x1_0000, true),
        (0, Seven, WRAM, Seven, 0x037F_0000, true),
        (0, Seven, WRAM7_START, Seven, 0x03FF_0000, true),
        (0, Seven, WRAM, Seven, WRAM + 0x8000, false),
        (0, Seven, 0x037F_8000, Seven, 0x0380_8000, true),
        (0, Seven, 0x037F_8000, Seven, WRAM7_START, false),
        (0, Seven, 0x037F_FFFF, Seven, 0x0380_FFFF, true),
    ];
    assert_eq!(visibility_failures(&cases), Vec::<Visible>::new());
}

#[test]
fn given_mode_1_when_one_cpu_writes_then_each_cpu_stays_in_its_own_16_kib_half() {
    let cases = [
        // The ARM9 has the second half, the ARM7 the first; each repeats every 16 KiB.
        (1, Nine, WRAM, Nine, WRAM + 0x4000, true),
        (1, Nine, WRAM, Nine, 0x03FF_C000, true),
        (1, Nine, WRAM + 0x3FFF, Nine, WRAM + 0x7FFF, true),
        (1, Nine, WRAM + 0x3FFF, Nine, WRAM + 0x4000, false),
        (1, Nine, WRAM, Seven, WRAM, false),
        (1, Seven, WRAM, Nine, WRAM, false),
        (1, Seven, WRAM, Seven, WRAM + 0x8000, true),
        (1, Seven, WRAM + 0x4000, Seven, WRAM, true),
        (1, Seven, 0x037F_8000, Seven, WRAM, true),
        // 0380_0000h is the ARM7's own WRAM whatever the mode.
        (1, Seven, WRAM, Seven, WRAM7_START, false),
        (1, Seven, WRAM7_START, Nine, WRAM, false),
    ];
    assert_eq!(visibility_failures(&cases), Vec::<Visible>::new());
}

#[test]
fn given_mode_2_when_one_cpu_writes_then_each_cpu_stays_in_its_own_16_kib_half() {
    let cases = [
        // The ARM9 has the first half, the ARM7 the second.
        (2, Nine, WRAM, Nine, WRAM + 0xC000, true),
        (2, Nine, WRAM + 0x3FFF, Nine, WRAM + 0x4000, false),
        (2, Nine, WRAM, Seven, WRAM, false),
        (2, Seven, WRAM, Nine, WRAM, false),
        (2, Seven, WRAM + 0x3FFF, Seven, WRAM + 0x7FFF, true),
        (2, Seven, WRAM + 0x3FFF, Seven, WRAM + 0x4000, false),
    ];
    assert_eq!(visibility_failures(&cases), Vec::<Visible>::new());
}

#[test]
fn given_mode_3_when_one_cpu_writes_then_the_arm7_owns_all_32_kib_and_the_arm9_gets_nothing() {
    let cases = [
        (3, Seven, WRAM, Seven, WRAM + 0x8000, true),
        (3, Seven, WRAM + 0x7FFF, Seven, 0x037F_FFFF, true),
        (3, Seven, WRAM, Nine, WRAM, false),
        // The ARM9's write is dropped, so the ARM7 never sees it.
        (3, Nine, WRAM, Seven, WRAM, false),
        (3, Nine, WRAM, Nine, WRAM, false),
        // 0380_0000h stays the ARM7's separate 64 KiB WRAM.
        (3, Seven, WRAM, Seven, WRAM7_START, false),
        (3, Seven, WRAM7_START, Seven, 0x0381_0000, true),
        (3, Seven, WRAM7_START, Seven, WRAM, false),
    ];
    assert_eq!(visibility_failures(&cases), Vec::<Visible>::new());
}

/// Offsets of the four marked bytes in the 32 KiB: the first and last byte of each half, so
/// the `+3FFFh` / `+4000h` boundary decides which half a byte is in.
const MARKED: [(u32, u8); 4] = [(0, 0xA1), (0x3FFF, 0xA2), (0x4000, 0xB1), (0x7FFF, 0xB2)];

/// A mismatch: (mode, CPU, address, expected, got).
type ReadDiff = (u8, Who, u32, u8, u8);

/// Fills the four marked bytes under mode 0 through the ARM9, switches to each row's mode, and
/// reads `addrs` through the row's CPU. Bytes keep their place across a mode change, so what a
/// CPU reads is the half it now owns (`wramcnt.md`, "Changing the mode").
fn read_failures(addrs: &[u32], rows: &[(u8, Who, &[u8])]) -> Vec<ReadDiff> {
    rows.iter()
        .flat_map(|&(mode, who, expected)| {
            let mut m = Bench::new();
            MARKED
                .iter()
                .for_each(|&(offset, value)| m.write8(&plain(), WRAM + offset, value));
            set_mode(&mut m, mode);
            addrs
                .iter()
                .zip(expected)
                .filter_map(|(&addr, &want)| {
                    let got = read_as(&m, who, addr);
                    (got != want).then_some((mode, who, addr, want, got))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn given_bytes_written_under_mode_0_when_each_mode_is_set_then_each_cpu_reads_the_half_it_owns() {
    // Addresses: the window start, the end of the first half, the start and end of the second,
    // then the two repetitions at +8000h and +C000h.
    let addrs = [
        WRAM,
        WRAM + 0x3FFF,
        WRAM + 0x4000,
        WRAM + 0x7FFF,
        WRAM + 0x8000,
        WRAM + 0xC000,
    ];
    let (a1, a2, b1, b2) = (0xA1, 0xA2, 0xB1, 0xB2);
    let rows: [(u8, Who, &[u8]); 8] = [
        (0, Nine, &[a1, a2, b1, b2, a1, b1]),
        (0, Seven, &[0; 6]),
        // Mode 1: the ARM9 has the second half (bytes 4000h..7FFFh of the 32 KiB).
        (1, Nine, &[b1, b2, b1, b2, b1, b1]),
        (1, Seven, &[a1, a2, a1, a2, a1, a1]),
        (2, Nine, &[a1, a2, a1, a2, a1, a1]),
        (2, Seven, &[b1, b2, b1, b2, b1, b1]),
        (3, Nine, &[0; 6]),
        (3, Seven, &[a1, a2, b1, b2, a1, b1]),
    ];
    assert_eq!(read_failures(&addrs, &rows), Vec::<ReadDiff>::new());
}

#[test]
fn given_bytes_written_under_mode_0_when_each_mode_is_set_then_the_window_ends_show_the_same_halves()
 {
    // 037F_8000h is the ARM7's last repetition of the shared allocation; 0380_0000h is the
    // ARM7's own WRAM but still the ARM9's shared window (`wramcnt.md`, "The 037F_8000h mirror").
    let addrs = [0x037F_8000, 0x037F_FFFF, WRAM7_START, 0x03FF_FFFF];
    let (a1, a2, b1, b2) = (0xA1, 0xA2, 0xB1, 0xB2);
    let rows: [(u8, Who, &[u8]); 8] = [
        (0, Nine, &[a1, b2, a1, b2]),
        (0, Seven, &[0; 4]),
        (1, Nine, &[b1, b2, b1, b2]),
        (1, Seven, &[a1, a2, 0, 0]),
        (2, Nine, &[a1, a2, a1, a2]),
        (2, Seven, &[b1, b2, 0, 0]),
        (3, Nine, &[0; 4]),
        (3, Seven, &[a1, b2, 0, 0]),
    ];
    assert_eq!(read_failures(&addrs, &rows), Vec::<ReadDiff>::new());
}

/// A mismatch: (mode, the 16 KiB page address, ARM9 region, ARM7 region) as (expected, got).
type RegionDiff = (u8, u32, (Region, Region), (Region, Region));

/// The pages whose `region()` / `region7()` differ from the row.
fn wram_region_failures(addrs: &[u32], rows: &[(u8, [Region; 6], [Region; 6])]) -> Vec<RegionDiff> {
    rows.iter()
        .flat_map(|&(mode, nine, seven)| {
            let m = in_mode(mode);
            addrs
                .iter()
                .enumerate()
                .filter_map(|(i, &addr)| {
                    let got = (m.region(addr), m.region7(addr));
                    ((nine[i], seven[i]) != got).then_some((mode, addr, (nine[i], seven[i]), got))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn given_each_mode_when_regions_are_asked_then_both_views_report_the_page_owner() {
    use Region::{SharedWram as S, Unmapped as U, Wram7 as W};
    let addrs = [
        WRAM,
        WRAM + 0x4000,
        0x037F_8000,
        WRAM7_START,
        0x03FF_C000,
        0x0400_0000,
    ];
    let rows = [
        (0, [S, S, S, S, S, U], [W, W, W, W, W, U]),
        (1, [S, S, S, S, S, U], [S, S, S, W, W, U]),
        (2, [S, S, S, S, S, U], [S, S, S, W, W, U]),
        (3, [U, U, U, U, U, U], [S, S, S, W, W, U]),
    ];
    assert_eq!(
        wram_region_failures(&addrs, &rows),
        Vec::<RegionDiff>::new()
    );
}

#[test]
fn given_a_power_on_memory_when_main_ram_is_asked_for_its_region_then_both_views_say_main_ram() {
    let m = Bench::new();
    assert_eq!(
        (
            m.region(0x0200_0000),
            m.region7(0x0200_0000),
            m.region7(0x02FF_C000)
        ),
        (Region::MainRam, Region::MainRam, Region::MainRam)
    );
}

#[test]
fn given_a_power_on_memory_when_wramcnt_is_read_then_mode_0_gives_the_arm9_all_32_kib() {
    // Power-on mode 0 is chosen, not stated (`wramcnt.md`, "Unverified").
    let m = Bench::new();
    assert_eq!(
        (
            m.wramcnt(),
            m.read8(&plain(), WRAMCNT),
            m.read8_arm7(0, WRAMSTAT)
        ),
        (
            WramState {
                mode: 0,
                arm9: WramShare::All32,
                arm7: WramShare::None
            },
            0,
            0
        )
    );
}

/// The modes whose decoded shares differ from the table in `wramcnt.md`, "WRAMCNT bits".
fn decode_failures() -> Vec<(u8, WramState)> {
    use WramShare::{All32, First16, None as Nothing, Second16};
    let table = [
        (0, All32, Nothing),
        (1, Second16, First16),
        (2, First16, Second16),
        (3, Nothing, All32),
    ];
    table
        .iter()
        .filter_map(|&(mode, arm9, arm7)| {
            let got = in_mode(mode).wramcnt();
            (got != WramState { mode, arm9, arm7 }).then_some((mode, got))
        })
        .collect()
}

#[test]
fn given_each_mode_when_wramcnt_is_decoded_then_the_shares_match_the_allocation_table() {
    assert_eq!(decode_failures(), Vec::<(u8, WramState)>::new());
}

/// (written value, expected mode) pairs whose three observers (ARM9 read of WRAMCNT, ARM7 read
/// of WRAMSTAT, `wramcnt().mode`) do not all say the expected mode.
fn readback_failures(rows: &[(u8, u8)]) -> Vec<(u8, u8, [u8; 3])> {
    rows.iter()
        .filter_map(|&(written, mode)| {
            let m = in_mode(written);
            let seen = [
                m.read8(&plain(), WRAMCNT),
                m.read8_arm7(0, WRAMSTAT),
                m.wramcnt().mode,
            ];
            (seen != [mode; 3]).then_some((written, mode, seen))
        })
        .collect()
}

#[test]
fn given_bits_2_to_7_set_in_a_write_when_read_back_then_only_the_mode_survives() {
    let rows = [
        (0x00, 0),
        (0x01, 1),
        (0x02, 2),
        (0x03, 3),
        (0x04, 0),
        (0x07, 3),
        (0x80, 0),
        (0x81, 1),
        (0xFC, 0),
        (0xFE, 2),
        (0xFF, 3),
    ];
    assert_eq!(readback_failures(&rows), Vec::<(u8, u8, [u8; 3])>::new());
}

#[test]
fn given_wramcnt_set_by_the_arm9_when_the_arm7_writes_0400_0247_then_the_mode_is_unchanged() {
    let mut m = in_mode(1);
    m.write8_arm7(0, WRAMCNT, 3);
    m.write16_arm7(0, WRAMCNT & !1, 0x0300);
    m.write32_arm7(0, WRAMCNT & !3, 0x0300_0000);
    assert_eq!(
        (m.read8(&plain(), WRAMCNT), m.read8_arm7(0, WRAMSTAT)),
        (1, 1)
    );
}

#[test]
fn given_wramcnt_set_by_the_arm9_when_the_arm7_writes_wramstat_then_the_mode_is_unchanged() {
    let mut m = in_mode(2);
    m.write8_arm7(0, WRAMSTAT, 3);
    assert_eq!(
        (m.read8_arm7(0, WRAMSTAT), m.read8(&plain(), WRAMCNT)),
        (2, 2)
    );
}

#[test]
fn given_mode_1_when_a_byte_is_written_at_0400_0246_then_it_reaches_vramcnt_g_and_the_mode_stays() {
    // 0400_0246h is VRAMCNT_G, not WRAMCNT: the write enables bank G's register only.
    let mut m = in_mode(1);
    m.write8(&plain(), 0x0400_0246, 3);
    assert_eq!((m.read8(&plain(), WRAMCNT), m.vram_a().enabled), (1, false));
}

#[test]
fn given_a_halfword_write_at_0400_0246_when_the_high_byte_is_2_then_wramcnt_becomes_2() {
    // Low byte 03h is VRAMCNT_G, high byte 02h is WRAMCNT.
    let mut m = Bench::new();
    m.write16(&plain(), 0x0400_0246, 0x0203);
    assert_eq!((m.read8(&plain(), WRAMCNT), m.wramcnt().mode), (2, 2));
}

#[test]
fn given_a_halfword_write_at_0400_0246_when_the_low_byte_is_3_and_the_high_byte_0_then_wramcnt_is_0()
 {
    let mut m = in_mode(1);
    m.write16(&plain(), 0x0400_0246, 0x0003);
    assert_eq!(m.read8(&plain(), WRAMCNT), 0);
}

#[test]
fn given_a_word_write_at_0400_0244_when_the_top_byte_is_3_then_wramcnt_becomes_3() {
    // The four registers 0400_0244h..0400_0247h: only the top byte is WRAMCNT.
    let mut m = Bench::new();
    m.write32(&plain(), 0x0400_0244, 0x03FF_FFFF);
    assert_eq!((m.read8(&plain(), WRAMCNT), m.vram_a().enabled), (3, false));
}

#[test]
fn given_a_word_write_at_0400_0244_when_the_top_byte_is_0_then_wramcnt_is_cleared() {
    let mut m = in_mode(2);
    m.write32(&plain(), 0x0400_0244, 0x0000_0003);
    assert_eq!(m.read8(&plain(), WRAMCNT), 0);
}

#[test]
fn given_vramcnt_a_enabled_when_wramcnt_is_written_then_the_vram_bank_is_unchanged() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0240, 0x80);
    let before = m.vram_a();
    set_mode(&mut m, 3);
    assert_eq!((m.vram_a(), m.region(0x0680_0000)), (before, Region::Vram));
}

#[test]
fn given_wramcnt_written_when_vramcnt_a_is_written_then_wramcnt_is_unchanged() {
    let mut m = in_mode(2);
    m.write8(&plain(), 0x0400_0240, 0x80);
    assert_eq!(
        (m.read8(&plain(), WRAMCNT), m.region(WRAM)),
        (2, Region::SharedWram)
    );
}

#[test]
fn given_a_byte_in_the_arm9s_share_when_the_mode_moves_it_to_the_arm7_then_the_old_mapping_is_gone()
{
    let mut m = Bench::new();
    m.write8(&plain(), WRAM, MARK);
    let before = (m.region(WRAM), m.region7(WRAM), m.read8_arm7(0, WRAM));
    set_mode(&mut m, 3);
    // Mode 3: the ARM9 window is empty and the ARM7 now reads the byte at the same address.
    let after = (
        m.region(WRAM),
        m.region7(WRAM),
        m.read8(&plain(), WRAM),
        m.read8_arm7(0, WRAM),
    );
    assert_eq!(
        (before, after),
        (
            (Region::SharedWram, Region::Wram7, 0),
            (Region::Unmapped, Region::SharedWram, 0, MARK)
        )
    );
}

#[test]
fn given_mode_3_after_mode_0_when_mode_0_returns_then_the_arm9_byte_is_back_and_the_arm7_sees_its_own_wram()
 {
    let mut m = Bench::new();
    m.write8(&plain(), WRAM, MARK);
    set_mode(&mut m, 3);
    m.write8_arm7(0, WRAM + 1, 0x33);
    set_mode(&mut m, 0);
    // The shared bytes did not move or clear; the ARM7 write at +1 went to the shared WRAM
    // under mode 3, so the ARM9 reads it now.
    assert_eq!(
        (
            m.read8(&plain(), WRAM),
            m.read8(&plain(), WRAM + 1),
            m.read8_arm7(0, WRAM),
            m.read8_arm7(0, WRAM + 1),
            m.region7(WRAM)
        ),
        (MARK, 0x33, 0, 0, Region::Wram7)
    );
}

#[test]
fn given_a_mode_cycle_0_1_2_3_when_each_cpu_reads_the_first_byte_then_the_contents_never_clear() {
    let mut m = Bench::new();
    m.write8(&plain(), WRAM, 0xA1);
    let mut seen = Vec::new();
    seen.push(m.read8(&plain(), WRAM));
    set_mode(&mut m, 1);
    seen.push(m.read8_arm7(0, WRAM));
    set_mode(&mut m, 2);
    seen.push(m.read8(&plain(), WRAM));
    set_mode(&mut m, 3);
    seen.push(m.read8_arm7(0, WRAM));
    set_mode(&mut m, 0);
    seen.push(m.read8(&plain(), WRAM));
    assert_eq!(seen, [0xA1; 5]);
}

#[test]
fn given_mode_3_when_the_arm9_writes_a_word_then_the_write_is_dropped_and_reads_are_zero_as_the_note_models_unverified()
 {
    // TODO(hw-unverified): GBATEK says "undefined data" for the empty window
    // (`wramcnt.md`, "Unverified").
    let mut m = in_mode(3);
    m.write32(&plain(), WRAM, 0x1122_3344);
    m.write16(&plain(), 0x03FF_FFFE, 0x5566);
    m.write8(&plain(), WRAM + 0x4000, 0x77);
    assert_eq!(
        (
            m.read32(&plain(), WRAM),
            m.read16(&plain(), 0x03FF_FFFE),
            m.read8(&plain(), WRAM + 0x4000),
            m.read32_arm7(0, WRAM),
            m.read8_arm7(0, WRAM + 0x4000)
        ),
        (0, 0, 0, 0, 0)
    );
}

#[test]
fn given_a_word_in_the_shared_wram_when_read_at_every_width_then_the_bytes_are_little_endian() {
    let mut m = Bench::new();
    m.write32(&plain(), WRAM + 0x7FFC, X);
    // The last word of the 32 KiB, seen through the next repetition.
    assert_eq!(
        (
            m.read32(&plain(), WRAM + 0xFFFC),
            m.read16(&plain(), WRAM + 0xFFFC),
            m.read16(&plain(), WRAM + 0xFFFE),
            m.read8(&plain(), WRAM + 0xFFFF)
        ),
        (X, 0x3344, 0x1122, 0x11)
    );
}

#[test]
fn given_mode_3_when_the_arm7_writes_a_halfword_at_the_end_of_the_window_then_it_reads_back_at_the_start()
 {
    let mut m = in_mode(3);
    m.write16_arm7(0, 0x037F_FFFE, 0xBEEF);
    assert_eq!(
        (
            m.read16_arm7(0, WRAM + 0x7FFE),
            m.read32_arm7(0, 0x037F_FFFC),
            m.read8_arm7(0, 0x037F_FFFF)
        ),
        (0xBEEF, 0xBEEF_0000, 0xBE)
    );
}

#[test]
fn given_mode_3_when_the_arm7_writes_across_the_96_kib_block_then_each_range_keeps_its_own_bytes() {
    // 037F_8000h..037F_FFFFh is the shared WRAM, 0380_0000h..0380_FFFFh the ARM7's own: adjacent
    // addresses, separate memories (`wramcnt.md`, "The 037F_8000h mirror").
    let mut m = in_mode(3);
    m.write8_arm7(0, 0x037F_8000, 0x11);
    m.write8_arm7(0, 0x037F_FFFF, 0x22);
    m.write8_arm7(0, WRAM7_START, 0x33);
    m.write8_arm7(0, 0x0380_FFFF, 0x44);
    let seen = [
        m.read8_arm7(0, 0x037F_8000),
        m.read8_arm7(0, WRAM),
        m.read8_arm7(0, 0x037F_FFFF),
        m.read8_arm7(0, WRAM + 0x7FFF),
        m.read8_arm7(0, WRAM7_START),
        m.read8_arm7(0, 0x0381_0000),
        m.read8_arm7(0, 0x0380_FFFF),
        m.read8_arm7(0, 0x03FF_FFFF),
    ];
    assert_eq!(seen, [0x11, 0x11, 0x22, 0x22, 0x33, 0x33, 0x44, 0x44]);
}

#[test]
fn given_the_arm7s_own_wram_when_every_mode_is_set_then_its_byte_never_moves() {
    let mut m = Bench::new();
    m.write8_arm7(0, WRAM7_START + 0x10, 0x77);
    let mut seen = Vec::new();
    seen.push(m.read8_arm7(0, WRAM7_START + 0x10));
    set_mode(&mut m, 1);
    seen.push(m.read8_arm7(0, WRAM7_START + 0x10));
    set_mode(&mut m, 2);
    seen.push(m.read8_arm7(0, WRAM7_START + 0x10));
    set_mode(&mut m, 3);
    seen.push(m.read8_arm7(0, WRAM7_START + 0x10));
    assert_eq!(seen, [0x77; 4]);
}

#[test]
fn given_mode_0_when_the_arm9_fetches_from_shared_wram_then_it_gets_the_stored_opcode() {
    let mut m = Bench::new();
    m.write32(&plain(), WRAM + 0x100, 0xE1A0_0000);
    m.write16(&plain(), WRAM + 0x204, 0x46C0);
    assert_eq!(
        (
            m.fetch32(&plain(), WRAM + 0x100),
            m.fetch16(&plain(), WRAM + 0x204)
        ),
        (0xE1A0_0000, 0x46C0)
    );
}

#[test]
fn given_mode_3_when_the_arm9_fetches_from_the_empty_window_then_it_reads_zero() {
    // TODO(hw-unverified): an empty ARM9 window reads 0 here (`wramcnt.md`, "Unverified").
    let mut m = Bench::new();
    m.write32(&plain(), WRAM + 0x100, 0xE1A0_0000);
    set_mode(&mut m, 3);
    assert_eq!(m.fetch32(&plain(), WRAM + 0x100), 0);
}

#[test]
fn given_an_opcode_in_the_arm7s_share_when_the_arm9_changes_the_mode_then_the_next_arm7_fetch_sees_the_change()
 {
    // A mode change is seen by the ARM7 on its next instruction fetch (`wramcnt.md`, "Edge
    // cases").
    let mut m = in_mode(3);
    m.write32_arm7(0, WRAM, 0xE1A0_1234);
    let before = (m.fetch32_arm7(0, WRAM), m.fetch16_arm7(0, WRAM));
    set_mode(&mut m, 0);
    assert_eq!(
        (before, m.fetch32_arm7(0, WRAM), m.fetch16_arm7(0, WRAM)),
        ((0xE1A0_1234, 0x1234), 0, 0)
    );
}

// ---------------------------------------------------------------- interrupt controller registers
//
// Expected values come from `docs/hw/interrupts.md` ("Registers", "Behavior", "Wider and
// narrower accesses", "Edge cases"): IME, IE and IF sit at 0400_0208h, 0400_0210h and
// 0400_0214h on both CPUs, each CPU with its own registers.

const IME: u32 = 0x0400_0208;
const IE: u32 = 0x0400_0210;
const IF: u32 = 0x0400_0214;

/// The registers a CPU sees as words at IME, IE and IF.
fn arm9_regs(m: &Bench) -> [u32; 3] {
    [
        m.read32(&plain(), IME),
        m.read32(&plain(), IE),
        m.read32(&plain(), IF),
    ]
}

fn arm7_regs(m: &Bench) -> [u32; 3] {
    [
        m.read32_arm7(0, IME),
        m.read32_arm7(0, IE),
        m.read32_arm7(0, IF),
    ]
}

#[test]
fn given_power_on_memory_when_the_interrupt_registers_are_read_then_both_cpus_see_zero() {
    let m = Bench::new();
    assert_eq!((arm9_regs(&m), arm7_regs(&m)), ([0; 3], [0; 3]));
}

#[test]
fn given_all_ones_written_to_ime_and_ie_on_the_arm9_when_read_as_words_then_ime_is_1_and_ie_is_the_arm9_mask()
 {
    let mut m = Bench::new();
    m.write32(&plain(), IME, 0xFFFF_FFFF);
    m.write32(&plain(), IE, 0xFFFF_FFFF);
    assert_eq!(arm9_regs(&m), [1, 0x003F_3F7F, 0]);
}

#[test]
fn given_all_ones_written_to_ime_and_ie_on_the_arm7_when_read_as_words_then_ime_is_1_and_ie_is_the_arm7_mask()
 {
    let mut m = Bench::new();
    m.write32_arm7(0, IME, 0xFFFF_FFFF);
    m.write32_arm7(0, IE, 0xFFFF_FFFF);
    assert_eq!(arm7_regs(&m), [1, 0x01DF_3FFF, 0]);
}

#[test]
fn given_an_arm7_write_to_ie_when_the_controller_is_read_then_the_write_landed() {
    // Before the controller existed every ARM7 I/O write was dropped.
    let mut m = Bench::new();
    m.write32_arm7(0, IE, 0x0000_0101);
    assert_eq!(m.irq7().ie(), 0x0000_0101);
}

#[test]
fn given_an_arm9_ie_write_when_the_arm7_reads_its_ie_then_it_sees_zero() {
    let mut m = Bench::new();
    m.write32(&plain(), IE, 0x0000_0001);
    m.write32(&plain(), IME, 1);
    assert_eq!(
        (arm7_regs(&m), m.irq9().ie(), m.irq7().ie()),
        ([0; 3], 1, 0)
    );
}

#[test]
fn given_an_arm7_ie_write_when_the_arm9_reads_its_ie_then_it_sees_zero() {
    let mut m = Bench::new();
    m.write32_arm7(0, IE, 0x0000_0080);
    m.write32_arm7(0, IME, 1);
    assert_eq!(
        (arm9_regs(&m), m.irq9().ie(), m.irq7().ie()),
        ([0; 3], 0, 0x80)
    );
}

#[test]
fn given_different_ie_values_on_the_two_cpus_when_each_reads_back_then_each_sees_its_own() {
    let mut m = Bench::new();
    m.write32(&plain(), IE, 0x0000_0011);
    m.write32_arm7(0, IE, 0x0000_0022);
    assert_eq!((m.read32(&plain(), IE), m.read32_arm7(0, IE)), (0x11, 0x22));
}

#[test]
fn given_a_halfword_written_to_each_half_of_ie_on_the_arm9_when_read_as_a_word_then_the_halves_combine()
 {
    let mut m = Bench::new();
    m.write16(&plain(), IE, 0x0102);
    m.write16(&plain(), IE + 2, 0x0004);
    assert_eq!(
        (
            m.read32(&plain(), IE),
            m.read16(&plain(), IE),
            m.read16(&plain(), IE + 2)
        ),
        (0x0004_0102, 0x0102, 0x0004)
    );
}

#[test]
fn given_a_halfword_written_to_each_half_of_ie_on_the_arm7_when_read_as_a_word_then_the_halves_combine()
 {
    let mut m = Bench::new();
    m.write16_arm7(0, IE, 0x0182);
    m.write16_arm7(0, IE + 2, 0x0104);
    assert_eq!(
        (
            m.read32_arm7(0, IE),
            m.read16_arm7(0, IE),
            m.read16_arm7(0, IE + 2)
        ),
        (0x0104_0182, 0x0182, 0x0104)
    );
}

/// The four IE bytes the ARM9 reads back after byte writes of FF to each.
fn arm9_ie_bytes_after_ff_writes() -> Vec<u8> {
    let mut m = Bench::new();
    (0..4).for_each(|i| m.write8(&plain(), IE + i, 0xFF));
    (0..4).map(|i| m.read8(&plain(), IE + i)).collect()
}

/// The same for the ARM7.
fn arm7_ie_bytes_after_ff_writes() -> Vec<u8> {
    let mut m = Bench::new();
    (0..4).for_each(|i| m.write8_arm7(0, IE + i, 0xFF));
    (0..4).map(|i| m.read8_arm7(0, IE + i)).collect()
}

#[test]
fn given_byte_writes_to_ie_on_the_arm9_when_read_back_then_each_byte_is_masked_to_the_arm9_bits() {
    assert_eq!(
        arm9_ie_bytes_after_ff_writes(),
        vec![0x7F, 0x3F, 0x3F, 0x00]
    );
}

#[test]
fn given_byte_writes_to_ie_on_the_arm7_when_read_back_then_each_byte_is_masked_to_the_arm7_bits() {
    assert_eq!(
        arm7_ie_bytes_after_ff_writes(),
        vec![0xFF, 0x3F, 0xDF, 0x01]
    );
}

#[test]
fn given_a_byte_write_of_ff_to_ime_byte_1_when_read_then_ime_is_off_and_the_byte_reads_0() {
    let mut m = Bench::new();
    m.write8(&plain(), IME + 1, 0xFF);
    m.write8_arm7(0, IME + 1, 0xFF);
    assert_eq!(
        (
            m.read8(&plain(), IME + 1),
            m.read8_arm7(0, IME + 1),
            m.irq9().ime(),
            m.irq7().ime()
        ),
        (0, 0, false, false)
    );
}

#[test]
fn given_a_word_write_at_the_address_after_ime_when_read_then_it_reads_0_and_no_register_changed() {
    // 0400_020Ch to 0400_020Fh belong to no interrupt register (`interrupts.md`, "Registers").
    let mut m = Bench::new();
    m.write32(&plain(), IME + 4, 0xFFFF_FFFF);
    m.write32_arm7(0, IME + 4, 0xFFFF_FFFF);
    assert_eq!(
        (
            m.read32(&plain(), IME + 4),
            m.read32_arm7(0, IME + 4),
            arm9_regs(&m),
            arm7_regs(&m)
        ),
        (0, 0, [0; 3], [0; 3])
    );
}

#[test]
fn given_a_request_on_each_cpu_when_the_if_word_is_read_then_each_cpu_sees_only_its_own() {
    let mut m = Bench::new();
    m.io.request_irq(Requests {
        arm9: 0x0000_0001,
        arm7: 0,
    });
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: 0x0000_0100,
    });
    assert_eq!(
        (m.read32(&plain(), IF), m.read32_arm7(0, IF)),
        (0x0000_0001, 0x0000_0100)
    );
}

#[test]
fn given_a_latched_request_when_1_is_written_to_its_if_byte_on_the_arm9_then_it_clears_through_the_bus()
 {
    let mut m = Bench::new();
    m.io.request_irq(Requests {
        arm9: 0b111,
        arm7: 0,
    });
    m.write8(&plain(), IF, 0x01);
    assert_eq!(m.read32(&plain(), IF), 0b110);
}

#[test]
fn given_a_latched_request_when_1_is_written_to_its_if_byte_on_the_arm7_then_it_clears_through_the_bus()
 {
    let mut m = Bench::new();
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: 0b111,
    });
    m.write8_arm7(0, IF, 0x04);
    assert_eq!(m.read32_arm7(0, IF), 0b011);
}

#[test]
fn given_latched_requests_when_a_halfword_write_to_the_low_half_of_if_is_made_then_the_high_half_stays()
 {
    let mut m = Bench::new();
    m.io.request_irq(Requests {
        arm9: 0x0003_0003,
        arm7: 0,
    });
    m.write16(&plain(), IF, 0xFFFF);
    assert_eq!(m.read32(&plain(), IF), 0x0003_0000);
}

#[test]
fn given_latched_requests_when_a_halfword_write_to_the_high_half_of_if_is_made_on_the_arm7_then_the_low_half_stays()
 {
    let mut m = Bench::new();
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: 0x0003_0003,
    });
    m.write16_arm7(0, IF + 2, 0x0001);
    assert_eq!(m.read32_arm7(0, IF), 0x0002_0003);
}

#[test]
fn given_latched_requests_when_a_zero_word_is_written_to_if_then_nothing_clears() {
    let mut m = Bench::new();
    m.io.request_irq(Requests {
        arm9: 0x0000_0007,
        arm7: 0,
    });
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: 0x0000_0007,
    });
    m.write32(&plain(), IF, 0);
    m.write32_arm7(0, IF, 0);
    assert_eq!((m.read32(&plain(), IF), m.read32_arm7(0, IF)), (7, 7));
}

#[test]
fn given_the_same_request_on_both_cpus_when_the_arm9_acknowledges_it_then_the_arm7_if_keeps_it() {
    let mut m = Bench::new();
    m.io.request_irq(Requests {
        arm9: 0x0000_0001,
        arm7: 0,
    });
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: 0x0000_0001,
    });
    m.write32(&plain(), IF, 0xFFFF_FFFF);
    assert_eq!((m.read32(&plain(), IF), m.read32_arm7(0, IF)), (0, 1));
}

#[test]
fn given_ones_written_to_if_when_nothing_was_requested_then_if_stays_zero_on_both_cpus() {
    let mut m = Bench::new();
    m.write32(&plain(), IF, 0xFFFF_FFFF);
    m.write32_arm7(0, IF, 0xFFFF_FFFF);
    assert_eq!((m.read32(&plain(), IF), m.read32_arm7(0, IF)), (0, 0));
}

#[test]
fn given_an_arm9_ime_write_when_a_halfword_and_a_byte_are_read_then_they_are_the_low_parts() {
    let mut m = Bench::new();
    m.write32(&plain(), IME, 0xFFFF_FFFF);
    assert_eq!(
        (
            m.read16(&plain(), IME),
            m.read8(&plain(), IME),
            m.read16(&plain(), IME + 2)
        ),
        (1, 1, 0)
    );
}

// ---------------------------------------------------------------- timer writes schedule at once
//
// Expected values come from `docs/hw/timers.md` ("Clock": one F/1 tick is 2 master cycles;
// "Model": a write to TMnCNT_H replaces the overflow event, and an event never lies before the
// end of the instruction that wrote it).

const TM0CNT_L: u32 = 0x0400_0100;
const TM0CNT_H: u32 = 0x0400_0102;
const TM1CNT_L: u32 = 0x0400_0104;
const START: u16 = 0x80;
/// Reload `FFF0h`: 16 ticks to the first overflow.
const RELOAD_16_TICKS: u16 = 0xFFF0;

fn timer_event_times(m: &Bench, arm9: bool, index: u8) -> Vec<u64> {
    let kind = crate::scheduler::EventKind::TimerOverflow { arm9, index };
    m.scheduler
        .pending()
        .iter()
        .filter(|event| event.kind == kind)
        .map(|event| event.time)
        .collect()
}

#[test]
fn given_an_arm9_view_when_start_is_written_to_tmxcnt_h_then_the_overflow_event_is_queued_without_a_machine_step()
 {
    let mut m = Bench::new();
    m.arm9(plain()).write16(TM0CNT_L, RELOAD_16_TICKS);
    m.arm9(plain()).write16(TM0CNT_H, START);
    assert_eq!(timer_event_times(&m, true, 0), vec![32]);
}

#[test]
fn given_an_arm7_view_when_start_is_written_to_tmxcnt_h_then_the_overflow_event_is_queued_without_a_machine_step()
 {
    let mut m = Bench::new();
    m.arm7(0).write16(TM0CNT_L, RELOAD_16_TICKS);
    m.arm7(0).write16(TM0CNT_H, START);
    assert_eq!(
        (
            timer_event_times(&m, false, 0),
            timer_event_times(&m, true, 0)
        ),
        (vec![32], vec![])
    );
}

#[test]
fn given_a_running_timer_when_stop_is_written_through_a_view_then_its_event_is_cancelled_at_once() {
    let mut m = Bench::new();
    m.arm9(plain()).write16(TM0CNT_L, RELOAD_16_TICKS);
    m.arm9(plain()).write16(TM0CNT_H, START);
    m.arm9(plain()).write16(TM0CNT_H, 0);
    assert_eq!(timer_event_times(&m, true, 0), Vec::<u64>::new());
}

#[test]
fn given_a_reload_write_through_a_view_when_the_timer_is_running_then_its_event_is_unchanged() {
    let mut m = Bench::new();
    m.arm9(plain()).write16(TM0CNT_L, RELOAD_16_TICKS);
    m.arm9(plain()).write16(TM0CNT_H, START);
    m.arm9(plain()).write16(TM0CNT_L, 0x8000);
    assert_eq!(timer_event_times(&m, true, 0), vec![32]);
}

/// A bench whose timer 0 was started at time 0 with reload `FFFFh` (one tick, so it overflows
/// every 2 cycles), then left behind: the master clock is 1000 and nothing fired.
fn lagging_one_tick_timer() -> Bench {
    let mut m = Bench::new();
    m.arm9(plain()).write16(TM0CNT_L, 0xFFFF);
    m.arm9(plain()).write16(TM0CNT_H, START);
    m.scheduler.advance(1000);
    m
}

#[test]
fn given_an_arm9_write_that_settles_a_long_overdue_timer_when_it_is_rescheduled_then_the_event_is_one_instruction_past_the_clock()
 {
    // The write at master time 1000 settles the timer's overdue overflows, 64 at most per write
    // (`SLICE`); the next one it computes, 130, is still far in the past. The event goes no
    // earlier than the end of the writing instruction, 1000 + 1 (`ARM9_INSTRUCTION_COST`), which
    // is where the machine put it when it replaced events after advancing the clock past the
    // instruction.
    let mut m = lagging_one_tick_timer();
    m.arm9(plain()).write8(TM1CNT_L, 0);
    assert_eq!(timer_event_times(&m, true, 0), vec![1001]);
}

#[test]
fn given_an_arm7_write_that_settles_a_long_overdue_timer_when_it_is_rescheduled_then_the_event_is_at_the_clock()
 {
    // The ARM7's step does not move the master clock, so its floor is the clock itself.
    let mut m = Bench::new();
    m.arm7(0).write16(TM0CNT_L, 0xFFFF);
    m.arm7(0).write16(TM0CNT_H, START);
    m.scheduler.advance(1000);
    m.arm7(1000).write8(TM1CNT_L, 0);
    assert_eq!(timer_event_times(&m, false, 0), vec![1000]);
}

/// Master time far above `now` (0), so the unfloored due time of a fresh timer lies below it.
const FAR_FLOOR: u64 = 5000;

/// The overflow event times of ARM9 timer 0 after START is written through a view at `now` 0
/// whose floor is `FAR_FLOOR`. Unfloored, the event is at 32 (16 ticks of 2 cycles).
fn arm9_event_after_start_with_far_floor() -> Vec<u64> {
    let mut m = Bench::new();
    let mut v = Arm9View::new(
        &mut m.memory,
        &mut m.io,
        &mut m.scheduler,
        plain(),
        0,
        FAR_FLOOR,
    );
    v.write16(TM0CNT_L, RELOAD_16_TICKS);
    v.write16(TM0CNT_H, START);
    timer_event_times(&m, true, 0)
}

/// The same through the ARM7's view.
fn arm7_event_after_start_with_far_floor() -> Vec<u64> {
    let mut m = Bench::new();
    let mut v = Arm7View::new(&mut m.memory, &mut m.io, &mut m.scheduler, 0, FAR_FLOOR);
    v.write16(TM0CNT_L, RELOAD_16_TICKS);
    v.write16(TM0CNT_H, START);
    timer_event_times(&m, false, 0)
}

#[test]
fn given_an_arm9_view_with_a_floor_above_the_due_time_when_a_timer_starts_then_the_event_is_at_the_floor()
 {
    assert_eq!(arm9_event_after_start_with_far_floor(), vec![FAR_FLOOR]);
}

#[test]
fn given_an_arm7_view_with_a_floor_above_the_due_time_when_a_timer_starts_then_the_event_is_at_the_floor()
 {
    assert_eq!(arm7_event_after_start_with_far_floor(), vec![FAR_FLOOR]);
}

// ------------------------------------------------------------------------------ the BIOS area

/// The ARM9 BIOS is 32 KiB at `FFFF0000h`; the ARM7 BIOS is 16 KiB at 0 (`hle-swi.md`, "The BIOS
/// area"). Rows: (address, ARM9 region, ARM7 region); the ARM9's low addresses are not BIOS
/// whatever the vector base is.
#[test]
fn given_power_on_memory_when_the_bios_edges_are_looked_up_then_each_cpu_sees_only_its_own() {
    let table = [
        (0x0000_0000, Region::Unmapped, Region::Bios),
        (0x0000_3FFF, Region::Unmapped, Region::Bios),
        (0x0000_4000, Region::Unmapped, Region::Unmapped),
        (0xFFFE_FFFF, Region::Unmapped, Region::Unmapped),
        (0xFFFF_0000, Region::Bios, Region::Unmapped),
        (0xFFFF_7FFF, Region::Bios, Region::Unmapped),
        (0xFFFF_8000, Region::Unmapped, Region::Unmapped),
    ];
    let m = Bench::new();
    let failures: Vec<_> = table
        .iter()
        .filter(|&&(addr, nine, seven)| (m.region(addr), m.region7(addr)) != (nine, seven))
        .collect();
    assert_eq!(failures, Vec::<&(u32, Region, Region)>::new());
}

#[test]
fn given_the_bios_area_when_written_then_it_still_reads_zero_on_both_cpus() {
    let mut m = Bench::new();
    write_as(&mut m, Nine, 0xFFFF_0008, 0xAB);
    write_as(&mut m, Seven, 0x0000_0008, 0xCD);
    assert_eq!(
        (
            read_as(&m, Nine, 0xFFFF_0008),
            read_as(&m, Seven, 0x0000_0008)
        ),
        (0, 0)
    );
}

#[test]
fn given_the_bios_area_when_fetched_then_a_synthetic_zero_word_is_read() {
    let mut m = Bench::new();
    assert_eq!(
        (
            m.fetch32(&plain(), 0xFFFF_0018),
            m.fetch32_arm7(0, 0x0000_0018)
        ),
        (0, 0)
    );
}

// ---------------------------------------------------------------- VRAM banks A-I (LCDC mode)

/// VRAMCNT address and LCDC start of each bank, A to I (`vram-banks.md`, "The banks").
const LCDC_BANKS: [(u32, u32); 9] = [
    (0x0400_0240, 0x0680_0000),
    (0x0400_0241, 0x0682_0000),
    (0x0400_0242, 0x0684_0000),
    (0x0400_0243, 0x0686_0000),
    (0x0400_0244, 0x0688_0000),
    (0x0400_0245, 0x0689_0000),
    (0x0400_0246, 0x0689_4000),
    (0x0400_0248, 0x0689_8000),
    (0x0400_0249, 0x068A_0000),
];

/// Banks whose marker word, written at the first address of their window, does not read back
/// there once every bank is enabled (each marker differs, so a bank mapped over another would show).
fn marker_failures() -> Vec<usize> {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    LCDC_BANKS
        .iter()
        .for_each(|&(reg, _)| v.write8(reg, ENABLE_MST0));
    LCDC_BANKS
        .iter()
        .enumerate()
        .for_each(|(i, &(_, start))| v.write32(start, 0xB000_0000 + i as u32));
    LCDC_BANKS
        .iter()
        .enumerate()
        .filter(|&(i, &(_, start))| v.read32(start) != 0xB000_0000 + i as u32)
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn given_all_nine_banks_enabled_when_a_marker_is_stored_in_each_window_then_each_reads_back_its_own()
 {
    assert_eq!(marker_failures(), Vec::<usize>::new());
}

#[test]
fn given_bank_g_enabled_when_a_byte_is_stored_in_its_window_then_it_is_ignored() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write8(0x0400_0246, ENABLE_MST0);
    v.write32(0x0689_4000, X);
    v.write8(0x0689_4000, 0xFF);
    assert_eq!(v.read32(0x0689_4000), X);
}

#[test]
fn given_a_word_written_at_0400_0248_when_decoded_then_banks_h_and_i_are_enabled() {
    let mut m = Bench::new();
    m.write32(&plain(), 0x0400_0248, 0xFFFF_8383_u32);
    let banks = m.vram_banks();
    assert_eq!(
        (banks[7], banks[8]),
        (
            state(true, 3, 0, VramMapping::Unmapped),
            state(true, 3, 0, VramMapping::ObjExtPaletteB)
        )
    );
}

#[test]
fn given_a_halfword_written_at_0400_0246_when_decoded_then_bank_g_and_wramcnt_take_their_own_byte()
{
    let mut m = Bench::new();
    m.write16(&plain(), 0x0400_0246, 0x0281);
    assert_eq!(
        (m.vram_banks()[6], m.read8(&plain(), WRAMCNT)),
        (state(true, 1, 0, VramMapping::BgA { offset: 0 }), 2)
    );
}

#[test]
fn given_vramcnt_registers_written_when_read_at_each_width_then_they_read_zero() {
    let mut m = Bench::new();
    let mut v = m.arm9(plain());
    v.write32(0x0400_0240, 0x8080_8080);
    v.write16(0x0400_0248, 0x8080);
    assert_eq!(
        (
            v.read32(0x0400_0240),
            v.read16(0x0400_0248),
            v.read8(0x0400_0249)
        ),
        (0, 0, 0)
    );
}

#[test]
fn given_bank_e_written_with_ofs_bits_when_decoded_then_the_offset_reads_zero() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0244, 0x80 | 0x18 | 1);
    assert_eq!(
        m.vram_banks()[4],
        state(true, 1, 0, VramMapping::BgA { offset: 0 })
    );
}

// ---------------------------------------------------------------- the ARM7's VRAM (banks C, D)

#[test]
fn given_c_and_d_set_to_mst_2_by_the_arm9_when_the_arm7_reads_vramstat_then_both_bits_are_set() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0242, 0x82);
    m.write8(&plain(), 0x0400_0243, 0x8A);
    assert_eq!(
        (m.read8_arm7(0, 0x0400_0240), m.read16_arm7(0, 0x0400_0240)),
        (3, 0x0003)
    );
}

#[test]
fn given_vramcnt_c_changed_when_the_arm7_reads_vramstat_again_then_it_follows_at_once() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0242, 0x82);
    let on = m.read8_arm7(0, 0x0400_0240);
    m.write8(&plain(), 0x0400_0242, 0x80);
    assert_eq!((on, m.read8_arm7(0, 0x0400_0240)), (1, 0));
}

#[test]
fn given_the_arm7_writes_vramstat_through_its_view_when_read_then_it_is_unchanged() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0242, 0x82);
    m.write8_arm7(0, 0x0400_0240, 0xFF);
    assert_eq!(m.read8_arm7(0, 0x0400_0240), 1);
}

#[test]
fn given_bank_c_in_the_arm7_mapping_when_the_arm7_stores_a_byte_through_its_view_then_it_lands() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0242, 0x82);
    m.write32_arm7(0, 0x0600_0000, 0x1122_3344);
    m.write8_arm7(0, 0x0600_0001, 0xEE);
    assert_eq!(m.read32_arm7(0, 0x0600_0000), 0x1122_EE44);
}

#[test]
fn given_bank_c_in_the_arm7_mapping_when_the_arm9_looks_for_it_then_every_address_is_empty() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0242, 0x82);
    m.write32_arm7(0, 0x0600_0000, 0xCAFE_F00D);
    assert_eq!(
        (
            m.read32(&plain(), 0x0600_0000),
            m.read32(&plain(), 0x0684_0000),
            m.read32(&plain(), 0x0620_0000)
        ),
        (0, 0, 0)
    );
}

#[test]
fn given_the_arm7_without_vram_banks_when_it_reads_the_vram_block_then_it_reads_zero() {
    let mut m = Bench::new();
    m.write8(&plain(), 0x0400_0240, 0x80);
    assert_eq!(m.read32_arm7(0, 0x0680_0000), 0);
}
