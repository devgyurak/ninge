// SPDX-License-Identifier: Apache-2.0
//! The VRAM banks: page-table mapping and the interpreted VRAMCNT (`docs/hw/vram-banks.md`).

use super::*;

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

// ---------------------------------------------------------------- BG and OBJ mappings

/// (bank, VRAMCNT value, CPU address the bank must start at): every MST and OFS that puts a bank
/// in a BG or OBJ window (`vram-banks.md`, "Where each bank goes"). OFS.1 is ignored where the
/// note says it must be zero (A, B in OBJ mode).
const WINDOW_MAPPINGS: [(usize, u8, u32); 47] = [
    (0, 0x81, 0x06000000),
    (0, 0x89, 0x06020000),
    (0, 0x91, 0x06040000),
    (0, 0x99, 0x06060000),
    (1, 0x81, 0x06000000),
    (1, 0x89, 0x06020000),
    (1, 0x91, 0x06040000),
    (1, 0x99, 0x06060000),
    (2, 0x81, 0x06000000),
    (2, 0x89, 0x06020000),
    (2, 0x91, 0x06040000),
    (2, 0x99, 0x06060000),
    (3, 0x81, 0x06000000),
    (3, 0x89, 0x06020000),
    (3, 0x91, 0x06040000),
    (3, 0x99, 0x06060000),
    (4, 0x81, 0x06000000),
    (5, 0x81, 0x06000000),
    (5, 0x89, 0x06004000),
    (5, 0x91, 0x06010000),
    (5, 0x99, 0x06014000),
    (6, 0x81, 0x06000000),
    (6, 0x89, 0x06004000),
    (6, 0x91, 0x06010000),
    (6, 0x99, 0x06014000),
    (0, 0x82, 0x06400000),
    (0, 0x8A, 0x06420000),
    (0, 0x92, 0x06400000),
    (0, 0x9A, 0x06420000),
    (1, 0x82, 0x06400000),
    (1, 0x8A, 0x06420000),
    (1, 0x92, 0x06400000),
    (1, 0x9A, 0x06420000),
    (4, 0x82, 0x06400000),
    (5, 0x82, 0x06400000),
    (5, 0x8A, 0x06404000),
    (5, 0x92, 0x06410000),
    (5, 0x9A, 0x06414000),
    (6, 0x82, 0x06400000),
    (6, 0x8A, 0x06404000),
    (6, 0x92, 0x06410000),
    (6, 0x9A, 0x06414000),
    (2, 0x84, 0x06200000),
    (7, 0x81, 0x06200000),
    (8, 0x81, 0x06208000),
    (3, 0x84, 0x06600000),
    (8, 0x82, 0x06600000),
];

fn memory_with(writes: &[(u32, u8)]) -> Memory {
    let mut memory = Memory::new();
    writes
        .iter()
        .for_each(|&(addr, value)| set(&mut memory, addr, value));
    memory
}

/// The rows whose bank does not occupy exactly its window at the expected address once it is the
/// only bank enabled: first and last byte are the bank's own bytes, the neighbours are not VRAM.
fn window_mapping_failures() -> Vec<(usize, u8)> {
    WINDOW_MAPPINGS
        .iter()
        .filter(|&&(bank, value, start)| {
            let memory = memory_with(&[(BANKS[bank].0, value)]);
            let size = BANKS[bank].2;
            let own = |addr: u32, offset: u32| {
                let page = page_of(&memory.pages9, addr);
                page.region == Region::Vram && page.offset == BANK_BASE[bank] as u32 + offset
            };
            let edges =
                own(start, 0) && own(start + size - 1, (size - 1) & !(PAGE_SIZE as u32 - 1));
            let outside = region9(&memory, start - 1) != Region::Vram
                && region9(&memory, start + size) != Region::Vram;
            let arm7_empty = page_of(&memory.pages7, start).region == Region::Unmapped;
            !(edges && outside && arm7_empty)
        })
        .map(|&(bank, value, _)| (bank, value))
        .collect()
}

#[test]
fn given_each_bank_with_each_mst_and_ofs_that_maps_a_window_when_looked_up_then_it_sits_exactly_at_the_expected_address()
 {
    assert_eq!(window_mapping_failures(), Vec::<(usize, u8)>::new());
}

