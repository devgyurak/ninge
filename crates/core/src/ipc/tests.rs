// SPDX-License-Identifier: Apache-2.0
//! `Ipc` on its own. Expected values come from `docs/hw/ipc.md` ("Registers", "Behavior",
//! "Edge cases"), never from running the code.
//!
//! Power-on has both FIFO enables off (`ipc.md`, "Unverified": reset `IPCFIFOCNT = 0101h`), so
//! the scenarios that need a working FIFO start from `enabled()`. Scenarios live in harness
//! functions that return what they observed; a test body only asserts.

use super::*;

const ENABLE: u16 = 1 << 15;
const SEND_IRQ_ENABLE: u16 = 1 << 2;
const CLEAR: u16 = 1 << 3;
const RECV_IRQ_ENABLE: u16 = 1 << 10;
const ERROR: u16 = 1 << 14;
const SEND_EMPTY: u16 = 1 << 0;
const SEND_FULL: u16 = 1 << 1;
const RECV_EMPTY: u16 = 1 << 8;
const RECV_FULL: u16 = 1 << 9;

const IF16: u32 = 1 << 16;
const IF17: u32 = 1 << 17;
const IF18: u32 = 1 << 18;
const NONE: Requests = Requests { irq9: 0, irq7: 0 };

/// Writes a halfword to IPCSYNC as CPU `ARM9`, byte by byte as the bus does; returns the requests.
fn write_sync<const ARM9: bool>(ipc: &mut Ipc, value: u16) -> Requests {
    write_half::<ARM9>(ipc, IPCSYNC_ADDR, value)
}

fn write_cnt<const ARM9: bool>(ipc: &mut Ipc, value: u16) -> Requests {
    write_half::<ARM9>(ipc, IPCFIFOCNT_ADDR, value)
}

fn write_half<const ARM9: bool>(ipc: &mut Ipc, addr: u32, value: u16) -> Requests {
    let [low, high] = value.to_le_bytes();
    let first = ipc.write_byte::<ARM9>(addr, low);
    let second = ipc.write_byte::<ARM9>(addr + 1, high);
    Requests {
        irq9: first.irq9 | second.irq9,
        irq7: first.irq7 | second.irq7,
    }
}

/// Both CPUs with their FIFOs enabled and nothing else set.
fn enabled() -> Ipc {
    let mut ipc = Ipc::new();
    write_cnt::<true>(&mut ipc, ENABLE);
    write_cnt::<false>(&mut ipc, ENABLE);
    ipc
}

/// Sends `words` from the ARM9, ignoring the requests.
fn send9(ipc: &mut Ipc, words: &[u32]) {
    words.iter().for_each(|&w| {
        ipc.send::<true>(w);
    });
}

/// Receives `count` words as the ARM7.
fn receive7(ipc: &mut Ipc, count: usize) -> Vec<u32> {
    (0..count).map(|_| ipc.receive::<false>().0).collect()
}

// ------------------------------------------------------------------------ IPCSYNC

#[test]
fn given_a_power_on_ipc_when_ipcsync_is_read_by_both_cpus_then_it_is_zero() {
    let ipc = Ipc::new();
    assert_eq!((ipc.sync::<true>(), ipc.sync::<false>()), (0, 0));
}

#[test]
fn given_the_arm9_wrote_an_output_nibble_when_the_arm7_reads_ipcsync_then_bits_0_to_3_hold_it() {
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0x0A00);
    assert_eq!(ipc.sync::<false>(), 0x000A);
}

#[test]
fn given_the_arm7_wrote_an_output_nibble_when_the_arm9_reads_ipcsync_then_bits_0_to_3_hold_it() {
    let mut ipc = Ipc::new();
    write_sync::<false>(&mut ipc, 0x0500);
    assert_eq!(ipc.sync::<true>(), 0x0005);
}

#[test]
fn given_the_arm9_wrote_an_output_nibble_when_it_reads_its_own_ipcsync_then_bits_8_to_11_hold_it() {
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0x0700);
    assert_eq!(ipc.sync::<true>(), 0x0700);
}

