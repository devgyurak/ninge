// SPDX-License-Identifier: Apache-2.0
//! DMA, observed through our own `dma` test ROM (`testroms/dma/`). Each CPU records what it reads
//! in main RAM (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; the slot layouts, the buffers and the
//! words copied are at the top of `arm9.s` and `arm7.s`). Expected values come from
//! `docs/hw/dma.md` ("Registers", "Behavior", "Edge cases", "Model", "Unverified") and
//! `docs/hw/interrupts.md` (IF bits), never from running the ROM.
//!
//! The source words are 11111111h, 22222222h, 33333333h, ... so a copy's result is read straight
//! off the destination. A destination word nothing wrote is 0 (main RAM is cleared by the ROM).
//!
//! Slots the note marks Unverified, or settles only as the model's choice, are in tests whose
//! names say so (`TODO(hw-unverified)`); their expected value is the model value in the note.

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_RECORD: u32 = 0x0200_4000;
const ARM7_RECORD: u32 = 0x0200_5000;

/// Frames to run: the ARM9 uses about five (three VBlanks, a full-frame HBlank count); the ARM7
/// about two.
const FRAMES: u32 = 10;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// Expected record: (slot, name, expected value).
type Row = (u32, &'static str, u32);

fn rom_machine() -> Machine {
    let rom = load_rom_file(&testrom_path("dma")).expect("dma ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("dma ROM has a valid header");
    machine
}

fn run(machine: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| machine.run_frame());
}

/// A machine that booted the `dma` ROM and ran long enough for both CPUs to finish.
fn boot() -> Machine {
    let mut machine = rom_machine();
    run(&mut machine, FRAMES);
    machine
}

fn mismatches(machine: &Machine, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    rows.iter()
        .map(|&(slot, name, expected)| (slot, name, expected, machine.arm9_peek32(base + slot * 4)))
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

fn arm9_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM9_RECORD, rows)
}

fn arm7_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM7_RECORD, rows)
}

// ------------------------------------------------------------------ completion

