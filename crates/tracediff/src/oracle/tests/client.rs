// SPDX-License-Identifier: Apache-2.0
//! `GdbClient` against the fake stub: acknowledgement and retry, register blocks, stop replies,
//! error replies, breakpoints, and the guarded memory read. The wire rules are the public GDB
//! remote protocol's (`$data#sum`, `+` ack, `-` retransmit, `E<nn>`, an empty reply meaning
//! "unsupported"); the BIOS rule is `docs/contributing/trace-oracle.md` ("BIOS collapsing").

use std::ops::Range;

use super::fake::{Fake, Mode, SharedLog, State, read_addresses, requests};
use crate::oracle::{GdbClient, Stop};
use crate::rsp::RspError;

/// The text of an `Unexpected` error, `None` for anything else.
fn unexpected_text<T>(outcome: Result<T, RspError>) -> Option<String> {
    outcome.err().and_then(|error| match error {
        RspError::Unexpected(reply) => Some(reply),
        _ => None,
    })
}

/// The address of a `Forbidden` error, `None` for anything else.
fn forbidden_address<T>(outcome: Result<T, RspError>) -> Option<u32> {
    outcome.err().and_then(|error| match error {
        RspError::Forbidden(addr) => Some(addr),
        _ => None,
    })
}

fn client(fake: Fake) -> GdbClient<Fake> {
    GdbClient::new(fake, Vec::new())
}

/// A client whose stub has `script`, and the stub's log.
fn scripted(script: Vec<State>) -> (GdbClient<Fake>, SharedLog) {
    let (fake, log) = Fake::new(script);
    (client(fake), log)
}

/// A client over a stub that answers `command` requests with `reply`.
fn replying(command: u8, reply: &[u8]) -> GdbClient<Fake> {
    let (fake, _) = Fake::new(vec![State::at(0x0200_0000)]);
    client(fake.reply(command, reply))
}

// ---- transact: ack and retry ----

#[test]
fn given_a_good_reply_when_transacted_then_the_client_acknowledges_it_with_one_plus() {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = client(fake.reply(b'q', b"hello"));
    let reply = client.transact(b"qX").unwrap();
    assert_eq!(
        (reply, log.borrow().acks.clone()),
        (b"hello".to_vec(), vec![b'+'])
    );
}

#[test]
fn given_a_payload_with_special_bytes_when_transacted_then_the_stub_sees_it_escaped_on_the_wire() {
    // The fake logs raw wire data; `$` is sent as `}` 0x04.
    let (fake, log) = Fake::new(vec![State::at(0)]);
    let mut client = client(fake.reply(b'q', b"x"));
    client.transact(b"q$").unwrap();
    assert_eq!(log.borrow().packets, vec!["q}\u{4}".to_string()]);
}

#[test]
fn given_a_nak_of_the_request_when_transacted_then_it_is_sent_again_and_answered() {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = client(fake.nak_requests(1).reply(b'q', b"ok"));
    let reply = client.transact(b"q").unwrap();
    assert_eq!((reply, log.borrow().packets.len()), (b"ok".to_vec(), 2));
}

#[test]
fn given_two_naks_when_transacted_then_the_third_send_is_answered() {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = client(fake.nak_requests(2).reply(b'q', b"ok"));
    let reply = client.transact(b"q").unwrap();
    assert_eq!((reply, log.borrow().packets.len()), (b"ok".to_vec(), 3));
}

#[test]
fn given_three_naks_when_transacted_then_it_gives_up_after_exactly_three_sends() {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = client(fake.nak_requests(3).reply(b'q', b"ok"));
    let outcome = client.transact(b"q");
    assert_eq!(
        (outcome.is_err(), log.borrow().packets.clone()),
        (true, vec!["q".to_string(); 3])
    );
}

#[test]
fn given_one_corrupted_reply_when_transacted_then_the_client_naks_and_gets_the_good_reply() {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = client(fake.corrupt_replies(1).reply(b'q', b"ok"));
    let reply = client.transact(b"q").unwrap();
    assert_eq!(
        (reply, log.borrow().acks.clone()),
        (b"ok".to_vec(), vec![b'-', b'+'])
    );
}

