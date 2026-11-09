// SPDX-License-Identifier: Apache-2.0
//! IPCSYNC and the IPC FIFOs, observed through our own `ipc` test ROM (`testroms/ipc/`). Each CPU
//! records what it reads in main RAM (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; the slot layouts
//! and the turn-taking are described at the top of `arm9.s` and `arm7.s`). Expected values come
//! from `docs/hw/ipc.md` ("Registers", "Behavior", "Edge cases", "Unverified"), never from running
//! the ROM.
//!
//! The CPUs take turns through IPCSYNC itself. Step k posts the nibble V(k) = (k mod 15) + 1 in the
//! poster's bits 8-11, so a CPU that reads IPCSYNC in its own step sees its last post in bits 8-11
//! and the other CPU's last post in bits 0-3 (`ipc.md`, "IPCSYNC bits"):
//!
//! | step | owner | V(step) |
//! |---|---|---|
//! | 1 | ARM9 | 2 |
//! | 2 | ARM7 | 3 |
//! | 3 | ARM9 | 4 (also sends the sync IRQ) |
//! | 4 | ARM7 | 5 (turns bit 14 on) |
//! | 5 | ARM9 | 6 (turns bit 14 on, sends the sync IRQ) |
//! | 6 | ARM7 | 7 (sends the sync IRQ) |
//! | 7 | ARM9 | 8 |
//!
//! FIFO words and IPCFIFOCNT values in the expectations are worked out from the note: bit 0 send
//! empty, bit 1 send full, bit 8 receive empty, bit 9 receive full, bit 14 error, bit 15 enable.
//!
//! Slots the note marks Unverified are in tests whose names say so (`TODO(hw-unverified)`).

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_RECORD: u32 = 0x0200_4000;
const ARM7_RECORD: u32 = 0x0200_5000;

/// Enable (bit 15), send empty (bit 0) and receive empty (bit 8): both FIFOs on and empty.
const ON_EMPTY: u32 = 0x8101;
const IF16: u32 = 1 << 16;
const IF17: u32 = 1 << 17;
const IF18: u32 = 1 << 18;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// Expected record: (slot, name, expected value, mask of the bits that are pinned).
type Row = (u32, &'static str, u32, u32);

const ALL: u32 = 0xFFFF_FFFF;

/// A machine that booted the `ipc` ROM and ran two frames (both CPUs finish well inside them).
fn boot() -> Machine {
    let rom = load_rom_file(&testrom_path("ipc")).expect("ipc ROM is built");
    let mut machine = Machine::new();
    machine.load_rom(rom).expect("ipc ROM has a valid header");
    machine.run_frame();
    machine.run_frame();
    machine
}

fn mismatches(machine: &Machine, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    rows.iter()
        .map(|&(slot, name, expected, mask)| {
            (
                slot,
                name,
                expected & mask,
                machine.arm9_peek32(base + slot * 4) & mask,
            )
        })
        .filter(|&(_, _, expected, got)| expected != got)
        .collect()
}

fn arm9_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM9_RECORD, rows)
}

fn arm7_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM7_RECORD, rows)
}