/// (bank, VRAMCNT value) the display controller alone sees, or that map nowhere: after the write
/// no page of the VRAM block is VRAM.
const NOT_IN_THE_CPU_SPACE: [(usize, u8); 31] = [
    // Texture images.
    (0, 0x83),
    (1, 0x8B),
    (2, 0x93),
    (3, 0x9B),
    // Texture palettes.
    (4, 0x83),
    (5, 0x83),
    (5, 0x9B),
    (6, 0x8B),
    // Engine A BG extended palettes.
    (4, 0x84),
    (5, 0x84),
    (5, 0x8C),
    (6, 0x84),
    // Engine A OBJ extended palettes.
    (5, 0x85),
    (6, 0x85),
    // Engine B extended palettes.
    (7, 0x82),
    (8, 0x83),
    // Values no table defines.
    (3, 0x87),
    (2, 0x85),
    (2, 0x86),
    (2, 0x87),
    (3, 0x85),
    (4, 0x85),
    (4, 0x86),
    (4, 0x87),
    (5, 0x86),
    (6, 0x87),
    (7, 0x83),
    (2, 0x95),
    (1, 0x83),
    (6, 0x86),
    (3, 0x86),
];

/// The (bank, value) pairs that leave a VRAM page in the block.
fn invisible_mapping_failures() -> Vec<(usize, u8)> {
    NOT_IN_THE_CPU_SPACE
        .iter()
        .copied()
        .filter(|&(bank, value)| {
            let memory = memory_with(&[(BANKS[bank].0, value)]);
            (0x0600_0000..0x0700_0000u32)
                .step_by(PAGE_SIZE)
                .any(|addr| region9(&memory, addr) == Region::Vram)
        })
        .collect()
}

#[test]
fn given_a_bank_in_a_texture_or_extended_palette_mode_or_an_undefined_mst_when_looked_up_then_no_cpu_address_holds_it()
 {
    assert_eq!(invisible_mapping_failures(), Vec::<(usize, u8)>::new());
}

/// (bank, VRAMCNT value, expected mapping) for the display-controller-only uses.
fn interpreted_cases() -> Vec<(usize, u8, VramMapping)> {
    use VramMapping::*;
    vec![
        (0, 0x83, Texture { slot: 0 }),
        (3, 0x9B, Texture { slot: 3 }),
        (4, 0x83, TexPalette { first: 0, count: 4 }),
        (5, 0x83, TexPalette { first: 0, count: 1 }),
        (5, 0x8B, TexPalette { first: 1, count: 1 }),
        (6, 0x93, TexPalette { first: 4, count: 1 }),
        (6, 0x9B, TexPalette { first: 5, count: 1 }),
        (4, 0x84, BgExtPaletteA { first: 0, count: 4 }),
        (5, 0x84, BgExtPaletteA { first: 0, count: 2 }),
        (5, 0x8C, BgExtPaletteA { first: 2, count: 2 }),
        (6, 0x8C, BgExtPaletteA { first: 2, count: 2 }),
        (5, 0x85, ObjExtPaletteA),
        (7, 0x82, BgExtPaletteB),
        (8, 0x83, ObjExtPaletteB),
        (0, 0x81, BgA { offset: 0 }),
        (1, 0x91, BgA { offset: 0x4_0000 }),
        (5, 0x99, BgA { offset: 0x1_4000 }),
        (1, 0x92, ObjA { offset: 0 }),
        (1, 0x8A, ObjA { offset: 0x2_0000 }),
        (8, 0x81, BgB { offset: 0x8000 }),
        (3, 0x84, ObjB { offset: 0 }),
        (2, 0x80, Lcdc),
        (2, 0x00, Disabled),
        (2, 0x85, Unmapped),
    ]
}

/// The cases whose decoded mapping differs.
fn interpreted_failures() -> Vec<(usize, u8, VramMapping)> {
    interpreted_cases()
        .into_iter()
        .filter(|&(bank, value, want)| {
            memory_with(&[(BANKS[bank].0, value)])
                .vram_bank(bank)
                .mapping
                != want
        })
        .collect()
}

#[test]
fn given_vramcnt_values_when_decoded_then_the_mapping_names_the_use_and_slot_or_offset() {
    assert_eq!(interpreted_failures(), vec![]);
}

#[test]
fn given_all_nine_banks_written_when_inspected_as_an_array_then_each_decodes_its_own_register() {
    let memory = memory_with(&[
        (0x0400_0240, 0x81),
        (0x0400_0242, 0x84),
        (0x0400_0244, 0x84),
        (0x0400_0248, 0x82),
    ]);
    let banks = memory.vram_banks();
    assert_eq!(
        (
            banks[0].mapping,
            banks[2].mapping,
            banks[4].mapping,
            banks[7].mapping,
            banks[8].mapping
        ),
        (
            VramMapping::BgA { offset: 0 },
            VramMapping::BgB { offset: 0 },
            VramMapping::BgExtPaletteA { first: 0, count: 4 },
            VramMapping::BgExtPaletteB,
            VramMapping::Disabled
        )
    );
}

// ---------------------------------------------------------------- overlap