#[test]
fn given_replies_that_stay_corrupted_when_transacted_then_it_gives_up_with_a_checksum_error() {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = client(fake.corrupt_replies(usize::MAX).reply(b'q', b"ok"));
    let outcome = client.transact(b"q");
    assert_eq!(
        (
            matches!(outcome, Err(RspError::Checksum { .. })),
            log.borrow().acks.iter().filter(|ack| **ack == b'-').count(),
        ),
        (true, 3)
    );
}

#[test]
fn given_a_reply_that_is_malformed_after_a_good_checksum_when_transacted_then_it_is_an_error() {
    let mut client = replying(b'q', b"}");
    assert!(matches!(client.transact(b"q"), Err(RspError::Malformed(_))));
}

#[test]
fn given_a_stub_that_never_answers_when_transacted_then_the_stream_is_closed() {
    let (fake, _) = Fake::new(vec![State::at(0)]);
    let mut client = client(fake.mode(Mode::Silent));
    assert!(matches!(client.transact(b"g"), Err(RspError::Closed)));
}

#[test]
fn given_a_stream_that_cannot_be_written_when_transacted_then_it_is_an_io_error() {
    let (fake, _) = Fake::new(vec![State::at(0)]);
    let mut client = client(fake.fail_writes());
    assert!(matches!(client.transact(b"g"), Err(RspError::Io(_))));
}

// ---- registers ----

#[test]
fn given_a_register_block_when_read_then_each_word_is_little_endian() {
    // Bytes 78 56 34 12 are the word 0x12345678; ef be ad de is 0xdeadbeef.
    let mut client = replying(b'g', b"78563412efbeadde");
    assert_eq!(client.registers().unwrap(), [0x1234_5678, 0xDEAD_BEEF]);
}

#[test]
fn given_a_scripted_cpu_when_registers_are_read_then_r0_to_r15_and_the_cpsr_come_back() {
    let state = State::at(0x0200_0000).reg(0, 7).reg(14, 0x0200_0100);
    let (mut client, _) = scripted(vec![state]);
    let words = client.registers().unwrap();
    assert_eq!(
        (words.len(), words[0], words[14], words[15], words[16]),
        (17, 7, 0x0200_0100, 0x0200_0000, 0x1F)
    );
}

#[test]
fn given_an_odd_length_register_reply_when_read_then_it_is_malformed() {
    let mut client = replying(b'g', b"7856341");
    assert!(matches!(
        client.registers(),
        Err(RspError::Malformed("odd hex length"))
    ));
}

#[test]
fn given_a_register_reply_that_is_not_whole_words_when_read_then_it_is_malformed() {
    let mut client = replying(b'g', b"785634");
    assert!(matches!(
        client.registers(),
        Err(RspError::Malformed("register block is not whole words"))
    ));
}

#[test]
fn given_a_register_reply_with_a_bad_digit_when_read_then_it_is_malformed() {
    let mut client = replying(b'g', b"7856341z");
    assert!(matches!(client.registers(), Err(RspError::Malformed(_))));
}

#[test]
fn given_an_e01_register_reply_when_read_then_it_is_a_stub_error_1() {
    let mut client = replying(b'g', b"E01");
    assert!(matches!(client.registers(), Err(RspError::Stub(1))));
}

#[test]
fn given_an_empty_register_reply_when_read_then_it_is_unsupported() {
    let mut client = replying(b'g', b"");
    assert!(matches!(client.registers(), Err(RspError::Unsupported)));
}

// ---- stop replies ----

/// The result of a step against a stub that answers `s` with `reply`.
fn step_with(reply: &[u8]) -> Result<Stop, RspError> {
    replying(b's', reply).step()
}

#[test]
fn given_an_s05_reply_when_stepped_then_it_is_signal_5() {
    assert!(matches!(step_with(b"S05"), Ok(Stop::Signal(5))));
}

#[test]
fn given_a_t_reply_with_fields_when_stepped_then_it_is_signal_5() {
    assert!(matches!(
        step_with(b"T05thread:p01.01;core:0;"),
        Ok(Stop::Signal(5))
    ));
}