#[test]
fn given_a_write_of_all_ones_when_ipcsync_is_read_then_only_the_output_nibble_and_the_enable_stay()
{
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0xFFFF);
    assert_eq!((ipc.sync::<true>(), ipc.sync::<false>()), (0x4F00, 0x000F));
}

#[test]
fn given_both_cpus_wrote_outputs_when_each_reads_then_each_sees_the_other_nibble_and_its_own() {
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0x0300);
    write_sync::<false>(&mut ipc, 0x0C00);
    assert_eq!((ipc.sync::<true>(), ipc.sync::<false>()), (0x030C, 0x0C03));
}

#[test]
fn given_the_arm7_wrote_ipcsync_when_the_arm9_output_is_read_then_it_is_unchanged() {
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0x0900);
    write_sync::<false>(&mut ipc, 0x0000);
    assert_eq!(ipc.sync::<true>() & 0x0F00, 0x0900);
}

#[test]
fn given_only_the_high_byte_written_when_ipcsync_is_read_then_the_output_nibble_is_set() {
    let mut ipc = Ipc::new();
    ipc.write_byte::<true>(IPCSYNC_ADDR + 1, 0x0B);
    assert_eq!(ipc.sync::<false>(), 0x000B);
}

#[test]
fn given_only_the_low_byte_written_when_ipcsync_is_read_then_nothing_changes() {
    let mut ipc = Ipc::new();
    ipc.write_byte::<true>(IPCSYNC_ADDR, 0xFF);
    assert_eq!((ipc.sync::<true>(), ipc.sync::<false>()), (0, 0));
}

#[test]
fn given_the_arm9_sends_the_sync_irq_when_the_arm7_enabled_it_then_if16_is_requested_on_the_arm7() {
    let mut ipc = Ipc::new();
    write_sync::<false>(&mut ipc, 0x4000);
    let requests = write_sync::<true>(&mut ipc, 0x2000);
    assert_eq!(
        requests,
        Requests {
            irq9: 0,
            irq7: IF16
        }
    );
}

#[test]
fn given_the_arm7_sends_the_sync_irq_when_the_arm9_enabled_it_then_if16_is_requested_on_the_arm9() {
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0x4000);
    let requests = write_sync::<false>(&mut ipc, 0x2000);
    assert_eq!(
        requests,
        Requests {
            irq9: IF16,
            irq7: 0
        }
    );
}

#[test]
fn given_the_arm9_sends_the_sync_irq_when_the_arm7_did_not_enable_it_then_nothing_is_requested_unverified()
 {
    let mut ipc = Ipc::new();
    assert_eq!(write_sync::<true>(&mut ipc, 0x2000), NONE);
}

#[test]
fn given_a_write_with_bit_13_clear_when_the_other_cpu_enabled_the_irq_then_nothing_is_requested() {
    let mut ipc = Ipc::new();
    write_sync::<false>(&mut ipc, 0x4000);
    assert_eq!(write_sync::<true>(&mut ipc, 0x0F00), NONE);
}

#[test]
fn given_a_sync_irq_was_sent_when_ipcsync_is_read_then_bit_13_reads_zero() {
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0x2000);
    assert_eq!(ipc.sync::<true>() & 0x2000, 0);
}

#[test]
fn given_the_enable_bit_set_by_one_cpu_when_the_other_reads_ipcsync_then_it_does_not_see_it() {
    let mut ipc = Ipc::new();
    write_sync::<true>(&mut ipc, 0x4000);
    assert_eq!(ipc.sync::<false>() & 0x4000, 0);
}

// ------------------------------------------------------------------ IPCFIFOCNT flags

#[test]
fn given_a_power_on_ipc_when_ipcfifocnt_is_read_then_both_fifos_show_empty_and_the_rest_is_zero() {
    let ipc = Ipc::new();
    assert_eq!(
        (ipc.control::<true>(), ipc.control::<false>()),
        (SEND_EMPTY | RECV_EMPTY, SEND_EMPTY | RECV_EMPTY)
    );
}

