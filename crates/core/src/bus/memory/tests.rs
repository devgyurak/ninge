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

// ---------------------------------------------------------------- VRAM banks A-I, LCDC mode
//
// Expected values are the tables of `docs/hw/vram-banks.md`, written out here, not derived from
// the code under test.

/// VRAMCNT address, LCDC start and size of each bank, A to I.
const BANKS: [(u32, u32, u32); 9] = [
    (0x0400_0240, 0x0680_0000, 0x2_0000),
    (0x0400_0241, 0x0682_0000, 0x2_0000),
    (0x0400_0242, 0x0684_0000, 0x2_0000),
    (0x0400_0243, 0x0686_0000, 0x2_0000),
    (0x0400_0244, 0x0688_0000, 0x1_0000),
    (0x0400_0245, 0x0689_0000, 0x4000),
    (0x0400_0246, 0x0689_4000, 0x4000),
    (0x0400_0248, 0x0689_8000, 0x8000),
    (0x0400_0249, 0x068A_0000, 0x4000),
];
const LCDC_END: u32 = 0x068A_4000;
const LCDC_ON: u8 = 0x80;

fn set(memory: &mut Memory, addr: u32, value: u8) {
    memory.write_control::<true>(addr - VRAMCNT_A_ADDR, Width::Byte, u32::from(value));
}

fn enable_all(memory: &mut Memory) {
    BANKS
        .iter()
        .for_each(|&(reg, _, _)| set(memory, reg, LCDC_ON));
}

fn region9(memory: &Memory, addr: u32) -> Region {
    page_of(&memory.pages9, addr).region
}

/// Banks whose first or last byte is not `Vram` once the bank alone is enabled.
fn window_edge_failures() -> Vec<usize> {
    (0..BANKS.len())
        .filter(|&i| {
            let mut memory = Memory::new();
            set(&mut memory, BANKS[i].0, LCDC_ON);
            let (_, start, size) = BANKS[i];
            region9(&memory, start) != Region::Vram
                || region9(&memory, start + size - 1) != Region::Vram
        })
        .collect()
}

/// Banks that, enabled alone, also map the byte before or after their window.
fn neighbour_failures() -> Vec<usize> {
    (0..BANKS.len())
        .filter(|&i| {
            let mut memory = Memory::new();
            set(&mut memory, BANKS[i].0, LCDC_ON);
            let (_, start, size) = BANKS[i];
            region9(&memory, start - 1) != Region::Unmapped
                || region9(&memory, start + size) != Region::Unmapped
        })
        .collect()
}

/// Banks that stay mapped, or whose neighbours vanish, when only that bank is disabled again.
fn disable_failures() -> Vec<usize> {
    (0..BANKS.len())
        .filter(|&i| {
            let mut memory = Memory::new();
            enable_all(&mut memory);
            set(&mut memory, BANKS[i].0, 0);
            let (_, start, size) = BANKS[i];
            let gone = region9(&memory, start) == Region::Unmapped
                && region9(&memory, start + size - 1) == Region::Unmapped;
            let others = BANKS
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .all(|(_, &(_, other, _))| region9(&memory, other) == Region::Vram);
            !(gone && others)
        })
        .collect()
}

/// Pages of the nine windows whose buffer offset is not their LCDC offset.
fn offset_failures() -> Vec<u32> {
    let mut memory = Memory::new();
    enable_all(&mut memory);
    (0x0680_0000..LCDC_END)
        .step_by(PAGE_SIZE)
        .filter(|&addr| page_of(&memory.pages9, addr).offset != addr - 0x0680_0000)
        .collect()
}

#[test]
fn given_each_bank_enabled_in_lcdc_mode_when_its_first_and_last_byte_are_looked_up_then_both_are_vram()
 {
    assert_eq!(window_edge_failures(), Vec::<usize>::new());
}

#[test]
fn given_each_bank_enabled_alone_when_the_bytes_around_its_window_are_looked_up_then_they_are_unmapped()
 {
    assert_eq!(neighbour_failures(), Vec::<usize>::new());
}

#[test]
fn given_all_banks_enabled_when_one_is_disabled_then_only_its_window_is_unmapped() {
    assert_eq!(disable_failures(), Vec::<usize>::new());
}

#[test]
fn given_all_banks_enabled_when_the_windows_are_looked_up_then_each_page_maps_its_lcdc_offset() {
    assert_eq!(offset_failures(), Vec::<u32>::new());
}