/// Rows with the full mask.
fn exact(rows: &[(u32, &'static str, u32)]) -> Vec<Row> {
    rows.iter().map(|&(s, n, v)| (s, n, v, ALL)).collect()
}

// ------------------------------------------------------------------ the ROM ran to its end

#[test]
fn given_ipc_rom_when_run_then_both_cpus_reach_their_last_step() {
    let rows = [
        (30, "ARM9 progress", 1, ALL),
        (31, "ARM9 sentinel", 0x4950_4331, ALL),
    ];
    let arm7 = [
        (62, "ARM7 progress", 1, ALL),
        (63, "ARM7 sentinel", 0x4950_4337, ALL),
    ];
    assert_eq!(
        (arm9_mismatches(&rows), arm7_mismatches(&arm7)),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

// ------------------------------------------------------------------------ reset values

// TODO(hw-unverified): no reset values are stated; the note models IPCSYNC = 0 and
// IPCFIFOCNT = 0101h (`ipc.md`, "Unverified"). IPCSYNC bits 0-3 are the other CPU's output, which
// depends on when its first post lands, so only bits 4-15 are pinned.
#[test]
fn given_ipc_rom_when_run_then_the_arm9_reads_the_modeled_reset_values_unverified() {
    let rows = [
        (0, "ARM9 IPCSYNC at reset, bits 4-15", 0, 0xFFF0),
        (1, "ARM9 IPCFIFOCNT at reset", 0x0101, ALL),
    ];
    assert_eq!(arm9_mismatches(&rows), Vec::<Mismatch>::new());
}

#[test]
fn given_ipc_rom_when_run_then_the_arm7_reads_the_modeled_reset_values_unverified() {
    let rows = [
        (0, "ARM7 IPCSYNC at reset, bits 4-15", 0, 0xFFF0),
        (1, "ARM7 IPCFIFOCNT at reset", 0x0101, ALL),
    ];
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------------------ IPCSYNC

// `ipc.md`, "IPCSYNC bits" and "Behavior": a CPU reads its own output in bits 8-11 and the
// other's in bits 0-3; bit 14 reads back; bit 13 reads 0.
#[test]
fn given_ipc_rom_when_the_arm9_reads_ipcsync_after_the_arm7_posted_then_both_nibbles_show() {
    let rows = exact(&[
        // Step 3: the ARM9's last post is V(1) = 2, the ARM7's is V(2) = 3.
        (4, "ARM9 IPCSYNC in step 3", 0x0203),
        // Step 7: the ARM9's last post is V(5) = 6 with bit 14 on and bit 13 sent (reads 0); the
        // ARM7's is V(6) = 7.
        (8, "ARM9 IPCSYNC in step 7", 0x4607),
    ]);
    assert_eq!(arm9_mismatches(&rows), Vec::<Mismatch>::new());
}

#[test]
fn given_ipc_rom_when_the_arm7_reads_ipcsync_after_the_arm9_posted_then_both_nibbles_show() {
    let rows = exact(&[
        // Step 2: the ARM7 has not posted; the ARM9's last post is V(1) = 2.
        (3, "ARM7 IPCSYNC in step 2", 0x0002),
        // Step 4: the ARM7's last post is V(2) = 3, the ARM9's is V(3) = 4 (bit 13 not visible).
        (6, "ARM7 IPCSYNC in step 4", 0x0304),
        // Step 6: the ARM7's last post is V(4) = 5 with bit 14 on; the ARM9's is V(5) = 6.
        (8, "ARM7 IPCSYNC in step 6", 0x4506),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// TODO(hw-unverified): an IRQ sent while the other CPU's bit 14 is 0 is lost
// (`ipc.md`, "Unverified").
#[test]
fn given_ipc_rom_when_the_arm9_sends_the_sync_irq_to_a_cpu_that_is_not_listening_then_no_if16_is_set_unverified()
 {
    let rows = exact(&[(5, "ARM7 IF in step 4", 0)]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Behavior": a 1 in bit 13 sets IF.16 on the other CPU when its bit 14 is on.
#[test]
fn given_ipc_rom_when_the_arm9_sends_the_sync_irq_to_a_listening_cpu_then_the_arm7_sees_if16_until_it_acknowledges()
 {
    let rows = exact(&[
        (7, "ARM7 IF after the IRQ", IF16),
        (9, "ARM7 IF after acknowledging", 0),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Behavior" and `interrupts.md`, "The IRQ line": the ARM9 (IME 1, IE bit 16, I clear)
// takes the IRQ through its vector once.
#[test]
fn given_ipc_rom_when_the_arm7_sends_the_sync_irq_then_the_arm9_handler_runs_once_and_saw_only_if16()
 {
    let rows = exact(&[
        (5, "ARM9 IF after the handler acknowledged", 0),
        (6, "ARM9 handler count", 1),
        (7, "OR of the IF values the handler saw", IF16),
    ]);
    assert_eq!(arm9_mismatches(&rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------------ the FIFO

// `ipc.md`, "Behavior" (disabled FIFO): a send is ignored and sets no error.
// TODO(hw-unverified): that the FIFO is disabled at power-on is the note's model.
#[test]
fn given_ipc_rom_when_the_arm9_sends_with_its_fifo_disabled_then_nothing_is_stored_and_no_error_is_set_unverified()
 {
    let arm9 = exact(&[
        (2, "ARM9 CNT after the ignored send", 0x0101),
        (3, "ARM9 CNT after enabling", ON_EMPTY),
    ]);
    let arm7 = exact(&[
        (2, "ARM7 CNT: nothing arrived", 0x0101),
        (4, "ARM7 CNT after enabling", ON_EMPTY),
    ]);
    assert_eq!(
        (arm9_mismatches(&arm9), arm7_mismatches(&arm7)),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

// `ipc.md`, "Status flags": the sender's send flags and the receiver's receive flags move together.
#[test]
fn given_ipc_rom_when_the_arm9_sends_three_words_then_the_flags_of_both_cpus_follow() {
    let arm9 = exact(&[
        // Send not empty (bit 0 clear), receive empty (bit 8), enabled.
        (9, "ARM9 CNT with 3 words sent", 0x8100),
        (10, "ARM9 CNT after the ARM7 took them", ON_EMPTY),
    ]);
    let arm7 = exact(&[
        // Receive not empty (bit 8 clear), send empty (bit 0), enabled.
        (10, "ARM7 CNT with 3 words waiting", 0x8001),
        (14, "ARM7 CNT after taking them", ON_EMPTY),
    ]);
    assert_eq!(
        (arm9_mismatches(&arm9), arm7_mismatches(&arm7)),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

#[test]
fn given_ipc_rom_when_the_arm7_takes_three_words_then_they_come_out_in_the_order_sent() {
    let rows = exact(&[
        (11, "first word", 0x1111_1111),
        (12, "second word", 0x2222_2222),
        (13, "third word", 0x3333_3333),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Behavior" (Receiving): an empty read returns the most recent word, leaves the FIFO and
// sets the reader's error bit; writing 1 to bit 14 acknowledges.
#[test]
fn given_ipc_rom_when_the_arm7_reads_an_empty_fifo_then_it_gets_the_last_word_and_the_error_bit() {
    let rows = exact(&[
        (15, "empty read returns the last word", 0x3333_3333),
        (16, "CNT after it: error bit set", 0xC101),
        (17, "CNT after acknowledging", ON_EMPTY),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Behavior" (Sending): the 17th word sets the sender's error bit; 16 words show full on
// the sender's send flag and the receiver's receive flag.
#[test]
fn given_ipc_rom_when_the_arm9_fills_the_fifo_and_sends_once_more_then_full_and_error_show() {
    let arm9 = exact(&[
        (11, "ARM9 CNT with 16 words (send full)", 0x8102),
        (12, "ARM9 CNT after a 17th send (error)", 0xC102),
        (13, "ARM9 CNT after acknowledging", 0x8102),
    ]);
    let arm7 = exact(&[
        (18, "ARM7 CNT with 16 words waiting (receive full)", 0x8201),
        (35, "ARM7 CNT after taking 16", ON_EMPTY),
    ]);
    assert_eq!(
        (arm9_mismatches(&arm9), arm7_mismatches(&arm7)),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

#[test]
fn given_ipc_rom_when_the_arm7_takes_16_words_then_they_come_out_in_the_order_sent() {
    let rows: Vec<Row> = (0..16u32)
        .map(|i| (19 + i, "word of 16", 0x100 + i, ALL))
        .collect();
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// TODO(hw-unverified): the word sent to a full FIFO is dropped (`ipc.md`, "Unverified"). Were it
// kept, a read after the 16th would return it instead of the last word.
#[test]
fn given_ipc_rom_when_a_17th_word_was_sent_to_a_full_fifo_then_the_arm7_never_sees_it_unverified() {
    let rows = exact(&[(36, "read after the 16th word", 0x10F)]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Behavior" (Clear): the sender's flush empties the receiver's FIFO and the last word
// reads as 0 afterwards.
#[test]
fn given_ipc_rom_when_the_arm9_flushes_its_send_fifo_then_the_arm7_receive_fifo_is_empty_and_reads_zero()
 {
    let arm9 = exact(&[(14, "ARM9 CNT after the flush (clear bit reads 0)", ON_EMPTY)]);
    let arm7 = exact(&[
        (37, "the word taken before the flush", 0x41),
        (38, "ARM7 CNT after the flush", ON_EMPTY),
        (39, "read after the flush returns 0", 0),
        (40, "ARM7 CNT after it: error bit set", 0xC101),
    ]);
    assert_eq!(
        (arm9_mismatches(&arm9), arm7_mismatches(&arm7)),
        (Vec::<Mismatch>::new(), Vec::<Mismatch>::new())
    );
}

// `ipc.md`, "Behavior" (Disabled FIFO): a read returns the oldest word and does not remove it.
#[test]
fn given_ipc_rom_when_the_arm7_reads_with_its_fifo_disabled_then_the_words_stay_in_the_fifo() {
    let rows = exact(&[
        (41, "first disabled read", 0x51),
        (42, "second disabled read", 0x51),
        // Receive not empty (bit 8 clear), send empty (bit 0), disabled, no error.
        (43, "CNT while disabled", 0x0001),
        (44, "first read after enabling", 0x51),
        (45, "second read after enabling", 0x52),
        (46, "CNT after taking both", ON_EMPTY),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// TODO(hw-unverified): a disabled, empty FIFO answers with the last word and no error
// (`ipc.md`, "Unverified").
#[test]
fn given_ipc_rom_when_the_arm7_reads_an_empty_disabled_fifo_then_it_gets_the_last_word_without_an_error_unverified()
 {
    let rows = exact(&[
        (47, "disabled empty read", 0x52),
        (48, "CNT after it", 0x0101),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------------------ IRQs

// `ipc.md`, "Interrupts": IF.18 is set when `IPCFIFOCNT.10 AND NOT IPCFIFOCNT.8` goes 0 to 1.
#[test]
fn given_ipc_rom_when_a_word_arrives_with_the_not_empty_irq_on_then_if18_is_set_once() {
    let rows = exact(&[
        (49, "IF with the IRQ on and the FIFO empty", 0),
        (50, "IF after the first word", IF18),
        (51, "IF after acknowledging", 0),
        // A second word while the first is unread: the condition stayed 1, so no new edge.
        (52, "IF after a second word", 0),
        (53, "first of the two", 0x61),
        (54, "second of the two", 0x62),
        // The FIFO drained and a word arrived: 0 to 1 again.
        (55, "IF after a word into the drained FIFO", IF18),
        (56, "the word", 0x63),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Interrupts": the IRQ is gated by IPCFIFOCNT bit 10. TODO(hw-unverified): enabling the
// IRQ while the condition already holds raises the flag (`ipc.md`, "Unverified").
#[test]
fn given_ipc_rom_when_the_not_empty_irq_is_turned_on_over_a_waiting_word_then_if18_is_set_unverified()
 {
    let rows = exact(&[
        (57, "IF after a word arrived with the IRQ off", 0),
        (58, "IF after turning the IRQ on", IF18),
        (59, "the word", 0x64),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Interrupts": IF.17 is set when `IPCFIFOCNT.2 AND IPCFIFOCNT.0` goes 0 to 1; a flush
// that empties the FIFO is such a change too.
#[test]
fn given_ipc_rom_when_the_send_fifo_empties_with_the_send_irq_on_then_the_arm9_sees_if17() {
    let rows = exact(&[
        (15, "IF with the IRQ on and a word in flight", 0),
        (16, "IF after the ARM7 took the last word", IF17),
        (17, "IF after acknowledging", 0),
        (18, "IF after a flush emptied the FIFO", IF17),
    ]);
    assert_eq!(arm9_mismatches(&rows), Vec::<Mismatch>::new());
}

// TODO(hw-unverified): enabling the send-empty IRQ on an empty FIFO raises the flag
// (`ipc.md`, "Unverified").
#[test]
fn given_ipc_rom_when_the_send_irq_is_turned_on_over_an_empty_fifo_then_if17_is_set_unverified() {
    let rows = exact(&[(19, "IF after enabling on an empty FIFO", IF17)]);
    assert_eq!(arm9_mismatches(&rows), Vec::<Mismatch>::new());
}

// `ipc.md`, "Interrupts" and `interrupts.md`, "The IRQ line": the ARM9 takes IF.17 when the ARM7
// empties its send FIFO and IF.18 when the ARM7 sends a word, each through the vector.
#[test]
fn given_ipc_rom_when_the_arm7_takes_and_then_sends_a_word_then_the_arm9_handler_runs_for_each() {
    let rows = exact(&[
        (20, "handler count after the ARM7 took a word", 2),
        (21, "OR of the IF values seen so far", IF16 | IF17),
        (22, "handler count after the ARM7 sent a word", 3),
        (23, "OR of the IF values seen", IF16 | IF17 | IF18),
        (24, "the word read through IPCFIFORECV", 0x81),
        (25, "IF at the end", 0),
    ]);
    assert_eq!(arm9_mismatches(&rows), Vec::<Mismatch>::new());
}

#[test]
fn given_ipc_rom_when_the_arm7_sends_in_the_last_step_then_its_own_flags_show_send_not_empty() {
    let rows = exact(&[
        (60, "word the ARM7 took in step 31", 0x75),
        // Send not empty (bit 0 clear), receive empty (bit 8), enabled.
        (61, "ARM7 CNT after sending", 0x8100),
    ]);
    assert_eq!(arm7_mismatches(&rows), Vec::<Mismatch>::new());
}

// -------------------------------------------------------------------- determinism and savestate

// AGENTS.md section 5.5: the same ROM gives the same savestate bytes.
#[test]
fn given_ipc_rom_when_run_twice_then_the_savestate_bytes_are_identical() {
    assert!(boot().save_state() == boot().save_state());
}

// A savestate taken mid-run continues like the uninterrupted run.
#[test]
fn given_ipc_rom_when_saved_after_one_frame_and_continued_then_it_equals_an_uninterrupted_run() {
    let rom = load_rom_file(&testrom_path("ipc")).expect("ipc ROM is built");
    let mut first = Machine::new();
    first.load_rom(rom.clone()).expect("valid header");
    first.run_frame();
    let bytes = first.save_state();
    let mut resumed = Machine::new();
    resumed.load_rom(rom).expect("valid header");
    let result = resumed.load_state(&bytes);
    resumed.run_frame();
    assert_eq!(
        (result, resumed.save_state() == boot().save_state()),
        (Ok(()), true)
    );
}