#[test]
fn given_dma_rom_when_run_then_the_arm9_finishes_and_writes_its_sentinel() {
    let rows: &[Row] = &[(62, "ARM9 progress", 1), (63, "ARM9 sentinel", 0x444D_4139)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_dma_rom_when_run_then_the_arm7_finishes_and_writes_its_sentinel() {
    let rows: &[Row] = &[(30, "ARM7 progress", 1), (31, "ARM7 sentinel", 0x444D_4137)];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ Immediate copies (ARM9)

#[test]
fn given_an_immediate_32_bit_copy_of_4_words_when_it_ends_then_exactly_those_words_moved() {
    // `dma.md`, "Transferring": count units, each read then written. Words 0 and 3 are the first
    // and last of SRC; word 4 is past the count, so it stays 0.
    let rows: &[Row] = &[
        (0, "DST1 word 0", 0x1111_1111),
        (1, "DST1 word 3", 0x4444_4444),
        (2, "DST1 word 4", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_immediate_16_bit_copy_of_3_units_when_it_ends_then_3_halfwords_moved() {
    // `dma.md`, control bit 26 = 0: 16-bit units. SRC halfwords are 1111h, 1111h, 2222h, ...,
    // so word 0 is 11111111h and word 1 holds 2222h in its low half only.
    let rows: &[Row] = &[
        (3, "DST2 word 0", 0x1111_1111),
        (4, "DST2 word 1", 0x0000_2222),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_destination_decrement_when_3_words_copy_then_they_land_downward_from_the_start() {
    // `dma.md`, "Transferring": decrement subtracts the unit size after each write.
    let rows: &[Row] = &[
        (5, "DST3 word 0", 0x1111_1111),
        (6, "DST3 word -1", 0x2222_2222),
        (7, "DST3 word -2", 0x3333_3333),
        (8, "DST3 word +1", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_fixed_destination_when_3_words_copy_then_the_last_word_is_left_in_place() {
    // `dma.md`, "Transferring": fixed means no change; each write overwrites the last.
    let rows: &[Row] = &[(9, "DST4 word 0", 0x3333_3333), (10, "DST4 word 1", 0)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_source_decrement_when_3_words_copy_from_word_2_then_they_arrive_in_reverse_order() {
    let rows: &[Row] = &[
        (11, "DST5 word 0", 0x3333_3333),
        (12, "DST5 word 1", 0x2222_2222),
        (13, "DST5 word 2", 0x1111_1111),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_fixed_source_when_3_words_copy_then_the_same_word_is_written_three_times() {
    let rows: &[Row] = &[
        (14, "DST6 word 0", 0x2222_2222),
        (15, "DST6 word 2", 0x2222_2222),
        (16, "DST6 word 3", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_unaligned_addresses_when_a_word_copies_then_both_are_aligned_down_to_the_unit() {
    // `dma.md`, "Transferring": addresses are aligned down to the unit's size. SRC+2 reads
    // SRC word 0; DST+2 writes DST word 0; word 1 is untouched.
    let rows: &[Row] = &[(26, "DST13 word 0", 0x1111_1111), (27, "DST13 word 1", 0)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ the busy flag

#[test]
fn given_a_finished_immediate_copy_when_enable_is_read_then_it_is_clear_and_the_word_moved() {
    // `dma.md`, "The end of a transfer" (3): enable is cleared when the transfer ends.
    let rows: &[Row] = &[
        (18, "bit 31 after the delay", 0),
        (19, "DST7 word 0", 0x1111_1111),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_immediate_copy_when_enable_is_read_right_after_the_write_then_it_is_clear_unverified() {
    // TODO(hw-unverified): `dma.md` says the transfer starts when the enable write retires and
    // the model runs it atomically then (and a real ARM9 is held until it ends), so the next
    // instruction reads 0. The note does not state the read-after-write value outright.
    let rows: &[Row] = &[(17, "bit 31 right after the enable write", 0)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ fill and registers

#[test]
fn given_a_fill_register_as_a_fixed_source_when_4_words_copy_then_all_hold_the_fill_word() {
    // `dma.md`, "Fill (ARM9)": an ordinary readable register used as a fixed source.
    let rows: &[Row] = &[
        (20, "DMA0FILL read back", 0xCAFE_F00D),
        (21, "DST8 word 0", 0xCAFE_F00D),
        (22, "DST8 word 3", 0xCAFE_F00D),
        (23, "DST8 word 4", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_copy_into_the_fill_registers_when_it_ends_then_they_hold_the_copied_words() {
    // `dma.md`, "Edge cases": a register as a destination takes effect as a register write
    // would, and the fill registers are ordinary readable registers.
    let rows: &[Row] = &[
        (24, "DMA1FILL after the copy", 0x1111_1111),
        (25, "DMA2FILL after the copy", 0x2222_2222),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ VBlank start and repeat

#[test]
fn given_an_armed_vblank_channel_when_line_191_is_reached_then_nothing_moved_and_enable_reads_1() {
    // `dma.md`, "Starting" (VBlank starts at line 192) and "Enabling" (enable reads 1 while
    // armed). Ch 1 is the repeating channel, ch 2 the one-shot.
    let rows: &[Row] = &[
        (28, "DST10 word 0 at line 191", 0),
        (29, "ch 2 bit 31 at line 191", 1),
        (30, "DST9 word 0 at line 191", 0),
        (31, "ch 1 bit 31 at line 191", 1),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_vblank_copy_of_2_words_when_line_195_is_reached_then_it_ran_with_its_latched_values() {
    // `dma.md`, "Enabling": a write that leaves enable at 1 changes the registers only, so
    // the transfer still copies 2 words from SRC to DST10, and the rewritten destination
    // (DST14, count 1, source SRC word 2) stays 0. Enable is clear at the end.
    let rows: &[Row] = &[
        (32, "DST10 word 0 at line 195", 0x1111_1111),
        (33, "DST10 word 1 at line 195", 0x2222_2222),
        (34, "DST14 word 0 at line 195", 0),
        (35, "ch 2 bit 31 at line 195", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_repeating_vblank_channel_when_3_vblanks_pass_then_the_source_advances_and_the_destination_reloads()
 {
    // `dma.md`, "The end of a transfer" (2): the count reloads, the destination reloads for
    // control 3, the source continues. So DST9 word 0 holds SRC word k after VBlank k + 1, word
    // 1 is never written, and enable stays 1 until the ROM clears it.
    let rows: &[Row] = &[
        (36, "DST9 word 0 after the first VBlank", 0x1111_1111),
        (37, "ch 1 bit 31 after the first VBlank", 1),
        (39, "DST9 word 0 after the second VBlank", 0x2222_2222),
        (42, "DST9 word 0 after the third VBlank", 0x3333_3333),
        (43, "DST9 word 1", 0),
        (44, "ch 1 bit 31 after the ROM cleared it", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_armed_vblank_channel_when_its_enable_is_cleared_before_line_192_then_it_never_runs() {
    // `dma.md`, "Enabling": a write that clears enable stops the channel; a transfer waiting
    // for its trigger never runs.
    let rows: &[Row] = &[(40, "DST15 word 0", 0), (41, "ch 2 bit 31", 0)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_two_vblank_channels_with_one_destination_when_vblank_starts_then_channel_3_writes_last_unverified()
 {
    // TODO(hw-unverified): `dma.md`, "Unverified" (same-time channel order): the model runs
    // channel 0 then 3, so channel 3's word (SRC word 1) is the one left behind.
    let rows: &[Row] = &[(38, "DST11 word 0", 0x2222_2222)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ HBlank start

#[test]
fn given_an_hblank_channel_when_50_visible_lines_pass_then_it_moved_50_units() {
    // `dma.md`, "Starting": HBlank start fires in each visible line. Enabled at the start of
    // line 10 and cleared at the start of line 60: lines 10-59, 50 units, one word each into a
    // growing destination (repeat, destination increment).
    let rows: &[Row] = &[(45, "non-zero words in HB1", 50)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_hblank_channel_when_a_whole_frame_passes_then_only_the_192_visible_lines_count_unverified()
 {
    // TODO(hw-unverified): `dma.md`, "Unverified" (HBlank start in the blank lines 192-262):
    // the model starts nothing there. From the start of line 100 to the start of line 100 of
    // the next frame the channel sees lines 100-191 and 0-99: 192 units.
    let rows: &[Row] = &[(46, "non-zero words in HB2", 192)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ IRQ (ARM9)

#[test]
fn given_channels_0_and_3_with_the_irq_bit_when_they_end_then_the_handler_saw_if_bits_8_and_11() {
    // `dma.md`, "IF bits": the end of channel n requests IF bit 8 + n when control bit 30 is
    // set; `interrupts.md` takes it through the vector. No earlier copy had the bit, so IF
    // bits 8-11 were clear before, and the handler acknowledged both at the end.
    let rows: &[Row] = &[
        (47, "IF & 0F00h before", 0),
        (48, "handler count", 2),
        (49, "OR of the IF values the handler saw", 0x900),
        (50, "IF & 0F00h after", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ DTCM

#[test]
fn given_a_dtcm_over_main_ram_when_dma_reads_it_then_it_sees_the_main_ram_behind_it() {
    // `dma.md`, "Transferring": DMA sees the bus with both TCM regions off. The DTCM at
    // 0x0270_0000 holds 0DD0DD0Dh for the CPU; behind it is main RAM, which mirrors
    // 0x0230_0000 (`arm9-memory-map.md`, "Main RAM"), holding BEEF0001h.
    let rows: &[Row] = &[
        (51, "CPU read of the DTCM", 0x0DD0_DD0D),
        (52, "DST12 word 0 after the DMA read", 0xBEEF_0001),
        (55, "main RAM behind the DTCM", 0xBEEF_0001),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_dtcm_over_main_ram_when_dma_writes_it_then_the_word_lands_behind_it_not_in_the_dtcm() {
    // `dma.md`, "Transferring": the write goes to the address behind the TCM, so the CPU
    // finds 11111111h at the mirror 0x0230_0010 and still 0 in the DTCM at 0x0270_0010.
    let rows: &[Row] = &[
        (
            53,
            "main RAM behind the DTCM after the DMA write",
            0x1111_1111,
        ),
        (54, "DTCM word the CPU wrote 0 to", 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ the ARM7's own channels

#[test]
fn given_an_arm7_immediate_copy_when_the_busy_flag_clears_then_the_3_words_are_there() {
    // `dma.md`, "Enabling" and "Model" (an ARM7 start can be late): the ROM polls bit 31.
    let rows: &[Row] = &[
        (0, "ARM7 DST1 word 0", 0x1111_1111),
        (1, "ARM7 DST1 word 2", 0x3333_3333),
        (2, "ARM7 DST1 word 3", 0),
        (3, "ARM7 bit 31 after the poll", 0),
    ];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_arm7_channel_2_count_of_4001h_when_it_copies_then_the_count_is_1() {
    // `dma.md`, "Word count": 14 bits for ARM7 channels 0-2, the bits above read 0. Bit 14 is
    // dropped, so one word moves and the low half of CNT reads 1.
    let rows: &[Row] = &[
        (4, "ARM7 DST2 word 0", 0x1111_1111),
        (5, "ARM7 DST2 word 1", 0),
        (6, "ARM7 ch 2 CNT low half", 1),
    ];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_arm7_copies_when_only_one_has_the_irq_bit_then_if_shows_just_that_channels_bit() {
    // `dma.md`, "IF bits": bit 8 + n only with control bit 30; channel 3 gives bit 11. IME is 0,
    // so IF stays visible; a 1 written to an IF bit clears it (`interrupts.md`).
    let rows: &[Row] = &[
        (7, "ARM7 IF & 0F00h before", 0),
        (8, "ARM7 IF & 0F00h after ch 3 ends", 0x800),
        (9, "ARM7 IF & 0F00h after the acknowledge", 0),
    ];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_arm7_vblank_copy_with_the_irq_bit_when_vblank_starts_then_it_runs_and_raises_if_bit_8()
{
    // `dma.md`, "Starting" (VBlank starts at line 192) and "IF bits" (channel 0 gives bit 8).
    let rows: &[Row] = &[
        (10, "ARM7 DST3 word 0 at line 191", 0),
        (11, "ARM7 ch 0 bit 31 at line 191", 1),
        (12, "ARM7 DST3 word 0 at line 195", 0x1111_1111),
        (13, "ARM7 DST3 word 1 at line 195", 0x2222_2222),
        (14, "ARM7 IF & 0F00h at line 195", 0x100),
        (15, "ARM7 ch 0 bit 31 at line 195", 0),
        (16, "ARM7 IF & 0F00h after the acknowledge", 0),
    ];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_the_arm7_when_it_writes_the_fill_register_address_then_the_read_is_0() {
    // `dma.md`, "Registers": the ARM7 has no fill registers; the block is not on its bus.
    let rows: &[Row] = &[(17, "ARM7 read of 0x0400_00E0 after a write", 0)];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ determinism

#[test]
fn given_the_dma_rom_when_run_twice_then_the_savestate_bytes_are_identical() {
    assert_eq!(boot().save_state(), boot().save_state());
}

#[test]
fn given_a_savestate_taken_mid_run_when_loaded_and_continued_then_it_matches_an_uninterrupted_run()
{
    // Taken after 2 frames, with VBlank and HBlank channels armed or repeating.
    let mut first = rom_machine();
    run(&mut first, 2);
    let state = first.save_state();
    let mut resumed = rom_machine();
    let loaded = resumed.load_state(&state);
    run(&mut resumed, FRAMES - 2);
    assert_eq!(
        (loaded, resumed.save_state()),
        (Ok(()), boot().save_state())
    );
}
