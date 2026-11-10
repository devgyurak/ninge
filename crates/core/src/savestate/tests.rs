// SPDX-License-Identifier: Apache-2.0
//! The savestate container (issue #19): magic, format version, ROM fingerprint, then the
//! encoded machine. Expected values come from the layout contract of the issue and from ARM
//! instruction encodings (ARM ARM, "A3 The ARM Instruction Set"); the FNV-1a constants are the
//! published ones (offset basis `cbf29ce484222325h`, prime `100000001b3h`). The fingerprint is
//! recomputed here by a reference loop so a test never depends on the code it checks.
//!
//! Byte layout: `0..8` magic, `8..12` version (LE), `12..20` ROM length (LE u64), `20..28`
//! FNV-1a 64 of the ROM (LE u64), `28..` the machine.

use super::*;
use crate::bus::{Region, VramBankState};
use crate::cart::test_image::{Image, words};
use crate::cpu::Bus;
use crate::irq::Requests;
use crate::{Machine, Screen};

/// Length and FNV-1a 64 of a power-on machine's savestate: pins the layout (see the test).
const PINNED_LEN: usize = 4_473_236;
const PINNED_HASH: u64 = 0xdf815eda4ec3325b;
/// The same pin for `counter_rom` after 2 frames: a non-`None` header, non-zero registers and
/// RAM, so a reordered `Header` or `Cpu` field changes it (the power-on pin cannot see those).
/// The hash moved when direct boot began setting WRAMCNT = 3 (one register byte and the
/// shared-WRAM contents); the length is unchanged. Version 6 (#104) added the display timing
/// state and a second queued event, so both pins moved. Version 7 (#24) added the IPC block:
/// 50 bytes at power-on (two FIFOs of 16 + 3 one-byte varints each, two ports of 6 one-byte fields),
/// so both lengths grew by 50. Version 8 (#25) added both CPUs' timers: 34 bytes at power-on
/// (4 timers of 4 one-byte fields and a one-byte dirty mask, twice), so both lengths grew by 34.
/// Version 9 (#109) split the devices out of the memory block without moving a byte and dropped the dirty mask: one byte per timer set, so both lengths
/// shrank by 2 and both hashes moved (the version field changed too).
/// Version 10 (#26) added both CPUs' DMA channels: 7 one-byte fields a channel and four fill
/// words for each CPU, so both lengths grew by 2 x (4 x 7 + 4) = 64 and both hashes moved.
const PINNED_BOOTED_LEN: usize = 4_473_323;
const PINNED_BOOTED_HASH: u64 = 0x7fb1a05bd2131e30;
/// Bytes before the encoded machine.
const HEADER_LEN: usize = 28;
/// The main-RAM word the counter program increments (outside the 0x100-byte ARM9 binary).
const COUNTER_ADDR: u32 = 0x0200_0800;
/// Bank A's LCDC window (`docs/hw/vram-banks.md`).
const LCDC_A: u32 = 0x0680_0000;

/// `MOV r2, #0x02000000`.
const MOV_R2_RAM: u32 = 0xE3A0_2402;
/// `ADD r2, r2, #0x800`.
const ADD_R2_800: u32 = 0xE282_2B02;
/// `LDR r3, [r2]`.
const LDR_R3_R2: u32 = 0xE592_3000;
/// `ADD r3, r3, #1`.
const ADD_R3_1: u32 = 0xE283_3001;
/// `STR r3, [r2]`.
const STR_R3_R2: u32 = 0xE582_3000;
/// `B` back to the `LDR`: the branch sits at word 5, R15 reads its address + 8, so the offset is
/// (2*4 - (5*4 + 8)) / 4 = -5 words.
const B_TO_LDR: u32 = 0xEAFF_FFFB;
/// `MOV r0, #0x04000000`.
const MOV_R0_IO: u32 = 0xE3A0_0301;
/// `ADD r0, r0, #0x240` (r0 = VRAMCNT_A).
const ADD_R0_240: u32 = 0xE280_0D09;
/// `MOV r1, #0x80` (enable, MST 0 = LCDC).
const MOV_R1_80: u32 = 0xE3A0_1080;
/// `STRB r1, [r0]`.
const STRB_R1_R0: u32 = 0xE5C0_1000;
/// `MOV r2, #0x06800000`.
const MOV_R2_LCDC: u32 = 0xE3A0_2668;
/// `STR r1, [r2]`.
const STR_R1_R2: u32 = 0xE582_1000;
/// `B .`.
const B_SELF: u32 = 0xEAFF_FFFE;

/// Published FNV-1a 64 constants, applied here by a reference loop.
fn reference_fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A ROM whose ARM9 increments the word at `COUNTER_ADDR` forever.
fn counter_rom() -> Vec<u8> {
    let mut image = Image::valid();
    image.arm9_code = words(&[
        MOV_R2_RAM, ADD_R2_800, LDR_R3_R2, ADD_R3_1, STR_R3_R2, B_TO_LDR,
    ]);
    image.build()
}

/// A ROM whose ARM9 enables VRAM bank A (MST 0) and stores 0x80 at the window start.
fn vram_rom() -> Vec<u8> {
    let mut image = Image::valid();
    image.arm9_code = words(&[
        MOV_R0_IO,
        ADD_R0_240,
        MOV_R1_80,
        STRB_R1_R0,
        MOV_R2_LCDC,
        STR_R1_R2,
        B_SELF,
    ]);
    image.build()
}

/// A ROM that differs from `counter_rom` only in one code byte.
fn other_rom() -> Vec<u8> {
    let mut image = Image::valid();
    image.arm9_code = words(&[MOV_R2_RAM, ADD_R2_800, B_SELF]);
    image.build()
}

/// A machine that booted `rom` and ran `frames` frames.
fn booted(rom: &[u8], frames: u32) -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(rom.to_vec()).expect("synthetic image");
    run_frames(&mut machine, frames);
    machine
}

/// Runs `frames` frames.
fn run_frames(machine: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| machine.run_frame());
}

/// `bytes` with `patch` written at `at`, growing the buffer if it is too short, so a stub that
/// returned nothing fails on the expected error rather than on an index panic.
fn patched(bytes: &[u8], at: usize, patch: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    if out.len() < at + patch.len() {
        out.resize(at + patch.len(), 0);
    }
    out[at..at + patch.len()].copy_from_slice(patch);
    out
}