#[test]
fn given_a_word_sent_when_the_flags_are_read_then_the_sender_send_and_the_receiver_receive_show_not_empty()
 {
    let mut ipc = enabled();
    send9(&mut ipc, &[1]);
    assert_eq!(
        (ipc.control::<true>(), ipc.control::<false>()),
        (ENABLE | RECV_EMPTY, ENABLE | SEND_EMPTY)
    );
}

#[test]
fn given_a_word_sent_when_the_sender_receive_flags_are_read_then_its_own_receive_fifo_is_still_empty()
 {
    let mut ipc = enabled();
    send9(&mut ipc, &[1]);
    assert_eq!(ipc.control::<true>() & RECV_EMPTY, RECV_EMPTY);
}

#[test]
fn given_16_words_sent_when_the_flags_are_read_then_send_is_full_and_the_other_receive_is_full() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 16]);
    assert_eq!(
        (ipc.control::<true>(), ipc.control::<false>()),
        (
            ENABLE | SEND_FULL | RECV_EMPTY,
            ENABLE | RECV_FULL | SEND_EMPTY
        )
    );
}

#[test]
fn given_a_full_fifo_when_one_word_is_received_then_the_full_flag_clears() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 16]);
    ipc.receive::<false>();
    assert_eq!(ipc.control::<true>() & SEND_FULL, 0);
}

#[test]
fn given_a_write_of_all_ones_to_ipcfifocnt_when_it_is_read_then_only_the_writable_bits_show() {
    let mut ipc = Ipc::new();
    write_cnt::<true>(&mut ipc, 0xFFFF);
    assert_eq!(
        ipc.control::<true>(),
        ENABLE | RECV_IRQ_ENABLE | SEND_IRQ_ENABLE | SEND_EMPTY | RECV_EMPTY
    );
}

#[test]
fn given_the_clear_bit_written_when_ipcfifocnt_is_read_then_bit_3_reads_zero() {
    let mut ipc = enabled();
    write_cnt::<true>(&mut ipc, ENABLE | CLEAR);
    assert_eq!(ipc.control::<true>() & CLEAR, 0);
}

#[test]
fn given_a_byte_write_to_the_low_byte_when_ipcfifocnt_is_read_then_the_high_byte_is_unchanged() {
    let mut ipc = enabled();
    ipc.write_byte::<true>(IPCFIFOCNT_ADDR, SEND_IRQ_ENABLE as u8);
    assert_eq!(
        ipc.control::<true>(),
        ENABLE | SEND_IRQ_ENABLE | RECV_EMPTY | SEND_EMPTY
    );
}

#[test]
fn given_a_byte_write_to_the_high_byte_when_ipcfifocnt_is_read_then_the_low_byte_is_unchanged() {
    let mut ipc = Ipc::new();
    ipc.write_byte::<true>(IPCFIFOCNT_ADDR, SEND_IRQ_ENABLE as u8);
    ipc.write_byte::<true>(IPCFIFOCNT_ADDR + 1, (ENABLE >> 8) as u8);
    assert_eq!(
        ipc.control::<true>(),
        ENABLE | SEND_IRQ_ENABLE | RECV_EMPTY | SEND_EMPTY
    );
}

#[test]
fn given_a_byte_address_past_the_two_registers_when_read_then_no_register_answers() {
    let ipc = Ipc::new();
    assert_eq!(
        (
            ipc.read_byte::<true>(IPCFIFOCNT_ADDR + 2),
            ipc.read_byte::<true>(IPCSYNC_ADDR + 2)
        ),
        (None, None)
    );
}

// ----------------------------------------------------------------- send and receive

#[test]
fn given_three_words_sent_when_received_then_they_come_out_in_the_order_sent() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0x11, 0x22, 0x33]);
    assert_eq!(receive7(&mut ipc, 3), vec![0x11, 0x22, 0x33]);
}

