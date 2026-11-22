// SPDX-License-Identifier: Apache-2.0
//! The 3D engine stub, observed through our own `gx` test ROM (`testroms/gx/`). The ARM9 sends
//! geometry commands (packed words, an `STM`, the command ports, a 32-parameter command, a display
//! list by DMA in start mode 7), reads GXSTAT and IF after each step and records what it read in
//! main RAM; it judges the record and shows six bands on the bottom screen, green for a pass.
//! Expected words are worked out by hand from `docs/hw/gx-stub.md`, never from running the ROM.
//!
//! GXSTAT is `0600_0000h` (FIFO empty and less than half full) with the stack levels in bits 8-13
//! and the error flag in bit 15. The coordinate pointer is 6 bits and GXSTAT shows its low 5:
//!
//! - Slot 1: one `STM` sets mode 1 and a packed word pushes three times, level 3: `0600_0300h`.
//! - Slot 2: a push by its port gives level 4; slot 3: a pop with offset 1 returns to 3.
//! - Slot 4: SHININESS takes 32 parameters, so none of the 32 words `11h` is a push; the last
//!   word is one, level 4.
//! - Slot 5: 32 more pushes take the pointer from 4 to 36. Entry 31 is invalid, so the error flag
//!   sets, and 36 shows as level 4 (`36 mod 32`): `0600_8400h`. Slot 6: writing 1 to bit 15 clears
//!   the flag.
//! - Slots 7-9: in projection mode a push gives level 1 (bit 13: `0600_2400h` with the coordinate
//!   level still 4), the second push finds entry 1 (error, pointer back to 0: `0600_8400h`), and
//!   the acknowledge clears it.
//! - Slots 10-12: the DMA moves `MTX_MODE` 1 and 298 pushes, 300 words. The pointer goes from 36 to
//!   334, which is 14 in 6 bits: level 14 and the error flag, `0600_8E00h`. The channel clears its
//!   enable bit and raises IF bit 8.
//! - Slots 13-15: IF bit 21 is set when the FIFO IRQ mode is 1, stays set after an acknowledge,
//!   and clears once the mode is 0 and the bit is acknowledged.
//!
//! Several of these are the note's modelling choices, listed under its "Unverified": a green band
//! says the model follows the note, not that hardware was measured. The ROM also relies on the
//! POWCNT1 reset value `820Fh` having bit 3 (the geometry engine) set, since it writes POWCNT1 only
//! after the commands (`display-mode-vram.md`, "Unverified").

use ninge_core::{Machine, SCREEN_WIDTH, Screen};
use ninge_runner::golden::{assert_deterministic, assert_frame_hash, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 4;
const GREEN: u32 = 0x0000_FF00;
/// The record slots the ROM writes: 0-15, the progress and the sentinel.
const SLOTS: [u32; 18] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 30, 31];

fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gx")).expect("gx ROM is built"))
        .expect("gx ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine
}

fn boot() -> Machine {
    boot_for(FRAMES)
}

fn record(machine: &Machine) -> Vec<u32> {
    SLOTS
        .iter()
        .map(|&slot| machine.inspect().arm9_peek32(REC + 4 * slot))
        .collect()
}

/// The colour of each band's middle pixel on the bottom screen, top band first.
fn bands(machine: &Machine) -> Vec<u32> {
    let frame = machine.frame(Screen::Bottom);
    [16, 48, 80, 112, 144, 176]
        .iter()
        .map(|&y| frame[y * SCREEN_WIDTH + 128])
        .collect()
}

#[test]
fn given_the_gx_rom_when_4_frames_run_then_the_record_holds_the_note_values() {
    assert_eq!(
        record(&boot()),
        [
            0x0600_0000, // GXSTAT at power-on
            0x0600_0300, // STM and a packed word: level 3
            0x0600_0400, // a push by its port: level 4
            0x0600_0300, // a pop by its port: level 3
            // TODO(hw-unverified): SHININESS takes 32 parameters (`gx-stub.md`, "Unverified").
            0x0600_0400, // SHININESS with 32 parameters, then a push
            // TODO(hw-unverified): entries 1Fh and above raise the error and the pointer moves
            // on (`gx-stub.md`, "Unverified").
            0x0600_8400, // 32 pushes pass entry 31: the error flag, level 36 mod 32
            0x0600_0400, // the error acknowledged
            0x0600_2400, // a projection push: bit 13
            // TODO(hw-unverified): the 1-bit projection pointer wraps (`gx-stub.md`, "Unverified").
            0x0600_8400, // a second projection push: the error, the pointer back to 0
            0x0600_0400, // the error acknowledged
            // TODO(hw-unverified): the same pointer and error rules as slot 5.
            0x0600_8E00, // the display list by DMA: pointer 334 mod 64 = 14, the error
            0,           // the channel cleared its enable bit
            0x100,       // IF bit 8, the transfer-complete IRQ
            0x0020_0000, // IF bit 21 with the FIFO IRQ on
            0x0020_0000, // still set after an acknowledge
            // TODO(hw-unverified): the bit already set stays until acknowledged once the mode is 0
            // (`gx-stub.md`, "Unverified").
            0,           // cleared once the mode is 0
            1,           // progress
            0x5453_5847  // "GXST"
        ]
    );
}

#[test]
fn given_the_gx_rom_when_4_frames_run_then_every_band_is_green() {
    assert_eq!(bands(&boot()), vec![GREEN; 6]);
}

#[test]
fn given_the_gx_rom_when_4_frames_run_then_the_screens_match_the_golden() {
    assert_frame_hash("gx", FRAMES);
}

#[test]
fn given_the_gx_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gx", FRAMES);
}

#[test]
fn given_the_gx_rom_when_run_twice_then_the_states_are_identical() {
    assert_eq!(boot().save_state(), boot().save_state());
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gx", 1, 3);
}
