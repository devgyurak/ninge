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
fn given_a_long_vram_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.vram.push(0);
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_short_vram_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.vram.pop();
    assert!(!memory.is_well_formed());
}

#[test]
fn given_an_unused_vramcnt_bit_when_checked_then_it_is_not_well_formed() {
    // Bit 2 is not used by bank A (`vram-banks.md`, "VRAMCNT bits").
    let mut memory = Memory::new();
    memory.vramcnt[0] = 0x04;
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

#[test]
fn given_a_short_oam_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.oam.pop();
    assert!(!memory.is_well_formed());
}

#[test]
fn given_a_long_oam_when_checked_then_it_is_not_well_formed() {
    let mut memory = Memory::new();
    memory.oam.push(0);
    assert!(!memory.is_well_formed());
}

mod ext;
mod texture;
mod vram;

#[test]
fn given_marked_palette_halves_when_engine_inputs_built_then_each_engine_gets_its_own_half() {
    let mut memory = Memory::new();
    memory.palette[0] = 0xA1;
    memory.palette[PALETTE_HALF] = 0xB2;
    let first_bytes = (
        memory.engine_inputs(EngineId::A).palette.first().copied(),
        memory.engine_inputs(EngineId::B).palette.first().copied(),
    );
    assert_eq!(first_bytes, (Some(0xA1), Some(0xB2)));
}

#[test]
fn given_marked_oam_halves_when_engine_inputs_built_then_each_engine_gets_its_own_half() {
    let mut memory = Memory::new();
    memory.oam[0] = 0xA1;
    memory.oam[OAM_HALF] = 0xB2;
    let first_bytes = (
        memory.engine_inputs(EngineId::A).oam.first().copied(),
        memory.engine_inputs(EngineId::B).oam.first().copied(),
    );
    assert_eq!(first_bytes, (Some(0xA1), Some(0xB2)));
}

#[test]
fn given_power_on_memory_when_oam_of_each_engine_then_halves_are_a_kilobyte_each() {
    let memory = Memory::new();
    let lengths = (
        memory.oam_of(EngineId::A).len(),
        memory.oam_of(EngineId::B).len(),
    );
    assert_eq!(lengths, (OAM_HALF, OAM_HALF));
}