#[test]
fn given_a_word_received_when_the_fifo_is_read_again_then_the_error_bit_is_set() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0x11]);
    receive7(&mut ipc, 2);
    assert_eq!(ipc.control::<false>() & ERROR, ERROR);
}

/// Sends 16 words and receives 16, `rounds` times over, so the ring wraps; returns the words
/// that came out.
fn wrap_rounds(rounds: u32) -> Vec<u32> {
    let mut ipc = enabled();
    (0..rounds)
        .flat_map(|round| {
            let words: Vec<u32> = (0..16).map(|i| round * 100 + i).collect();
            send9(&mut ipc, &words);
            receive7(&mut ipc, 16)
        })
        .collect()
}

#[test]
fn given_the_fifo_filled_and_drained_three_times_when_received_then_every_word_comes_out_in_order()
{
    let expected: Vec<u32> = (0..3)
        .flat_map(|r| (0..16).map(move |i| r * 100 + i))
        .collect();
    assert_eq!(wrap_rounds(3), expected);
}

#[test]
fn given_a_partly_drained_fifo_when_it_is_refilled_then_the_order_holds_across_the_wrap() {
    let mut ipc = enabled();
    send9(&mut ipc, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    receive7(&mut ipc, 10);
    send9(&mut ipc, &[13, 14, 15, 16, 17, 18, 19, 20]);
    assert_eq!(
        receive7(&mut ipc, 10),
        vec![11, 12, 13, 14, 15, 16, 17, 18, 19, 20]
    );
}

#[test]
fn given_a_full_fifo_when_a_17th_word_is_sent_then_the_sender_error_bit_is_set() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 17]);
    assert_eq!(ipc.control::<true>() & ERROR, ERROR);
}

#[test]
fn given_a_full_fifo_when_a_17th_word_is_sent_then_the_receiver_error_bit_stays_clear() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 17]);
    assert_eq!(ipc.control::<false>() & ERROR, 0);
}

#[test]
fn given_a_full_fifo_when_a_17th_word_is_sent_then_the_first_16_are_kept_and_the_17th_is_dropped_unverified()
 {
    let mut ipc = enabled();
    let mut words: Vec<u32> = (1..=16).collect();
    words.push(99);
    send9(&mut ipc, &words);
    let received = receive7(&mut ipc, 17);
    let mut expected: Vec<u32> = (1..=16).collect();
    expected.push(16);
    assert_eq!(received, expected);
}

#[test]
fn given_16_words_sent_when_no_17th_is_sent_then_the_error_bit_stays_clear() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 16]);
    assert_eq!(ipc.control::<true>() & ERROR, 0);
}

#[test]
fn given_a_never_used_fifo_when_it_is_read_then_zero_comes_back_and_the_error_bit_is_set() {
    let mut ipc = enabled();
    let (word, _) = ipc.receive::<false>();
    assert_eq!((word, ipc.control::<false>() & ERROR), (0, ERROR));
}

#[test]
fn given_the_last_word_was_received_when_the_empty_fifo_is_read_then_that_word_comes_back_again() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0xAA, 0xBB]);
    assert_eq!(receive7(&mut ipc, 4), vec![0xAA, 0xBB, 0xBB, 0xBB]);
}

#[test]
fn given_an_empty_read_when_the_fifo_is_inspected_then_it_is_still_empty() {
    let mut ipc = enabled();
    receive7(&mut ipc, 2);
    assert_eq!(ipc.control::<false>() & RECV_EMPTY, RECV_EMPTY);
}

#[test]
fn given_an_error_set_when_one_is_written_to_bit_14_then_the_error_clears() {
    let mut ipc = enabled();
    ipc.receive::<false>();
    write_cnt::<false>(&mut ipc, ENABLE | ERROR);
    assert_eq!(ipc.control::<false>() & ERROR, 0);
}

#[test]
fn given_an_error_set_when_zero_is_written_to_bit_14_then_the_error_stays() {
    let mut ipc = enabled();
    ipc.receive::<false>();
    write_cnt::<false>(&mut ipc, ENABLE);
    assert_eq!(ipc.control::<false>() & ERROR, ERROR);
}