/// Which bank's bytes the page at CPU address `addr` holds: the bank whose buffer range contains
/// the page's offset, or `None` when no VRAM is there.
fn owner_at(memory: &Memory, addr: u32) -> Option<usize> {
    let page = page_of(&memory.pages9, addr);
    (page.region == Region::Vram)
        .then(|| {
            (0..BANKS.len()).find(|&bank| {
                let start = BANK_BASE[bank] as u32;
                (start..start + BANKS[bank].2).contains(&page.offset)
            })
        })
        .flatten()
}

/// Register writes, the CPU address looked up, and the bank expected to own it.
type OverlapCase = (Vec<(u32, u8)>, u32, Option<usize>);

/// Overlap cases: (writes, CPU address, expected owner). The earlier bank in the alphabet wins
/// (`vram-banks.md`, "Overlap").
fn overlap_cases() -> Vec<OverlapCase> {
    let (a, b, c, d, e, f, g, h, i) = (
        0x0400_0240,
        0x0400_0241,
        0x0400_0242,
        0x0400_0243,
        0x0400_0244,
        0x0400_0245,
        0x0400_0246,
        0x0400_0248,
        0x0400_0249,
    );
    vec![
        // Two banks at the same BG offset: A wins; B shows once A is gone.
        (vec![(a, 0x81), (b, 0x81)], 0x0600_0000, Some(0)),
        (vec![(a, 0x81), (b, 0x81), (a, 0x00)], 0x0600_0000, Some(1)),
        // E under A: A covers E's whole 64 KiB.
        (vec![(e, 0x81), (a, 0x81)], 0x0600_0000, Some(0)),
        (vec![(e, 0x81), (a, 0x81)], 0x0601_C000, Some(0)),
        // E beside A (A at OFS 1): both show.
        (vec![(e, 0x81), (a, 0x89)], 0x0600_0000, Some(4)),
        (vec![(e, 0x81), (a, 0x89)], 0x0602_0000, Some(0)),
        // G and F at the same 16 KiB: F wins; at neighbouring steps both show.
        (vec![(g, 0x81), (f, 0x81)], 0x0600_0000, Some(5)),
        (vec![(f, 0x81), (g, 0x89)], 0x0600_0000, Some(5)),
        (vec![(f, 0x81), (g, 0x89)], 0x0600_4000, Some(6)),
        // Engine B BG: C covers H and I; H and I alone sit side by side.
        (vec![(c, 0x84), (h, 0x81), (i, 0x81)], 0x0620_0000, Some(2)),
        (vec![(c, 0x84), (h, 0x81), (i, 0x81)], 0x0620_8000, Some(2)),
        (vec![(h, 0x81), (i, 0x81)], 0x0620_0000, Some(7)),
        (vec![(h, 0x81), (i, 0x81)], 0x0620_8000, Some(8)),
        // Engine B OBJ: D wins over I.
        (vec![(d, 0x84), (i, 0x82)], 0x0660_0000, Some(3)),
        (vec![(i, 0x82)], 0x0660_0000, Some(8)),
        // Different windows never overlap: A in BG A, B in OBJ A.
        (vec![(a, 0x81), (b, 0x82)], 0x0640_0000, Some(1)),
    ]
}

fn overlap_failures() -> Vec<(u32, Option<usize>, Option<usize>)> {
    overlap_cases()
        .into_iter()
        .map(|(writes, addr, want)| (addr, want, owner_at(&memory_with(&writes), addr)))
        .filter(|&(_, want, got)| want != got)
        .collect()
}

#[test]
fn given_overlapping_banks_when_looked_up_then_the_earlier_bank_in_the_alphabet_owns_each_page() {
    assert_eq!(overlap_failures(), vec![]);
}

#[test]
fn given_overlapping_banks_when_the_vram_block_is_rebuilt_in_place_then_it_equals_a_full_rebuild() {
    // The overlap order must not depend on the order the registers were written.
    let writes = [
        (0x0400_0241, 0x81),
        (0x0400_0240, 0x81),
        (0x0400_0249, 0x82),
        (0x0400_0243, 0x84),
    ];
    assert!(!rebuild_differs(&writes));
}

#[test]
fn given_two_banks_at_one_address_when_the_later_one_is_written_first_then_the_earlier_still_wins()
{
    let a_then_b = memory_with(&[(0x0400_0240, 0x81), (0x0400_0241, 0x81)]);
    let b_then_a = memory_with(&[(0x0400_0241, 0x81), (0x0400_0240, 0x81)]);
    assert_eq!(
        (
            owner_at(&a_then_b, 0x0600_0000),
            owner_at(&b_then_a, 0x0600_0000)
        ),
        (Some(0), Some(0))
    );
}

// ---------------------------------------------------------------- the renderer's windows