#[test]
fn given_a_w_reply_when_stepped_then_the_target_has_exited() {
    assert!(matches!(step_with(b"W00"), Ok(Stop::Exited)));
}

#[test]
fn given_an_x_reply_when_stepped_then_the_target_has_exited() {
    assert!(matches!(step_with(b"X0f"), Ok(Stop::Exited)));
}

#[test]
fn given_a_signal_other_than_5_when_stepped_then_it_is_that_signal() {
    assert!(matches!(step_with(b"S0b"), Ok(Stop::Signal(0x0B))));
}

#[test]
fn given_a_garbage_stop_reply_when_stepped_then_it_is_unexpected() {
    assert_eq!(unexpected_text(step_with(b"zz?")), Some("zz?".to_string()));
}

#[test]
fn given_a_stop_reply_with_a_bad_signal_when_stepped_then_it_is_unexpected() {
    assert!(matches!(step_with(b"Sxy"), Err(RspError::Unexpected(_))));
}

#[test]
fn given_a_truncated_stop_reply_when_stepped_then_it_is_unexpected() {
    assert!(matches!(step_with(b"S0"), Err(RspError::Unexpected(_))));
}

#[test]
fn given_an_e01_stop_reply_when_stepped_then_it_is_a_stub_error_1() {
    assert!(matches!(step_with(b"E01"), Err(RspError::Stub(1))));
}

#[test]
fn given_an_empty_stop_reply_when_stepped_then_it_is_unsupported() {
    assert!(matches!(step_with(b""), Err(RspError::Unsupported)));
}

#[test]
fn given_a_scripted_cpu_when_stepped_then_the_next_state_shows_and_the_stub_saw_an_s() {
    let (mut client, log) = scripted(vec![State::at(0x0200_0000), State::at(0x0200_0004)]);
    let stop = client.step().unwrap();
    let pc = client.registers().unwrap()[15];
    assert_eq!(
        (stop, pc, requests(&log, 's')),
        (Stop::Signal(5), 0x0200_0004, vec!["s".to_string()])
    );
}

#[test]
fn given_a_scripted_cpu_when_resumed_then_it_stops_at_the_end_of_the_script_with_a_c_packet() {
    let (mut client, log) = scripted(vec![
        State::at(0x0200_0000),
        State::at(0x0200_0004),
        State::at(0x0200_0008),
    ]);
    let stop = client.resume().unwrap();
    let pc = client.registers().unwrap()[15];
    assert_eq!(
        (stop, pc, requests(&log, 'c')),
        (Stop::Signal(5), 0x0200_0008, vec!["c".to_string()])
    );
}

#[test]
fn given_an_error_reply_to_continue_when_resumed_then_it_is_a_stub_error() {
    let mut client = replying(b'c', b"E0a");
    assert!(matches!(client.resume(), Err(RspError::Stub(0x0A))));
}

// ---- breakpoints ----

#[test]
fn given_an_arm_breakpoint_when_set_then_the_stub_gets_z0_with_the_address_and_kind_4() {
    let (mut client, log) = scripted(vec![State::at(0)]);
    client.set_breakpoint(0x0200_0000, 4).unwrap();
    assert_eq!(requests(&log, 'Z'), vec!["Z0,2000000,4".to_string()]);
}

#[test]
fn given_a_thumb_breakpoint_when_cleared_then_the_stub_gets_lowercase_z0_with_kind_2() {
    let (mut client, log) = scripted(vec![State::at(0)]);
    client.clear_breakpoint(0x0238_000A, 2).unwrap();
    assert_eq!(requests(&log, 'z'), vec!["z0,238000a,2".to_string()]);
}

#[test]
fn given_a_stub_with_no_breakpoints_when_one_is_set_then_it_is_unsupported() {
    let (fake, _) = Fake::new(vec![State::at(0)]);
    let mut client = client(fake.mode(Mode::Empty));
    assert!(matches!(
        client.set_breakpoint(0x0200_0000, 4),
        Err(RspError::Unsupported)
    ));
}