#[test]
fn given_the_arm7_error_set_when_the_arm9_acknowledges_then_the_arm7_error_stays() {
    let mut ipc = enabled();
    ipc.receive::<false>();
    write_cnt::<true>(&mut ipc, ENABLE | ERROR);
    assert_eq!(ipc.control::<false>() & ERROR, ERROR);
}

// ----------------------------------------------------------------------------- clear

#[test]
fn given_words_in_flight_when_the_sender_clears_then_the_receiver_fifo_is_empty() {
    let mut ipc = enabled();
    send9(&mut ipc, &[1, 2, 3]);
    write_cnt::<true>(&mut ipc, ENABLE | CLEAR);
    assert_eq!(
        (
            ipc.control::<true>() & SEND_EMPTY,
            ipc.control::<false>() & RECV_EMPTY
        ),
        (SEND_EMPTY, RECV_EMPTY)
    );
}

#[test]
fn given_words_in_flight_when_the_receiver_clears_then_the_senders_fifo_is_untouched() {
    let mut ipc = enabled();
    send9(&mut ipc, &[1, 2, 3]);
    write_cnt::<false>(&mut ipc, ENABLE | CLEAR);
    assert_eq!(receive7(&mut ipc, 3), vec![1, 2, 3]);
}

#[test]
fn given_a_cleared_fifo_that_once_delivered_a_word_when_read_then_zero_comes_back_with_the_error() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0x55]);
    receive7(&mut ipc, 1);
    write_cnt::<true>(&mut ipc, ENABLE | CLEAR);
    let (word, _) = ipc.receive::<false>();
    assert_eq!((word, ipc.control::<false>() & ERROR), (0, ERROR));
}

#[test]
fn given_a_cleared_full_fifo_when_it_is_sent_to_again_then_no_error_is_set() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 16]);
    write_cnt::<true>(&mut ipc, ENABLE | CLEAR);
    send9(&mut ipc, &[1]);
    assert_eq!(ipc.control::<true>() & ERROR, 0);
}

// ----------------------------------------------------------------------- enable gate

#[test]
fn given_the_sender_fifo_disabled_when_a_word_is_sent_then_nothing_is_stored_and_no_error_is_set() {
    let mut ipc = Ipc::new();
    write_cnt::<false>(&mut ipc, ENABLE);
    ipc.send::<true>(5);
    assert_eq!(
        (ipc.control::<true>(), ipc.control::<false>()),
        (SEND_EMPTY | RECV_EMPTY, ENABLE | SEND_EMPTY | RECV_EMPTY)
    );
}

#[test]
fn given_a_full_fifo_when_the_sender_is_disabled_and_sends_then_no_error_is_set() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 16]);
    write_cnt::<true>(&mut ipc, 0);
    ipc.send::<true>(1);
    assert_eq!(ipc.control::<true>() & ERROR, 0);
}

#[test]
fn given_the_receiver_disabled_when_it_reads_then_the_oldest_word_comes_back_and_stays() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0x31, 0x32]);
    write_cnt::<false>(&mut ipc, 0);
    let first = ipc.receive::<false>().0;
    let second = ipc.receive::<false>().0;
    assert_eq!(
        (first, second, ipc.control::<false>() & (ERROR | RECV_EMPTY)),
        (0x31, 0x31, 0)
    );
}

#[test]
fn given_the_receiver_disabled_when_it_is_enabled_again_then_the_words_are_still_there() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0x31, 0x32]);
    write_cnt::<false>(&mut ipc, 0);
    ipc.receive::<false>();
    write_cnt::<false>(&mut ipc, ENABLE);
    assert_eq!(receive7(&mut ipc, 2), vec![0x31, 0x32]);
}

#[test]
fn given_the_receiver_disabled_and_the_fifo_empty_when_it_reads_then_the_last_word_comes_back_without_an_error_unverified()
 {
    let mut ipc = Ipc::new();
    let (word, _) = ipc.receive::<false>();
    assert_eq!((word, ipc.control::<false>() & ERROR), (0, 0));
}