#[test]
fn given_all_banks_enabled_when_the_address_past_bank_i_is_looked_up_then_it_is_unmapped() {
    let mut memory = Memory::new();
    enable_all(&mut memory);
    assert_eq!(
        (
            region9(&memory, LCDC_END),
            region9(&memory, 0x06FF_FFFF),
            region9(&memory, 0x067F_FFFF)
        ),
        (Region::Unmapped, Region::Unmapped, Region::Unmapped)
    );
}

#[test]
fn given_a_bank_enabled_with_mst_0_when_the_arm7_looks_up_its_window_then_it_is_unmapped() {
    let mut memory = Memory::new();
    enable_all(&mut memory);
    assert_eq!(
        page_of(&memory.pages7, 0x0680_0000).region,
        Region::Unmapped
    );
}

/// Control bytes written to each VRAMCNT, and what each register keeps (`vram-banks.md`,
/// "VRAMCNT bits"): the registers whose kept bits differ.
fn kept_bit_failures() -> Vec<(usize, u8)> {
    let kept = [0x9B, 0x9B, 0x9F, 0x9F, 0x87, 0x9F, 0x9F, 0x83, 0x83];
    let mut memory = Memory::new();
    BANKS
        .iter()
        .for_each(|&(reg, _, _)| set(&mut memory, reg, 0xFF));
    (0..BANKS.len())
        .map(|i| (i, memory.vramcnt[i]))
        .filter(|&(i, got)| got != kept[i])
        .collect()
}

#[test]
fn given_0xff_written_to_every_vramcnt_when_stored_then_each_keeps_only_its_used_bits() {
    assert_eq!(kept_bit_failures(), Vec::<(usize, u8)>::new());
}

#[test]
fn given_wramcnt_address_between_vramcnt_g_and_h_when_written_then_no_vram_register_changes() {
    let mut memory = Memory::new();
    set(&mut memory, 0x0400_0247, 0xFF);
    assert_eq!(memory.vramcnt, [0; 9]);
}

/// Whether the tables after `writes` equal the tables a full rebuild gives.
fn rebuild_differs(writes: &[(u32, u8)]) -> bool {
    let mut memory = Memory::new();
    writes
        .iter()
        .for_each(|&(addr, value)| set(&mut memory, addr, value));
    let mut full = memory.clone();
    full.build_page_table();
    memory.pages9 != full.pages9 || memory.pages7 != full.pages7
}

/// The write sequences whose partial rebuild differs from the full one.
fn partial_rebuild_failures() -> Vec<usize> {
    let sequences: [&[(u32, u8)]; 4] = [
        &[(0x0400_0240, 0x80), (0x0400_0241, 0x80)],
        &[(0x0400_0240, 0x81), (0x0400_0240, 0x80), (0x0400_0240, 0)],
        &[(0x0400_0247, 3), (0x0400_0249, 0x80), (0x0400_0246, 0x80)],
        &[
            (0x0400_0248, 0x83),
            (0x0400_0245, 0x87),
            (0x0400_0245, 0x80),
        ],
    ];
    (0..sequences.len())
        .filter(|&i| rebuild_differs(sequences[i]))
        .collect()
}

#[test]
fn given_vramcnt_writes_when_the_vram_block_is_rebuilt_then_both_tables_equal_a_full_rebuild() {
    assert_eq!(partial_rebuild_failures(), Vec::<usize>::new());
}

#[test]
fn given_a_page_outside_the_vram_block_changed_when_vramcnt_is_written_then_it_is_not_rebuilt() {
    let mut memory = Memory::new();
    let main_ram_page = (MAIN_RAM_BASE >> PAGE_SHIFT) as usize;
    memory.pages9[main_ram_page] = UNMAPPED_PAGE;
    set(&mut memory, 0x0400_0240, LCDC_ON);
    assert_eq!(memory.pages9[main_ram_page], UNMAPPED_PAGE);
}

#[test]
fn given_a_page_in_the_vram_block_changed_when_vramcnt_is_written_then_it_is_rebuilt() {
    let mut memory = Memory::new();
    let stray = (0x06F0_0000u32 >> PAGE_SHIFT) as usize;
    memory.pages9[stray] = Page {
        region: Region::MainRam,
        offset: 0,
    };
    set(&mut memory, 0x0400_0240, LCDC_ON);
    assert_eq!(memory.pages9[stray], UNMAPPED_PAGE);
}
