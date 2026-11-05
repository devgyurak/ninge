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
use crate::bus::VramBankState;
use crate::cart::test_image::{Image, words};
use crate::{Machine, Screen};

/// Length and FNV-1a 64 of a power-on machine's savestate: pins the layout (see the test).
const PINNED_LEN: usize = 4_374_661;
const PINNED_HASH: u64 = 0x130c_51da_7a88_1a0a;
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
        &1u32.to_le_bytes(),
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
    let bytes = patched(&booted(&rom, 0).save_state(), 8, &7u32.to_le_bytes());
    let result = booted(&rom, 0).load_state(&bytes);
    assert_eq!(
        result,
        Err(SavestateError::VersionMismatch {
            found: 7,
            expected: 1
        })
    );
}

#[test]
fn given_a_version_mismatch_when_displayed_then_the_message_names_both_versions() {
    let rom = counter_rom();
    let bytes = patched(&booted(&rom, 0).save_state(), 8, &7u32.to_le_bytes());
    let message = booted(&rom, 0)
        .load_state(&bytes)
        .expect_err("version 7 is not version 1")
        .to_string();
    assert!(message.contains("version 7") && message.contains("version 1"));
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