#[test]
fn given_a_receiver_peek_when_the_fifo_holds_words_then_the_oldest_is_shown_and_nothing_changes() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0x41, 0x42]);
    let before = ipc.clone();
    let seen = ipc.peek_recv::<false>();
    assert_eq!((seen, ipc == before), (0x41, true));
}

#[test]
fn given_an_empty_fifo_when_peeked_then_the_last_word_is_shown_and_no_error_is_set() {
    let mut ipc = enabled();
    send9(&mut ipc, &[0x41]);
    receive7(&mut ipc, 1);
    let seen = ipc.peek_recv::<false>();
    assert_eq!((seen, ipc.control::<false>() & ERROR), (0x41, 0));
}

// --------------------------------------------------------------------------- IRQs

/// Enables the IRQ bits `bits` of IPCFIFOCNT on the ARM9 (FIFO enabled too) and returns the
/// requests of that write.
fn arm9_cnt(ipc: &mut Ipc, bits: u16) -> Requests {
    write_cnt::<true>(ipc, ENABLE | bits)
}

#[test]
fn given_an_empty_send_fifo_when_the_send_empty_irq_is_enabled_then_if17_is_requested_on_the_same_cpu_unverified()
 {
    let mut ipc = Ipc::new();
    assert_eq!(
        arm9_cnt(&mut ipc, SEND_IRQ_ENABLE),
        Requests {
            irq9: IF17,
            irq7: 0
        }
    );
}

#[test]
fn given_the_send_empty_irq_enabled_while_empty_when_the_enable_is_written_again_then_nothing_is_requested()
 {
    let mut ipc = Ipc::new();
    arm9_cnt(&mut ipc, SEND_IRQ_ENABLE);
    assert_eq!(arm9_cnt(&mut ipc, SEND_IRQ_ENABLE), NONE);
}

#[test]
fn given_the_send_empty_irq_enabled_when_a_word_is_sent_then_nothing_is_requested() {
    let mut ipc = enabled();
    arm9_cnt(&mut ipc, SEND_IRQ_ENABLE);
    assert_eq!(ipc.send::<true>(1), NONE);
}

#[test]
fn given_a_non_empty_send_fifo_when_the_receiver_takes_the_last_word_then_if17_is_requested_on_the_sender()
 {
    let mut ipc = enabled();
    send9(&mut ipc, &[1, 2]);
    arm9_cnt(&mut ipc, SEND_IRQ_ENABLE);
    let first = ipc.receive::<false>().1;
    let second = ipc.receive::<false>().1;
    assert_eq!(
        (first, second),
        (
            NONE,
            Requests {
                irq9: IF17,
                irq7: 0
            }
        )
    );
}

#[test]
fn given_a_non_empty_send_fifo_when_the_irq_enable_is_off_and_the_fifo_drains_then_nothing_is_requested()
 {
    let mut ipc = enabled();
    send9(&mut ipc, &[1]);
    assert_eq!(ipc.receive::<false>().1, NONE);
}

#[test]
fn given_if17_raised_once_when_the_fifo_fills_and_drains_again_then_it_is_raised_again() {
    let mut ipc = enabled();
    arm9_cnt(&mut ipc, SEND_IRQ_ENABLE);
    send9(&mut ipc, &[1]);
    let again = ipc.receive::<false>().1;
    assert_eq!(
        again,
        Requests {
            irq9: IF17,
            irq7: 0
        }
    );
}

#[test]
fn given_a_non_empty_send_fifo_when_the_sender_clears_it_then_if17_is_requested_on_the_sender() {
    let mut ipc = enabled();
    send9(&mut ipc, &[1, 2, 3]);
    write_cnt::<true>(&mut ipc, ENABLE | SEND_IRQ_ENABLE);
    let requests = write_cnt::<true>(&mut ipc, ENABLE | SEND_IRQ_ENABLE | CLEAR);
    assert_eq!(
        requests,
        Requests {
            irq9: IF17,
            irq7: 0
        }
    );
}

