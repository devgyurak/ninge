// SPDX-License-Identifier: Apache-2.0
//! The geometry command FIFO, observed through our own `gxfifo` test ROM (`testroms/gxfifo/`).
//! SWAP_BUFFERS holds the geometry engine until the next VBlank, so while it waits the ARM9 fills
//! the queue, looks at GXSTAT and the FIFO IRQ, writes to a full FIFO and starts a DMA channel in
//! start mode 7. The ROM records what it read in main RAM at `0x0200_8000`.
//! Expected words are worked out by hand from `docs/hw/geometry-engine.md`, never from running the
//! ROM:
//!
//! - Slot 0: GXSTAT at power-on, `0600_0000h` (FIFO less than half full and empty).
//! - Slot 1: SWAP_BUFFERS and its parameter leave the queue at once and the engine waits for
//!   VBlank: busy (bit 27) on top of the reset value, `0E00_0000h`.
//! - Slot 2: ten pushes queue behind it. The first four go straight into the PIPE, so the FIFO
//!   holds six: bits 16-23 = 6, less than half full, busy: `0A06_0000h`.
//! - Slots 3-5: FIFO IRQ mode 2 (empty) raises nothing while six entries wait (0); mode 1 (less
//!   than half full) raises IF bit 21 (`0020_0000h`); mode 0 and an acknowledge leave it clear (0).
//! - Slot 6: 250 more pushes fill the FIFO to 256: the count's low byte is 0, bit 24 (full) is
//!   set, bits 25 and 26 clear, busy: `0900_0000h`.
//! - Slot 7: the 261st push waits for the engine, which runs SWAP_BUFFERS at line 192 and then a
//!   push; the write returns inside line 192, so VCOUNT reads 192.
//! - Slot 8: 261 pushes in projection mode (the 1-bit pointer toggles, the second finds entry 1
//!   invalid): the pointer is 1 (bit 13) and the error flag (bit 15) is set: `0600_A000h`.
//! - Slot 9: bit 15 written with 1 clears the flag and the pointer: `0600_0000h`.
//! - Slot 10: a second SWAP_BUFFERS holds the engine again and a 300-word list goes by DMA in
//!   start mode 7. The first burst of 112 words leaves the FIFO below half (the first four words
//!   are in the PIPE, so 108 in the FIFO), the second burst follows (220 in the FIFO), and the
//!   third finds the FIFO at or above half and waits: count 220 = `DCh`, busy: `08DC_0000h`.
//! - Slot 11: DMA0CNT bit 31 still reads 1 meanwhile. Slot 12: IF bit 8 once it finishes.
//! - Slot 13: the 300 pushes ran: the pointer toggled an even number of times (0), the error flag is
//!   set again: `0600_8000h`.

use ninge_core::Machine;
use ninge_runner::golden::{assert_deterministic, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 6;
/// The record slots the ROM writes: 0-13, the progress and the sentinel.
const SLOTS: [u32; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 30, 31];

fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxfifo")).expect("gxfifo ROM is built"))
        .expect("gxfifo ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine
}

fn record(machine: &Machine) -> Vec<u32> {
    SLOTS
        .iter()
        .map(|&slot| machine.inspect().arm9_peek32(REC + 4 * slot))
        .collect()
}

#[test]
fn given_the_gxfifo_rom_when_6_frames_run_then_the_record_holds_the_note_values() {
    assert_eq!(
        record(&boot_for(FRAMES)),
        [
            0x0600_0000, // GXSTAT at power-on
            // TODO(hw-unverified): a command takes its entries from the queue only when all are
            // there (`geometry-engine.md`, "Unverified").
            0x0E00_0000, // SWAP_BUFFERS waits for VBlank: busy, FIFO empty
            // TODO(hw-unverified): the PIPE takes the first four entries and refills instantly
            // (`geometry-engine.md`, "Unverified").
            0x0A06_0000, // ten pushes: PIPE 4, FIFO 6
            0,           // mode 2 (empty): not empty, no IRQ
            0x0020_0000, // mode 1 (less than half full): IF bit 21
            0,           // mode 0 and an acknowledge: clear
            0x0900_0000, // FIFO full
            // TODO(hw-unverified): the stall ends when the engine takes an entry, after
            // SWAP_BUFFERS (`geometry-engine.md`, "Unverified").
            192,         // VCOUNT after the write that waited
            0x0600_A000, // 261 pushes in projection mode: pointer 1, error
            0x0600_0000, // the error acknowledged
            // TODO(hw-unverified): a burst is atomic and the next waits 2 cycles a word
            // (`geometry-engine.md`, "Unverified").
            0x08DC_0000, // two bursts of 112 words in: 220 in the FIFO
            1,           // the channel is still enabled
            0x100,       // IF bit 8, the transfer-complete IRQ
            0x0600_8000, // 300 pushes ran: pointer 0, error
            1,           // progress
            0x4946_5847  // "GXFI"
        ]
    );
}

#[test]
fn given_the_gxfifo_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxfifo", FRAMES);
}

#[test]
fn given_the_gxfifo_rom_when_run_twice_then_the_states_are_identical() {
    assert_eq!(boot_for(FRAMES).save_state(), boot_for(FRAMES).save_state());
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxfifo", 1, 5);
}