fn put_word(memory: &mut Memory, addr: u32, value: u32) {
    memory.store::<true, 4>(&TcmConfig::NONE, addr, value.to_le_bytes());
}

fn window_byte(memory: &Memory, kind: VramWindowKind, offset: u32) -> u8 {
    memory.vram_window(kind).byte(offset)
}

#[test]
fn given_a_word_stored_through_the_cpu_in_each_window_when_the_window_is_read_by_offset_then_it_is_the_same_bytes()
 {
    let mut memory = memory_with(&[
        (0x0400_0240, 0x81),
        (0x0400_0241, 0x82),
        (0x0400_0242, 0x84),
        (0x0400_0243, 0x84),
    ]);
    put_word(&mut memory, 0x0600_0010, 0x4433_2211);
    put_word(&mut memory, 0x0640_0010, 0x8877_6655);
    put_word(&mut memory, 0x0620_0010, 0xCCBB_AA99);
    put_word(&mut memory, 0x0660_0010, 0x1234_5678);
    let read = |kind| {
        let w = memory.vram_window(kind);
        (w.byte(0x10), w.half(0x12))
    };
    assert_eq!(
        (
            read(VramWindowKind::BgA),
            read(VramWindowKind::ObjA),
            read(VramWindowKind::BgB),
            read(VramWindowKind::ObjB)
        ),
        (
            (0x11, 0x4433),
            (0x55, 0x8877),
            (0x99, 0xCCBB),
            (0x78, 0x1234)
        )
    );
}

#[test]
fn given_a_window_when_read_past_its_size_then_it_reads_zero_even_if_a_bank_lies_beyond() {
    // Engine A BG is 512 KiB, engine B BG 128 KiB: engine B's window starts at 0620_0000h, far
    // past the A window's size, and a read there by offset must not reach it.
    let mut memory = memory_with(&[(0x0400_0242, 0x84)]);
    put_word(&mut memory, 0x0620_0000, 0xFFFF_FFFF);
    assert_eq!(
        (
            window_byte(&memory, VramWindowKind::BgB, 0),
            window_byte(&memory, VramWindowKind::BgB, 0x2_0000),
            window_byte(&memory, VramWindowKind::BgA, 0x20_0000),
            window_byte(&memory, VramWindowKind::BgA, u32::MAX)
        ),
        (0xFF, 0, 0, 0)
    );
}

#[test]
fn given_a_bank_that_moved_when_the_window_is_read_then_it_follows_the_register() {
    let mut memory = memory_with(&[(0x0400_0240, 0x81)]);
    put_word(&mut memory, 0x0600_0000, 0xA5A5_A5A5);
    set(&mut memory, 0x0400_0240, 0x89);
    let moved = (
        window_byte(&memory, VramWindowKind::BgA, 0),
        window_byte(&memory, VramWindowKind::BgA, 0x2_0000),
    );
    set(&mut memory, 0x0400_0240, 0x00);
    assert_eq!(
        (moved, window_byte(&memory, VramWindowKind::BgA, 0x2_0000)),
        ((0, 0xA5), 0)
    );
}

#[test]
fn given_a_window_with_no_bank_when_read_then_every_offset_is_zero() {
    let memory = Memory::new();
    assert_eq!(
        (
            window_byte(&memory, VramWindowKind::BgA, 0),
            window_byte(&memory, VramWindowKind::ObjA, 0),
            window_byte(&memory, VramWindowKind::BgB, 0),
            window_byte(&memory, VramWindowKind::ObjB, 0)
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn given_a_byte_stored_in_each_window_when_read_back_then_the_store_was_ignored() {
    let mut memory = memory_with(&[
        (0x0400_0240, 0x81),
        (0x0400_0241, 0x82),
        (0x0400_0242, 0x84),
        (0x0400_0243, 0x84),
    ]);
    let stored: Vec<bool> = [0x0600_0000u32, 0x0640_0000, 0x0620_0000, 0x0660_0000]
        .iter()
        .map(|&addr| memory.store::<true, 1>(&TcmConfig::NONE, addr, [0xEE]))
        .collect();
    assert_eq!(
        (stored, window_byte(&memory, VramWindowKind::BgA, 0)),
        (vec![false; 4], 0)
    );
}

#[test]
fn given_the_window_geometry_when_asked_then_it_matches_the_note() {
    use VramWindowKind::*;
    let geometry: Vec<(u32, u32)> = [BgA, ObjA, BgB, ObjB]
        .iter()
        .map(|k| (k.base(), k.size()))
        .collect();
    assert_eq!(
        geometry,
        vec![
            (0x0600_0000, 0x8_0000),
            (0x0640_0000, 0x4_0000),
            (0x0620_0000, 0x2_0000),
            (0x0660_0000, 0x2_0000)
        ]
    );
}