/// The cut points whose truncated state is not refused as `Corrupt`.
fn cuts_not_refused_as_corrupt(rom: &[u8], bytes: &[u8], cuts: &[usize]) -> Vec<usize> {
    cuts.iter()
        .copied()
        .filter(|&cut| {
            let mut target = booted(rom, 0);
            let head = &bytes[..cut.min(bytes.len())];
            target.load_state(head) != Err(SavestateError::Corrupt)
        })
        .collect()
}

/// The extra-byte tails whose appended state is not refused as `Corrupt`.
fn tails_not_refused_as_corrupt(rom: &[u8], bytes: &[u8], tails: &[&[u8]]) -> Vec<usize> {
    tails
        .iter()
        .enumerate()
        .filter(|(_, tail)| {
            let mut target = booted(rom, 0);
            let extended = [bytes, tail].concat();
            target.load_state(&extended) != Err(SavestateError::Corrupt)
        })
        .map(|(i, _)| i)
        .collect()
}

// ------------------------------------------------------------------- container layout

#[test]
fn given_a_machine_with_a_rom_when_saved_then_the_header_holds_magic_version_and_fingerprint() {
    let rom = counter_rom();
    let bytes = booted(&rom, 1).save_state();
    let header = bytes[..HEADER_LEN.min(bytes.len())].to_vec();
    let expected = [
        b"NINGESTA".as_slice(),
        &10u32.to_le_bytes(),
        &(rom.len() as u64).to_le_bytes(),
        &reference_fnv1a64(&rom).to_le_bytes(),
    ]
    .concat();
    assert_eq!(header, expected);
}

#[test]
fn given_a_machine_with_a_rom_when_saved_then_a_body_follows_the_header() {
    let bytes = booted(&counter_rom(), 1).save_state();
    assert!(bytes.len() > HEADER_LEN);
}

#[test]
fn given_a_machine_without_a_rom_when_saved_then_the_fingerprint_is_the_empty_rom_one() {
    let bytes = Machine::new().save_state();
    let fingerprint = bytes[12..HEADER_LEN.min(bytes.len()).max(12)].to_vec();
    let expected = [0u64.to_le_bytes(), reference_fnv1a64(&[]).to_le_bytes()].concat();
    assert_eq!(fingerprint, expected);
}

// ------------------------------------------------------------------- refusals

#[test]
fn given_a_state_with_a_wrong_magic_when_loaded_then_it_is_refused_as_bad_magic() {
    let rom = counter_rom();
    let bytes = patched(&booted(&rom, 0).save_state(), 0, b"XINGESTA");
    let result = booted(&rom, 0).load_state(&bytes);
    assert_eq!(result, Err(SavestateError::BadMagic));
}

#[test]
fn given_a_state_with_another_format_version_when_loaded_then_it_is_refused_with_both_versions() {
    let rom = counter_rom();
    let bytes = patched(&booted(&rom, 0).save_state(), 8, &11u32.to_le_bytes());
    let result = booted(&rom, 0).load_state(&bytes);
    assert_eq!(
        result,
        Err(SavestateError::VersionMismatch {
            found: 11,
            expected: 10
        })
    );
}

#[test]
fn given_a_version_mismatch_when_displayed_then_the_message_names_both_versions() {
    let rom = counter_rom();
    let bytes = patched(&booted(&rom, 0).save_state(), 8, &11u32.to_le_bytes());
    let message = booted(&rom, 0)
        .load_state(&bytes)
        .expect_err("version 11 is not version 10")
        .to_string();
    assert!(message.contains("version 11") && message.contains("version 10"));
}

#[test]
fn given_a_state_cut_at_several_points_when_loaded_then_every_cut_is_refused_as_corrupt() {
    let rom = counter_rom();
    let bytes = booted(&rom, 1).save_state();
    // Empty, inside the magic, inside the version, inside the fingerprint, header only, one
    // byte into the body, mid-body, and one byte short.
    let cuts = [
        0,
        4,
        10,
        20,
        HEADER_LEN,
        HEADER_LEN + 1,
        bytes.len() / 2 + 14,
        bytes.len().saturating_sub(1),
    ];
    assert_eq!(
        cuts_not_refused_as_corrupt(&rom, &bytes, &cuts),
        Vec::<usize>::new()
    );
}

#[test]
fn given_a_state_followed_by_extra_bytes_when_loaded_then_every_tail_is_refused_as_corrupt() {
    let rom = counter_rom();
    let bytes = booted(&rom, 1).save_state();
    let tails: [&[u8]; 3] = [&[0], &[0xFF, 0xFF], &[7; 64]];
    assert_eq!(
        tails_not_refused_as_corrupt(&rom, &bytes, &tails),
        Vec::<usize>::new()
    );
}

#[test]
fn given_a_state_from_another_rom_when_loaded_then_it_is_refused_as_rom_mismatch() {
    let bytes = booted(&counter_rom(), 1).save_state();
    let result = booted(&other_rom(), 0).load_state(&bytes);
    assert_eq!(result, Err(SavestateError::RomMismatch));
}

#[test]
fn given_a_state_from_a_longer_rom_when_loaded_then_it_is_refused_as_rom_mismatch() {
    let mut image = Image::valid();
    image.len = Some(0x8000 + 0x80 + 0x1000);
    let longer = image.build();
    let bytes = booted(&longer, 0).save_state();
    let shorter = Image::valid().build();
    let result = booted(&shorter, 0).load_state(&bytes);
    assert_eq!(result, Err(SavestateError::RomMismatch));
}

#[test]
fn given_a_state_taken_with_a_rom_when_loaded_onto_a_machine_without_one_then_it_is_rom_mismatch() {
    let bytes = booted(&counter_rom(), 1).save_state();
    let result = Machine::new().load_state(&bytes);
    assert_eq!(result, Err(SavestateError::RomMismatch));
}

#[test]
fn given_a_state_taken_without_a_rom_when_loaded_onto_a_machine_with_one_then_it_is_rom_mismatch() {
    let bytes = Machine::new().save_state();
    let result = booted(&counter_rom(), 0).load_state(&bytes);
    assert_eq!(result, Err(SavestateError::RomMismatch));
}

// ------------------------------------------------------------------- failed load leaves no trace

#[test]
fn given_a_bad_magic_when_loaded_then_the_target_machine_is_unchanged() {
    let rom = counter_rom();
    let bytes = patched(&booted(&rom, 2).save_state(), 0, b"XINGESTA");
    let mut target = booted(&rom, 1);
    let before = target.clone();
    let result = target.load_state(&bytes);
    assert_eq!(
        (result, target == before),
        (Err(SavestateError::BadMagic), true)
    );
}