#[test]
fn given_a_stub_that_answers_e01_when_a_breakpoint_is_set_then_it_is_a_stub_error_1() {
    let (fake, _) = Fake::new(vec![State::at(0)]);
    let mut client = client(fake.mode(Mode::Error));
    assert!(matches!(
        client.set_breakpoint(0x0200_0000, 4),
        Err(RspError::Stub(1))
    ));
}

#[test]
fn given_a_stub_with_no_breakpoints_when_one_is_cleared_then_it_is_unsupported() {
    let (fake, _) = Fake::new(vec![State::at(0)]);
    let mut client = client(fake.mode(Mode::Empty));
    assert!(matches!(
        client.clear_breakpoint(0x0200_0000, 4),
        Err(RspError::Unsupported)
    ));
}

#[test]
fn given_a_breakpoint_reply_other_than_ok_when_set_then_it_is_unexpected() {
    let mut client = replying(b'Z', b"OK?");
    assert!(matches!(
        client.set_breakpoint(0x0200_0000, 4),
        Err(RspError::Unexpected(_))
    ));
}

#[test]
fn given_a_malformed_error_code_when_a_breakpoint_is_set_then_it_is_unexpected() {
    let mut client = replying(b'Z', b"Ezz");
    assert!(matches!(
        client.set_breakpoint(0x0200_0000, 4),
        Err(RspError::Unexpected(_))
    ));
}

#[test]
fn given_a_short_error_reply_when_a_breakpoint_is_set_then_it_is_unexpected() {
    let mut client = replying(b'Z', b"E1");
    assert!(matches!(
        client.set_breakpoint(0x0200_0000, 4),
        Err(RspError::Unexpected(_))
    ));
}

// ---- read_memory ----

#[test]
fn given_mapped_memory_when_read_then_the_bytes_come_back_and_the_stub_saw_one_m_packet() {
    let mut state = State::at(0x0200_0000);
    state.put(0x0200_0000, &[0x55, 0x00, 0xA0, 0xE3]);
    let (mut client, log) = scripted(vec![state]);
    let bytes = client.read_memory(0x0200_0000, 4).unwrap();
    assert_eq!(
        (bytes, requests(&log, 'm')),
        (vec![0x55, 0x00, 0xA0, 0xE3], vec!["m2000000,4".to_string()])
    );
}

#[test]
fn given_a_reply_of_the_wrong_length_when_memory_is_read_then_it_is_unexpected() {
    let mut client = replying(b'm', b"aabb");
    assert_eq!(
        unexpected_text(client.read_memory(0x0200_0000, 4)),
        Some("aabb".to_string())
    );
}

#[test]
fn given_a_reply_with_a_bad_digit_when_memory_is_read_then_it_is_malformed() {
    let mut client = replying(b'm', b"aabbccdz");
    assert!(matches!(
        client.read_memory(0x0200_0000, 4),
        Err(RspError::Malformed(_))
    ));
}

#[test]
fn given_an_unmapped_address_when_memory_is_read_then_the_stub_error_comes_back() {
    let (mut client, _) = scripted(vec![State::at(0x0200_0000)]);
    assert!(matches!(
        client.read_memory(0x0300_0000, 4),
        Err(RspError::Stub(0x14))
    ));
}

#[test]
fn given_an_empty_reply_when_memory_is_read_then_it_is_unsupported() {
    let mut client = replying(b'm', b"");
    assert!(matches!(
        client.read_memory(0x0200_0000, 4),
        Err(RspError::Unsupported)
    ));
}

// ---- the forbidden ranges ----

/// What a read of `(addr, len)` did against a stub that has `0x100..0x200` and `0x1000..0x1004`
/// forbidden: whether it was refused as forbidden, and whether an `m` packet reached the stub.
fn forbidden_outcome(addr: u32, len: u32) -> (bool, bool) {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let forbidden: Vec<Range<u32>> = vec![0x100..0x200, 0x1000..0x1004];
    let mut client = GdbClient::new(fake, forbidden);
    let outcome = client.read_memory(addr, len);
    (
        forbidden_address(outcome) == Some(addr),
        !requests(&log, 'm').is_empty(),
    )
}

