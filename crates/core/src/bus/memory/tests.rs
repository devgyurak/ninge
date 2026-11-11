// SPDX-License-Identifier: Apache-2.0
//! Tests of `memory`.

use super::*;

#[test]
fn given_power_on_memory_when_checked_then_it_is_well_formed() {
    assert!(Memory::new().is_well_formed());
}

#[test]
fn given_a_short_main_ram_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.main_ram.pop();
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_short_itcm_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.itcm.pop();
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_short_dtcm_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.dtcm.pop();
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_long_vram_a_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.vram_a.push(0);
    assert!(!memory.is_well_formed());
}

#[test]
fn given_an_unused_vramcnt_a_bit_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.vramcnt_a = !VRAMCNT_USED;
    assert!(!memory.is_well_formed());
}

#[test]
fn given_wramcnt_with_bits_2_to_7_set_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.wramcnt = 0x04;
    assert!(!memory.is_well_formed());
}

#[test]
fn given_wramcnt_3_when_checked_then_it_is_well_formed() {
    let mut memory = Memory::new();
    memory.wramcnt = 3;
    assert!(memory.is_well_formed());
}

#[test]
fn given_a_short_shared_wram_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.shared_wram.pop();
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_long_shared_wram_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.shared_wram.push(0);
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_short_palette_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.palette.pop();
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_long_palette_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.palette.push(0);
    assert!(!memory.is_well_formed());
}