#[test]
fn given_a_rom_mismatch_when_loaded_then_the_target_machine_is_unchanged() {
    let bytes = booted(&counter_rom(), 2).save_state();
    let mut target = booted(&other_rom(), 1);
    let before = target.clone();
    let result = target.load_state(&bytes);
    assert_eq!(
        (result, target == before),
        (Err(SavestateError::RomMismatch), true)
    );
}

#[test]
fn given_a_truncated_body_when_loaded_then_the_target_machine_is_unchanged() {
    let rom = counter_rom();
    let bytes = booted(&rom, 2).save_state();
    let cut =
        &bytes[..(HEADER_LEN + (bytes.len() - HEADER_LEN.min(bytes.len())) / 2).min(bytes.len())];
    let mut target = booted(&rom, 1);
    let before = target.clone();
    let result = target.load_state(cut);
    assert_eq!(
        (result, target == before),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_whole_state_with_trailing_bytes_when_loaded_then_the_target_machine_is_unchanged() {
    let rom = counter_rom();
    let bytes = [booted(&rom, 2).save_state(), vec![0xAA, 0xBB]].concat();
    let mut target = booted(&rom, 1);
    let before = target.clone();
    let result = target.load_state(&bytes);
    assert_eq!(
        (result, target == before),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_garbage_after_a_valid_header_when_loaded_then_it_is_refused_as_corrupt_without_panic() {
    let rom = counter_rom();
    let header = booted(&rom, 0).save_state();
    let bytes = [&header[..HEADER_LEN.min(header.len())], &[0xFF; 300][..]].concat();
    let result = booted(&rom, 0).load_state(&bytes);
    assert_eq!(result, Err(SavestateError::Corrupt));
}

// ------------------------------------------------------------------- round trips

#[test]
fn given_a_saved_machine_when_loaded_and_saved_again_then_the_bytes_are_identical() {
    let rom = counter_rom();
    let first = booted(&rom, 2).save_state();
    let mut target = booted(&rom, 0);
    let result = target.load_state(&first);
    let second = target.save_state();
    assert_eq!(
        (result, second == first, first.is_empty()),
        (Ok(()), true, false)
    );
}

#[test]
fn given_a_machine_running_the_counter_program_when_a_frame_runs_then_the_counter_is_non_zero() {
    // Guards the round-trip test below against a vacuous program: the ARM9 loop
    // LDR / ADD #1 / STR must have incremented the word at least once.
    let machine = booted(&counter_rom(), 1);
    assert_ne!(machine.arm9_peek32(COUNTER_ADDR), 0);
}

#[test]
fn given_a_state_saved_after_2_frames_when_loaded_and_run_2_more_then_it_equals_a_4_frame_run() {
    let rom = counter_rom();
    let saved_at_2 = booted(&rom, 2);
    let counter_at_2 = saved_at_2.arm9_peek32(COUNTER_ADDR);
    let bytes = saved_at_2.save_state();

    let mut resumed = booted(&rom, 0);
    let result = resumed.load_state(&bytes);
    run_frames(&mut resumed, 2);

    let straight = booted(&rom, 4);
    assert_eq!(
        (
            result,
            resumed.save_state() == straight.save_state(),
            resumed.frame(Screen::Top) == straight.frame(Screen::Top),
            resumed.frame(Screen::Bottom) == straight.frame(Screen::Bottom),
            resumed.frame_count(),
            resumed.arm9_peek32(COUNTER_ADDR) > counter_at_2,
        ),
        (Ok(()), true, true, true, 4, true)
    );
}

#[test]
fn given_a_machine_with_drawn_frames_when_state_is_loaded_then_both_screens_are_black() {
    let rom = counter_rom();
    let bytes = booted(&rom, 1).save_state();
    let mut target = booted(&rom, 0);
    let result = target.load_state(&bytes);
    let black = |screen| target.frame(screen).iter().all(|&p| p == 0);
    let sizes = [
        target.frame(Screen::Top).len(),
        target.frame(Screen::Bottom).len(),
    ];
    assert_eq!(
        (result, black(Screen::Top), black(Screen::Bottom), sizes),
        (Ok(()), true, true, [49_152, 49_152])
    );
}

#[test]
fn given_a_state_with_frames_run_when_loaded_onto_a_fresh_machine_then_frame_count_matches() {
    let rom = counter_rom();
    let bytes = booted(&rom, 3).save_state();
    let mut target = booted(&rom, 0);
    let result = target.load_state(&bytes);
    assert_eq!((result, target.frame_count()), (Ok(()), 3));
}

#[test]
fn given_a_machine_without_a_rom_when_saved_and_loaded_then_it_equals_the_original() {
    let mut original = Machine::new();
    run_frames(&mut original, 3);
    let mut target = Machine::new();
    let result = target.load_state(&original.save_state());
    assert_eq!((result, target == original), (Ok(()), true));
}

// ------------------------------------------------------------------- derived state is rebuilt

#[test]
fn given_a_loaded_state_when_main_ram_is_peeked_then_it_matches_the_saved_machine() {
    let rom = counter_rom();
    let saved = booted(&rom, 1);
    let mut target = booted(&rom, 0);
    let result = target.load_state(&saved.save_state());
    let peeks = |m: &Machine| [m.arm9_peek32(0x0200_0000), m.arm9_peek32(COUNTER_ADDR)];
    assert_eq!(
        (result, peeks(&target), peeks(&saved)),
        (Ok(()), peeks(&saved), peeks(&saved))
    );
}

#[test]
fn given_a_state_with_vram_a_mapped_when_loaded_onto_an_unmapped_machine_then_the_window_reads_it()
{
    let rom = vram_rom();
    let saved = booted(&rom, 1);
    let mut target = booted(&rom, 0);
    let before = target.arm9_peek32(LCDC_A);
    let result = target.load_state(&saved.save_state());
    assert_eq!(
        (result, before, target.arm9_peek32(LCDC_A), target.vram_a()),
        (
            Ok(()),
            0,
            0x80,
            VramBankState {
                enabled: true,
                mst: 0,
                offset: 0
            }
        )
    );
}

#[test]
fn given_a_state_with_vram_a_unmapped_when_loaded_onto_a_mapped_machine_then_the_window_reads_zero()
{
    let rom = vram_rom();
    let bytes = booted(&rom, 0).save_state();
    let mut target = booted(&rom, 1);
    let before = target.arm9_peek32(LCDC_A);
    let result = target.load_state(&bytes);
    assert_eq!(
        (
            result,
            before,
            target.arm9_peek32(LCDC_A),
            target.vram_a().enabled
        ),
        (Ok(()), 0x80, 0, false)
    );
}

// ------------------------------------------------------------------------------ FNV-1a vectors

#[test]
fn given_the_empty_input_when_hashed_with_fnv1a64_then_it_is_the_offset_basis() {
    assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn given_the_input_a_when_hashed_with_fnv1a64_then_it_matches_the_published_vector() {
    assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
}

#[test]
fn given_the_input_foobar_when_hashed_with_fnv1a64_then_it_matches_the_published_vector() {
    assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
}

// ------------------------------------------------------------------------------- pinned layout

#[test]
fn given_a_power_on_machine_when_saved_then_len_and_hash_match_the_pinned_layout() {
    let bytes = Machine::new().save_state();
    assert_eq!(
        (bytes.len(), fnv1a64(&bytes)),
        (PINNED_LEN, PINNED_HASH),
        "the savestate layout changed. If the change is intended, bump FORMAT_VERSION in \
         savestate.rs and update PINNED_LEN and PINNED_HASH here; if not, the saved state \
         changed by accident"
    );
}

#[test]
fn given_a_booted_machine_after_two_frames_when_saved_then_len_and_hash_match_the_pinned_layout() {
    let bytes = booted(&counter_rom(), 2).save_state();
    assert_eq!(
        (bytes.len(), fnv1a64(&bytes)),
        (PINNED_BOOTED_LEN, PINNED_BOOTED_HASH),
        "the savestate layout changed (header, registers or RAM). If the change is intended, \
         bump FORMAT_VERSION in savestate.rs and update the pinned values here; if not, the \
         saved state changed by accident"
    );
}

#[test]
fn given_a_state_whose_register_bank_disagrees_with_its_cpsr_mode_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.arm9.regs_mut().break_bank_for_test();
    let mut target = booted(&rom, 0);
    let before = target.clone();
    let result = target.load_state(&saver.save_state());
    assert_eq!(
        (result, target == before),
        (Err(SavestateError::Corrupt), true)
    );
}

// ------------------------------------------------- the scheduler and the ARM7 (#21)

/// The format version that adds the ARM7, its clock and the scheduler (`scheduler.md`).
#[test]
fn given_a_state_saved_by_version_2_when_loaded_then_it_is_refused_and_version_10_is_expected() {
    let rom = counter_rom();
    let bytes = patched(&booted(&rom, 0).save_state(), 8, &2u32.to_le_bytes());
    let result = booted(&rom, 0).load_state(&bytes);
    assert_eq!(
        result,
        Err(SavestateError::VersionMismatch {
            found: 2,
            expected: 10
        })
    );
}

/// The format version that adds WRAMCNT and the 32 KiB of shared WRAM (`wramcnt.md`).
#[test]
fn given_a_state_saved_by_version_3_when_loaded_then_it_is_refused_and_version_10_is_expected() {
    let rom = counter_rom();
    let bytes = patched(&booted(&rom, 0).save_state(), 8, &3u32.to_le_bytes());
    let result = booted(&rom, 0).load_state(&bytes);
    assert_eq!(
        result,
        Err(SavestateError::VersionMismatch {
            found: 3,
            expected: 10
        })
    );
}

#[test]
fn given_a_saved_state_when_its_version_field_is_read_then_it_is_9() {
    let bytes = booted(&counter_rom(), 0).save_state();
    assert_eq!(bytes[8..12], 10u32.to_le_bytes());
}

/// Steps the ARM9 `count` times.
fn step_arm9_times(machine: &mut Machine, count: u32) {
    (0..count).for_each(|_| machine.step_arm9());
}

/// Loads `saver`'s state onto a fresh machine with the same ROM; returns the result and whether
/// the target was left exactly as it was.
fn load_onto_fresh(rom: &[u8], saver: &Machine) -> (Result<(), SavestateError>, bool) {
    let mut target = booted(rom, 0);
    let before = target.clone();
    let result = target.load_state(&saver.save_state());
    (result, target == before)
}

#[test]
fn given_a_state_whose_sequence_counter_is_not_above_a_queued_seq_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    let queued = saver.scheduler.pending()[0].seq;
    saver.scheduler.next_seq = queued;
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_queued_event_before_now_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    // The queued line events lie just past one frame; move the clock (and the ARM7 with it, so
    // that only the event rule is broken) one cycle past them.
    let past = crate::scheduler::FRAME_CYCLES + crate::scheduler::LINE_CYCLES + 1;
    saver.scheduler.now = past;
    saver.arm7_clock = past;
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_whose_arm7_register_bank_disagrees_with_its_cpsr_mode_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.arm7.regs_mut().break_bank_for_test();
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_whose_arm7_clock_is_3_cycles_ahead_of_now_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.arm7_clock = saver.scheduler.now + 3;
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_whose_arm7_clock_is_one_cycle_ahead_of_now_when_loaded_then_it_is_accepted() {
    // One ARM7 instruction that starts before `now` may end one master cycle past it.
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.arm7_clock = saver.scheduler.now + 1;
    assert_eq!(load_onto_fresh(&rom, &saver).0, Ok(()));
}

#[test]
fn given_a_machine_stopped_mid_frame_when_saved_and_loaded_then_the_clocks_and_queue_come_back() {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    step_arm9_times(&mut saver, 1001);
    let mut target = booted(&rom, 0);
    let result = target.load_state(&saver.save_state());
    assert_eq!(
        (
            result,
            target.scheduler().now(),
            target.arm7_clock(),
            target.scheduler().pending()
        ),
        (
            Ok(()),
            saver.scheduler().now(),
            saver.arm7_clock(),
            saver.scheduler().pending()
        )
    );
}

#[test]
fn given_a_booted_state_with_no_line_start_queued_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    // Without a LineStart, `run_frame` would never see a frame finish.
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.scheduler.pop_due(u64::MAX);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

/// A booted machine whose queue holds an HBlank start at `now` and line starts at `events`
/// (times), at clock `now`.
fn with_queue(rom: &[u8], now: u64, events: &[u64]) -> Machine {
    let mut machine = booted(rom, 1);
    machine.scheduler.pop_due(u64::MAX);
    machine.scheduler.now = now;
    machine.arm7_clock = now;
    machine
        .scheduler
        .schedule(now, crate::scheduler::EventKind::HBlankStart);
    events.iter().for_each(|&time| {
        machine
            .scheduler
            .schedule(time, crate::scheduler::EventKind::LineStart)
    });
    machine
}

#[test]
fn given_a_state_with_two_line_starts_at_the_same_time_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    // Both would fire in one round and a line would be skipped.
    let rom = counter_rom();
    let saver = with_queue(&rom, 100, &[5_000, 5_000]);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_whose_clock_is_near_u64_max_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    // The next line start would saturate at u64::MAX and stay due forever.
    let rom = counter_rom();
    let now = u64::MAX - 10;
    let saver = with_queue(&rom, now, &[u64::MAX - 5]);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_line_start_more_than_a_line_ahead_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let saver = with_queue(&rom, 100, &[100 + (1 << 60)]);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_booted_state_whose_arm7_lags_the_clock_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    // A booted machine is always settled: the ARM7 is at or just past `now`.
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.arm7_clock = 0;
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_the_latest_state_the_old_margin_allowed_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    // now = MAX - 2F with the line start at MAX - F passed the first margin check; the line
    // after it saturates at u64::MAX and stays due.
    let rom = counter_rom();
    let frame = crate::scheduler::FRAME_CYCLES;
    let now = u64::MAX - 2 * frame;
    let saver = with_queue(&rom, now, &[u64::MAX - frame]);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_clock_just_above_the_time_limit_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let now = crate::scheduler::MAX_TIME + 1;
    let saver = with_queue(&rom, now, &[now + 10]);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

// ------------------------------------------------------ shared WRAM and WRAMCNT (#22)

/// Where shared WRAM starts in both CPUs' windows (`wramcnt.md`, "Behavior").
const SHARED: u32 = 0x0300_0000;
/// WRAMCNT (`wramcnt.md`, "Registers").
const WRAMCNT_ADDR: u32 = 0x0400_0247;
/// A machine whose WRAMCNT is `mode` and whose shared WRAM holds a word at the start of each
/// 16 KiB half. A booted machine is in mode 3 (direct boot, `wramcnt.md`), so the mode is first
/// set to 0 (the whole 32 KiB is the ARM9's) for the writes, then to `mode`.
fn with_wram(mode: u8) -> Machine {
    let mut machine = booted(&counter_rom(), 0);
    machine.bus9().write8(WRAMCNT_ADDR, 0);
    machine.bus9().write32(SHARED, 0xA1A1_A1A1);
    machine.bus9().write32(SHARED + 0x4000, 0xB2B2_B2B2);
    machine.bus9().write8(WRAMCNT_ADDR, mode);
    machine
}

/// What each CPU reads at the start of the window and of the second half.
fn wram_reads(machine: &Machine) -> [u32; 4] {
    [
        machine.arm9_peek32(SHARED),
        machine.arm9_peek32(SHARED + 0x4000),
        machine.arm7_peek32(0, SHARED),
        machine.arm7_peek32(0, SHARED + 0x4000),
    ]
}

#[test]
fn given_a_state_with_wramcnt_1_when_loaded_onto_a_mode_3_machine_then_the_mode_and_both_views_come_back()
 {
    let saved = with_wram(1);
    let mut target = with_wram(3);
    let before = (
        target.memory.region(SHARED),
        target.memory.region7(SHARED + 0x4000),
    );
    let result = target.load_state(&saved.save_state());
    // Mode 1: the ARM9 owns the second half (offset 0x4000 of the 32 KiB, 0xB2B2_B2B2) and the
    // ARM7 the first (0xA1A1_A1A1); each half repeats every 16 KiB through the window.
    assert_eq!(
        (result, before, wram_reads(&target)),
        (
            Ok(()),
            (Region::Unmapped, Region::SharedWram),
            [0xB2B2_B2B2, 0xB2B2_B2B2, 0xA1A1_A1A1, 0xA1A1_A1A1]
        )
    );
}

#[test]
fn given_a_state_with_wramcnt_1_when_loaded_then_both_page_tables_are_rebuilt() {
    let saved = with_wram(1);
    let mut target = booted(&counter_rom(), 0);
    let before = (target.memory.region(SHARED), target.memory.wramcnt().mode);
    let result = target.load_state(&saved.save_state());
    // Direct boot leaves mode 3: the ARM7 sees all of the shared WRAM at 0300_0000 and the ARM9
    // none of it; mode 1 splits it, giving the ARM9 the second half.
    assert_eq!(
        (
            result,
            before,
            target.memory.region(SHARED),
            target.memory.region7(SHARED),
            target.memory.region7(0x0380_0000),
            target.memory.wramcnt().mode,
            // Mode 1 gives the ARM7 the first half, which repeats every 16 KiB; mode 3 would
            // show the second half here (0xB2B2_B2B2).
            target.arm7_peek32(0, SHARED + 0x4000)
        ),
        (
            Ok(()),
            (Region::Unmapped, 3),
            Region::SharedWram,
            Region::SharedWram,
            Region::Wram7,
            1,
            0xA1A1_A1A1
        )
    );
}

#[test]
fn given_a_state_with_wramcnt_3_when_loaded_onto_a_mode_0_machine_then_the_arm9_window_is_empty() {
    let saved = with_wram(3);
    let mut target = with_wram(0);
    let before = target.memory.region(SHARED);
    let result = target.load_state(&saved.save_state());
    assert_eq!(
        (
            result,
            before,
            target.memory.region(SHARED),
            wram_reads(&target)
        ),
        (
            Ok(()),
            Region::SharedWram,
            Region::Unmapped,
            [0, 0, 0xA1A1_A1A1, 0xB2B2_B2B2]
        )
    );
}

#[test]
fn given_a_state_with_shared_wram_filled_when_saved_loaded_and_saved_again_then_the_bytes_match() {
    let saved = with_wram(2).save_state();
    let mut target = booted(&counter_rom(), 0);
    let result = target.load_state(&saved);
    assert_eq!((result, target.save_state() == saved), (Ok(()), true));
}

// ------------------------------------------------------------------- interrupt controllers
//
// Expected values come from `docs/hw/interrupts.md`: IE keeps only the CPU's valid bits
// (ARM9 `003F_3F7Fh`, ARM7 `01DF_3FFFh`), IF latches only valid bits, IME keeps bit 0.

const IRQ_IME: u32 = 0x0400_0208;
const IRQ_IE: u32 = 0x0400_0210;

/// A booted counter machine, `frames` frames in, with non-default IME, IE and IF on both CPUs:
/// ARM9 IME 1, IE all ones (kept as the ARM9 mask), IF bits 0 and 3; ARM7 IME 0, IE bit 7 and
/// bit 16, IF bit 7 and bit 24. The ARM9's I flag is set, so no IRQ is taken.
fn with_irq_state(frames: u32) -> Machine {
    let mut machine = booted(&counter_rom(), frames);
    machine.bus9().write32(IRQ_IME, 0xFFFF_FFFF);
    machine.bus9().write32(IRQ_IE, 0xFFFF_FFFF);
    machine.bus7(0).write32(IRQ_IE, 1 << 7 | 1 << 16);
    machine.io.request_irq(Requests {
        arm9: 0b1001,
        arm7: 0,
    });
    machine.io.request_irq(Requests {
        arm9: 0,
        arm7: 1 << 7 | 1 << 24,
    });
    machine
}

/// (ARM9 IME, IE, IF, ARM7 IME, IE, IF).
type IrqState = (bool, u32, u32, bool, u32, u32);

fn irq_state(machine: &Machine) -> IrqState {
    let (a9, a7) = (machine.irq9(), machine.irq7());
    (a9.ime(), a9.ie(), a9.if_(), a7.ime(), a7.ie(), a7.if_())
}

const IRQ_STATE: IrqState = (
    true,
    0x003F_3F7F,
    0b1001,
    false,
    1 << 7 | 1 << 16,
    1 << 7 | 1 << 24,
);

#[test]
fn given_non_default_interrupt_registers_when_set_up_then_both_controllers_hold_the_expected_values()
 {
    // Guards the round-trip tests below against a vacuous setup.
    assert_eq!(irq_state(&with_irq_state(1)), IRQ_STATE);
}

#[test]
fn given_non_default_interrupt_registers_when_saved_and_loaded_onto_a_fresh_machine_then_both_controllers_come_back()
 {
    let saved = with_irq_state(1);
    let mut target = booted(&counter_rom(), 0);
    let result = target.load_state(&saved.save_state());
    assert_eq!((result, irq_state(&target)), (Ok(()), IRQ_STATE));
}

#[test]
fn given_non_default_interrupt_registers_when_loaded_and_saved_again_then_the_bytes_are_identical()
{
    let first = with_irq_state(1).save_state();
    let mut target = booted(&counter_rom(), 0);
    let result = target.load_state(&first);
    assert_eq!(
        (result, target.save_state() == first, first.is_empty()),
        (Ok(()), true, false)
    );
}

#[test]
fn given_a_state_with_interrupt_registers_when_loaded_over_a_machine_with_others_then_the_loaded_ones_win()
 {
    let saved = with_irq_state(1);
    let mut target = booted(&counter_rom(), 0);
    target.io.request_irq(Requests {
        arm9: 1 << 16,
        arm7: 0,
    });
    target.io.request_irq(Requests {
        arm9: 0,
        arm7: 1 << 8,
    });
    let result = target.load_state(&saved.save_state());
    assert_eq!((result, irq_state(&target)), (Ok(()), IRQ_STATE));
}

#[test]
fn given_the_same_rom_and_interrupt_setup_when_run_twice_then_the_savestate_bytes_are_identical() {
    let mut first = with_irq_state(1);
    let mut second = with_irq_state(1);
    run_frames(&mut first, 2);
    run_frames(&mut second, 2);
    assert_eq!(
        (
            first.save_state() == second.save_state(),
            irq_state(&first),
            irq_state(&second)
        ),
        (true, IRQ_STATE, IRQ_STATE)
    );
}

#[test]
fn given_a_state_with_interrupt_registers_saved_mid_run_when_loaded_and_continued_then_it_equals_an_uninterrupted_run()
 {
    let bytes = with_irq_state(1).save_state();
    let mut resumed = booted(&counter_rom(), 0);
    let result = resumed.load_state(&bytes);
    run_frames(&mut resumed, 2);

    let mut straight = with_irq_state(1);
    run_frames(&mut straight, 2);
    assert_eq!(
        (
            result,
            resumed.save_state() == straight.save_state(),
            irq_state(&resumed)
        ),
        (Ok(()), true, IRQ_STATE)
    );
}

// ------------------------------------------------------------------ display timing (#104)

#[test]
fn given_a_state_with_vcount_263_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged() {
    // `docs/hw/display-timing.md`: there are 263 lines, 0-262.
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.io.timing_mut().set_vcount_for_test(263);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_whose_vblank_flag_disagrees_with_vcount_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.io.timing_mut().set_vblank_for_test(true);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_dispstat_bit_that_does_not_exist_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    // Bit 0 (the VBlank flag) is derived, never stored: only `FFB8h` can be written.
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.io.timing_mut().set_arm7_control_for_test(0x0001);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_booted_state_with_no_hblank_start_queued_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver
        .scheduler
        .pop_due(crate::scheduler::FRAME_CYCLES + crate::scheduler::HBLANK_START_CYCLES);
    assert_eq!(
        load_onto_fresh(&rom, &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

// ------------------------------------------------------------------------- IPC (#24)

const IPC_CNT: u32 = 0x0400_0184;
const IPC_SEND: u32 = 0x0400_0188;
const IPC_RECV: u32 = 0x0410_0000;

/// A booted counter machine, `frames` frames in, with both FIFOs enabled and the ARM9 having sent
/// three words (`ipc.md`, "Behavior"); the ARM7 has taken none, and has its receive IRQ on.
fn with_words_in_flight(frames: u32) -> Machine {
    let mut machine = booted(&counter_rom(), frames);
    machine.bus9().write16(IPC_CNT, 1 << 15);
    machine.bus7(0).write16(IPC_CNT, 1 << 15 | 1 << 10);
    [0x11, 0x2222, 0x33_3333]
        .into_iter()
        .for_each(|word| machine.bus9().write32(IPC_SEND, word));
    machine
}

/// The three words the ARM7 takes from `machine`, then the error flag of its IPCFIFOCNT.
fn arm7_takes_three(machine: &mut Machine) -> ([u32; 3], u16) {
    let words = std::array::from_fn(|_| machine.bus7(0).read32(IPC_RECV));
    (words, machine.arm7_peek16(0, IPC_CNT) & (1 << 14))
}

#[test]
fn given_words_in_flight_when_saved_and_loaded_onto_a_fresh_machine_then_the_arm7_receives_them_in_order()
 {
    let saved = with_words_in_flight(1);
    let mut target = booted(&counter_rom(), 0);
    let result = target.load_state(&saved.save_state());
    assert_eq!(
        (result, arm7_takes_three(&mut target)),
        (Ok(()), ([0x11, 0x2222, 0x33_3333], 0))
    );
}

#[test]
fn given_words_in_flight_when_loaded_and_saved_again_then_the_bytes_are_identical() {
    let first = with_words_in_flight(1).save_state();
    let mut target = booted(&counter_rom(), 0);
    let result = target.load_state(&first);
    assert_eq!((result, target.save_state() == first), (Ok(()), true));
}

#[test]
fn given_words_in_flight_when_saved_mid_run_and_continued_then_it_equals_an_uninterrupted_run() {
    let bytes = with_words_in_flight(1).save_state();
    let mut resumed = booted(&counter_rom(), 0);
    let result = resumed.load_state(&bytes);
    run_frames(&mut resumed, 2);
    let mut straight = with_words_in_flight(1);
    run_frames(&mut straight, 2);
    assert_eq!(
        (result, resumed.save_state() == straight.save_state()),
        (Ok(()), true)
    );
}

#[test]
fn given_the_same_rom_and_ipc_traffic_when_run_twice_then_the_savestate_bytes_are_identical() {
    let mut first = with_words_in_flight(1);
    let mut second = with_words_in_flight(1);
    run_frames(&mut first, 2);
    run_frames(&mut second, 2);
    assert!(first.save_state() == second.save_state());
}

#[test]
fn given_words_in_flight_when_the_state_loads_over_a_machine_with_an_empty_fifo_then_the_loaded_words_win()
 {
    let saved = with_words_in_flight(1);
    let mut target = booted(&counter_rom(), 0);
    let result = target.load_state(&saved.save_state());
    assert_eq!(
        (
            result,
            target.ipc().send_words::<true>().collect::<Vec<u32>>()
        ),
        (Ok(()), vec![0x11, 0x2222, 0x33_3333])
    );
}

/// Loads a state whose ARM9 IPC block is `ipc` onto a fresh machine; returns the result and
/// whether the target was left as it was.
fn load_with_ipc(ipc: crate::ipc::Ipc) -> (Result<(), SavestateError>, bool) {
    let rom = counter_rom();
    let mut saver = booted(&rom, 1);
    saver.io.replace_ipc_for_test(ipc);
    load_onto_fresh(&rom, &saver)
}

#[test]
fn given_a_state_with_a_fifo_length_of_17_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    assert_eq!(
        load_with_ipc(crate::ipc::Ipc::with_corrupt_fifo_len(17)),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_fifo_head_of_16_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
{
    assert_eq!(
        load_with_ipc(crate::ipc::Ipc::with_corrupt_fifo_head(16)),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_sync_output_above_15_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    assert_eq!(
        load_with_ipc(crate::ipc::Ipc::with_corrupt_sync_out(0x10)),
        (Err(SavestateError::Corrupt), true)
    );
}

// ------------------------------------------------------------------------------------ DMA

/// ARM9 channel 0 copies one word to `0x0230_0000` as soon as it runs: `ENABLE | WORD | 1`.
const DMA_IMMEDIATE_WORD: u32 = (1 << 31) | (1 << 26) | 1;
const DMA0SAD: u32 = 0x0400_00B0;
const DMA0DAD: u32 = 0x0400_00B4;
const DMA0CNT: u32 = 0x0400_00B8;

fn dma_kind(arm9: bool, channel: u8) -> crate::scheduler::EventKind {
    crate::scheduler::EventKind::DmaStart { arm9, channel }
}

/// A booted machine one frame in, with an Immediate start queued on ARM9 channel 0.
fn with_pending_arm9_dma() -> Machine {
    let mut machine = booted(&counter_rom(), 1);
    machine.bus9().write32(DMA0SAD, 0x0220_0000);
    machine.bus9().write32(DMA0DAD, 0x0230_0000);
    machine.bus9().write32(DMA0CNT, DMA_IMMEDIATE_WORD);
    machine
}

/// The same on ARM7 channel 0, written at the ARM7's own clock.
fn with_pending_arm7_dma() -> Machine {
    let mut machine = booted(&counter_rom(), 1);
    let at = machine.arm7_clock();
    machine.bus7(at).write32(DMA0SAD, 0x0220_0000);
    machine.bus7(at).write32(DMA0DAD, 0x0230_0000);
    machine.bus7(at).write32(DMA0CNT, DMA_IMMEDIATE_WORD);
    machine
}

#[test]
fn given_a_state_with_a_pending_dma_channel_and_its_start_event_when_loaded_then_it_is_accepted() {
    let saver = with_pending_arm9_dma();
    assert_eq!(load_onto_fresh(&counter_rom(), &saver).0, Ok(()));
}

#[test]
fn given_a_state_with_a_pending_arm9_dma_channel_and_no_start_event_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let mut saver = with_pending_arm9_dma();
    saver.scheduler.cancel(dma_kind(true, 0));
    assert_eq!(
        load_onto_fresh(&counter_rom(), &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_pending_arm7_dma_channel_and_no_start_event_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let mut saver = with_pending_arm7_dma();
    saver.scheduler.cancel(dma_kind(false, 0));
    assert_eq!(
        load_onto_fresh(&counter_rom(), &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_dma_start_event_for_a_channel_with_nothing_waiting_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let mut saver = booted(&counter_rom(), 1);
    let now = saver.scheduler.now();
    saver.scheduler.schedule(now, dma_kind(true, 2));
    assert_eq!(
        load_onto_fresh(&counter_rom(), &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_an_arm7_dma_start_event_for_a_channel_with_nothing_waiting_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let mut saver = booted(&counter_rom(), 1);
    let now = saver.scheduler.now();
    saver.scheduler.schedule(now, dma_kind(false, 3));
    assert_eq!(
        load_onto_fresh(&counter_rom(), &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_two_start_events_for_one_pending_channel_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let mut saver = with_pending_arm9_dma();
    let now = saver.scheduler.now();
    saver.scheduler.schedule(now + 1, dma_kind(true, 0));
    assert_eq!(
        load_onto_fresh(&counter_rom(), &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

/// A pending ARM9 channel whose start event sits `ahead` master cycles past `now`.
fn with_dma_event_ahead(ahead: u64) -> Machine {
    let mut saver = with_pending_arm9_dma();
    let now = saver.scheduler.now();
    saver.scheduler.cancel(dma_kind(true, 0));
    saver.scheduler.schedule(now + ahead, dma_kind(true, 0));
    saver
}

#[test]
fn given_a_state_with_a_dma_start_event_far_beyond_a_slice_when_loaded_then_it_is_corrupt_and_the_target_is_unchanged()
 {
    let saver = with_dma_event_ahead(crate::scheduler::SLICE + 1);
    assert_eq!(
        load_onto_fresh(&counter_rom(), &saver),
        (Err(SavestateError::Corrupt), true)
    );
}

#[test]
fn given_a_state_with_a_dma_start_event_exactly_a_slice_ahead_when_loaded_then_it_is_accepted() {
    let saver = with_dma_event_ahead(crate::scheduler::SLICE);
    assert_eq!(load_onto_fresh(&counter_rom(), &saver).0, Ok(()));
}

/// The part of a channel's saved encoding the tests below rebuild, field for field as `dma.rs`
/// declares it: postcard is positional, so only the order and the types matter.
#[derive(serde::Serialize)]
struct ChannelBytes {
    source: u32,
    destination: u32,
    control: u32,
    source_now: u32,
    destination_now: u32,
    count_now: u32,
    pending: bool,
}

#[derive(serde::Serialize)]
struct DmaBytes {
    channels: [ChannelBytes; 4],
    fill: [u32; 4],
}

const fn idle_channel() -> ChannelBytes {
    ChannelBytes {
        source: 0,
        destination: 0,
        control: 0,
        source_now: 0,
        destination_now: 0,
        count_now: 0,
        pending: false,
    }
}

/// The saved bytes of `machine` with the one encoding of `real` inside them replaced by that of
/// `replacement`, or `None` when `real` does not occur exactly once.
fn bytes_with_replaced<T: serde::Serialize, U: serde::Serialize>(
    machine: &Machine,
    real: &T,
    replacement: &U,
) -> Option<Vec<u8>> {
    let needle = postcard::to_allocvec(real).expect("plain data encodes");
    let bytes = machine.save_state();
    let hits: Vec<usize> = bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle.as_slice())
        .map(|(at, _)| at)
        .collect();
    let at = *hits.first().filter(|_| hits.len() == 1)?;
    let new = postcard::to_allocvec(replacement).expect("plain data encodes");
    Some([&bytes[..at], &new, &bytes[at + needle.len()..]].concat())
}

/// A booted machine whose two DMA sets differ from each other and from power-on, so each one's
/// encoding is found once: ARM9 channel 0 holds source `0x0220_0000`, ARM7 channel 0 `0x0230_0000`.
fn with_distinct_dma() -> Machine {
    let mut machine = booted(&counter_rom(), 1);
    machine.bus9().write32(DMA0SAD, 0x0220_0000);
    machine
        .bus7(machine.arm7_clock())
        .write32(DMA0SAD, 0x0230_0000);
    machine
}

/// The ARM9 encoding with channel 0's registers as given and everything else idle.
fn arm9_bytes(channel0: ChannelBytes) -> DmaBytes {
    DmaBytes {
        channels: [channel0, idle_channel(), idle_channel(), idle_channel()],
        fill: [0; 4],
    }
}

/// Loads `bytes` (a state from `with_distinct_dma`) onto a fresh machine.
fn load_bytes(bytes: Option<Vec<u8>>) -> Result<(), SavestateError> {
    booted(&counter_rom(), 0).load_state(&bytes.expect("the DMA block is found once"))
}

#[test]
fn given_a_state_whose_dma_block_is_replaced_by_an_equal_encoding_when_loaded_then_it_is_accepted()
{
    // Proves the splice finds and rebuilds the right bytes, so the refusals below are about the
    // one bit each of them breaks.
    let machine = with_distinct_dma();
    let same = arm9_bytes(ChannelBytes {
        source: 0x0220_0000,
        ..idle_channel()
    });
    assert_eq!(
        load_bytes(bytes_with_replaced(&machine, machine.dma9(), &same)),
        Ok(())
    );
}

#[test]
fn given_a_state_with_an_arm9_dma_source_bit_that_does_not_exist_when_loaded_then_it_is_corrupt() {
    // The ARM9's addresses are 28 bits (`dma.md`, "Address masks"): bit 28 cannot be set.
    let machine = with_distinct_dma();
    let corrupt = arm9_bytes(ChannelBytes {
        source: 0x1220_0000,
        ..idle_channel()
    });
    assert_eq!(
        load_bytes(bytes_with_replaced(&machine, machine.dma9(), &corrupt)),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_a_state_with_an_arm9_dma_latched_count_above_the_field_when_loaded_then_it_is_corrupt() {
    // The ARM9's count is 21 bits, so the latched count is at most 200000h.
    let machine = with_distinct_dma();
    let corrupt = arm9_bytes(ChannelBytes {
        source: 0x0220_0000,
        count_now: 0x20_0001,
        ..idle_channel()
    });
    assert_eq!(
        load_bytes(bytes_with_replaced(&machine, machine.dma9(), &corrupt)),
        Err(SavestateError::Corrupt)
    );
}

/// The ARM7 encoding with channel 0's registers and the fill block as given.
fn arm7_bytes(channel0: ChannelBytes, fill: [u32; 4]) -> DmaBytes {
    DmaBytes {
        channels: [channel0, idle_channel(), idle_channel(), idle_channel()],
        fill,
    }
}

#[test]
fn given_a_state_with_an_arm7_dma_control_bit_that_does_not_exist_when_loaded_then_it_is_corrupt() {
    // The ARM7's start timing has two bits, from bit 28; bit 27 does not exist (`dma.md`,
    // "DMAnCNT control bits").
    let machine = with_distinct_dma();
    let corrupt = arm7_bytes(
        ChannelBytes {
            source: 0x0230_0000,
            control: 1 << 27,
            ..idle_channel()
        },
        [0; 4],
    );
    assert_eq!(
        load_bytes(bytes_with_replaced(&machine, machine.dma7(), &corrupt)),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_a_state_with_an_arm7_dma_fill_register_set_when_loaded_then_it_is_corrupt() {
    // The ARM7 has no fill registers (`dma.md`, "Registers").
    let machine = with_distinct_dma();
    let corrupt = arm7_bytes(
        ChannelBytes {
            source: 0x0230_0000,
            ..idle_channel()
        },
        [1, 0, 0, 0],
    );
    assert_eq!(
        load_bytes(bytes_with_replaced(&machine, machine.dma7(), &corrupt)),
        Err(SavestateError::Corrupt)
    );
}

#[test]
fn given_a_state_with_a_dma_channel_pending_but_disabled_when_loaded_then_it_is_corrupt() {
    // A waiting start needs enable, which is the busy flag (`dma.md`, "Enabling"). A start event
    // is queued, so only the channel itself is wrong.
    let mut machine = with_distinct_dma();
    let now = machine.scheduler.now();
    machine.scheduler.schedule(now, dma_kind(true, 0));
    let corrupt = arm9_bytes(ChannelBytes {
        source: 0x0220_0000,
        pending: true,
        ..idle_channel()
    });
    assert_eq!(
        load_bytes(bytes_with_replaced(&machine, machine.dma9(), &corrupt)),
        Err(SavestateError::Corrupt)
    );
}