/// `(addr, len, refused, an m packet was sent)`; the unmapped bytes answer `E14`, so an allowed
/// read still ends in an error, but not a `Forbidden` one.
const FORBIDDEN_CASES: [(u32, u32, bool, bool); 14] = [
    // Inside the first range.
    (0x150, 4, true, false),
    // Starts before the range and runs into it.
    (0xFC, 8, true, false),
    // Starts before the range and covers all of it.
    (0xF0, 0x200, true, false),
    // Starts in the range and runs past its end.
    (0x1FC, 8, true, false),
    // The last byte of the range is the first byte read.
    (0x1FF, 1, true, false),
    // The range's first byte is the last byte read.
    (0xFF, 2, true, false),
    // Ends exactly where the range starts: no overlap.
    (0xFC, 4, false, true),
    // Starts exactly where the range ends: no overlap.
    (0x200, 4, false, true),
    // Well clear of both.
    (0x0200_0000, 4, false, true),
    // The second range, and a read that straddles its end.
    (0x1002, 1, true, false),
    (0x1003, 4, true, false),
    // Between the two ranges.
    (0x800, 4, false, true),
    // The second range's neighbours.
    (0xFFC, 4, false, true),
    (0x1004, 4, false, true),
];

/// `(addr, len, wanted, got)` of a case that differed.
type Failure = (u32, u32, (bool, bool), (bool, bool));

fn forbidden_failures() -> Vec<Failure> {
    FORBIDDEN_CASES
        .iter()
        .map(|&(addr, len, refused, sent)| {
            (addr, len, (refused, sent), forbidden_outcome(addr, len))
        })
        .filter(|(_, _, want, got)| want != got)
        .collect()
}

#[test]
fn given_forbidden_ranges_when_reads_touch_them_then_only_the_overlapping_ones_are_refused_unsent()
{
    assert_eq!(
        forbidden_failures(),
        Vec::<(u32, u32, (bool, bool), (bool, bool))>::new()
    );
}

#[test]
fn given_a_forbidden_read_when_refused_then_no_m_packet_of_any_address_reached_the_stub() {
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = GdbClient::new(fake, std::iter::once(0xFFFF_0000..0xFFFF_8000).collect());
    let refused = client.read_memory(0xFFFF_0018, 4);
    assert_eq!(
        (
            matches!(refused, Err(RspError::Forbidden(0xFFFF_0018))),
            read_addresses(&log)
        ),
        (true, Vec::<u32>::new())
    );
}

#[test]
fn given_a_read_whose_end_passes_the_top_of_the_address_space_when_checked_then_it_is_refused_unsent()
 {
    // 0xFFFF_FFFC + 8 passes 2^32: a stub that wraps would serve addresses from 0, which the
    // range check cannot see, so the read is refused outright.
    assert_eq!(forbidden_outcome(0xFFFF_FFFC, 8), (true, false));
}

#[test]
fn given_a_read_that_wraps_past_the_top_of_memory_when_made_then_it_is_forbidden_and_no_m_is_sent()
{
    // Read at 0xFFFF_FFFE for 4 bytes: a stub that wraps would serve address 0, the ARM7 BIOS.
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = GdbClient::new(fake, vec![ninge_core::trace::TraceCpu::Arm7.bios_area()]);
    let outcome = client.read_memory(0xFFFF_FFFE, 4);
    assert_eq!(
        (forbidden_address(outcome), requests(&log, 'm')),
        (Some(0xFFFF_FFFE), Vec::<String>::new())
    );
}

#[test]
fn given_a_one_byte_read_of_the_last_address_when_made_then_it_is_forbidden_and_no_m_is_sent() {
    // 0xFFFF_FFFF is outside the ARM9 block's range (its end is a u32), so the top-of-memory
    // rule is what refuses it.
    let (fake, log) = Fake::new(vec![State::at(0x0200_0000)]);
    let mut client = GdbClient::for_cpu(fake, ninge_core::trace::TraceCpu::Arm9);
    let outcome = client.read_memory(0xFFFF_FFFF, 1);
    assert_eq!(
        (forbidden_address(outcome), requests(&log, 'm')),
        (Some(0xFFFF_FFFF), Vec::<String>::new())
    );
}