#[test]
fn given_the_receive_irq_enabled_when_a_word_arrives_then_if18_is_requested_on_the_receiver() {
    let mut ipc = enabled();
    write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE);
    assert_eq!(
        ipc.send::<true>(1),
        Requests {
            irq9: 0,
            irq7: IF18
        }
    );
}

#[test]
fn given_the_receive_irq_enabled_when_a_second_word_arrives_then_nothing_is_requested() {
    let mut ipc = enabled();
    write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE);
    ipc.send::<true>(1);
    assert_eq!(ipc.send::<true>(2), NONE);
}

#[test]
fn given_the_receive_irq_enabled_when_the_fifo_drains_and_a_word_arrives_again_then_if18_is_raised_again()
 {
    let mut ipc = enabled();
    write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE);
    send9(&mut ipc, &[1]);
    receive7(&mut ipc, 1);
    assert_eq!(
        ipc.send::<true>(2),
        Requests {
            irq9: 0,
            irq7: IF18
        }
    );
}

#[test]
fn given_the_receive_irq_disabled_when_a_word_arrives_then_nothing_is_requested() {
    let mut ipc = enabled();
    assert_eq!(ipc.send::<true>(1), NONE);
}

#[test]
fn given_a_waiting_word_when_the_receive_irq_is_enabled_then_if18_is_requested_unverified() {
    let mut ipc = enabled();
    send9(&mut ipc, &[1]);
    assert_eq!(
        write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE),
        Requests {
            irq9: 0,
            irq7: IF18
        }
    );
}

#[test]
fn given_a_word_sent_to_a_full_fifo_when_the_receive_irq_is_enabled_then_nothing_is_requested() {
    let mut ipc = enabled();
    send9(&mut ipc, &[7; 16]);
    write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE);
    assert_eq!(ipc.send::<true>(1), NONE);
}

#[test]
fn given_a_disabled_sender_when_a_word_is_sent_then_no_if18_is_requested_on_the_receiver() {
    let mut ipc = Ipc::new();
    write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE);
    assert_eq!(ipc.send::<true>(1), NONE);
}

#[test]
fn given_the_receive_irq_enabled_when_the_sender_clears_a_non_empty_fifo_then_nothing_is_requested()
{
    let mut ipc = enabled();
    write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE);
    ipc.send::<true>(1);
    assert_eq!(write_cnt::<true>(&mut ipc, ENABLE | CLEAR), NONE);
}

#[test]
fn given_a_receive_irq_enable_written_when_ipcfifocnt_is_read_then_bit_10_reads_back() {
    let mut ipc = Ipc::new();
    write_cnt::<false>(&mut ipc, RECV_IRQ_ENABLE);
    assert_eq!(ipc.control::<false>() & RECV_IRQ_ENABLE, RECV_IRQ_ENABLE);
}

// -------------------------------------------------------------------- well-formed

#[test]
fn given_a_power_on_ipc_when_checked_then_it_is_well_formed() {
    assert!(Ipc::new().is_well_formed());
}

#[test]
fn given_a_busy_ipc_when_checked_then_it_is_well_formed() {
    let mut ipc = enabled();
    write_cnt::<false>(&mut ipc, ENABLE | RECV_IRQ_ENABLE);
    send9(&mut ipc, &[1, 2, 3]);
    receive7(&mut ipc, 1);
    assert!(ipc.is_well_formed());
}

#[test]
fn given_a_fifo_length_of_17_when_checked_then_it_is_not_well_formed() {
    assert!(!Ipc::with_corrupt_fifo_len(17).is_well_formed());
}

#[test]
fn given_a_fifo_head_of_16_when_checked_then_it_is_not_well_formed() {
    assert!(!Ipc::with_corrupt_fifo_head(16).is_well_formed());
}

#[test]
fn given_a_sync_output_above_15_when_checked_then_it_is_not_well_formed() {
    assert!(!Ipc::with_corrupt_sync_out(0x10).is_well_formed());
}
