// SPDX-License-Identifier: Apache-2.0
//! The packet layer of the GDB remote serial protocol. Expected values come from the protocol's
//! public description (`$<data>#<checksum>`, the checksum is the byte sum modulo 256, `}` escapes
//! `$ # } *` as the byte XOR 0x20, `<c>*<n>` repeats `c` another `n - 29` times) and are written
//! out by hand, never taken from the code under test.

use super::*;
use std::io::Cursor;

/// A stream that hands out one byte per `read`, so the reader's buffering is exercised.
struct OneByte(Cursor<Vec<u8>>);

impl Read for OneByte {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let limit = buf.len().min(1);
        self.0.read(&mut buf[..limit])
    }
}

/// A stream whose first read is interrupted, then serves `rest`.
struct InterruptedOnce {
    interrupted: bool,
    rest: Cursor<Vec<u8>>,
}

impl Read for InterruptedOnce {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match std::mem::replace(&mut self.interrupted, true) {
            false => Err(io::ErrorKind::Interrupted.into()),
            true => self.rest.read(buf),
        }
    }
}

/// A stream that fails every read with `kind`.
struct Failing(io::ErrorKind);

impl Read for Failing {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(self.0.into())
    }
}

/// The first packet of `wire`, read through a plain in-memory stream.
fn read_packet(wire: &[u8]) -> Result<Vec<u8>, RspError> {
    Reader::new(Cursor::new(wire.to_vec())).packet()
}

// ---- checksum ----

/// Payloads whose byte sums are worked out by hand.
const CHECKSUMS: [(&[u8], u8); 6] = [
    (b"", 0x00),
    (b"g", 0x67),
    (b"OK", 0x9A),
    (b"S05", 0xB8),
    (b"m0,4", 0xFD),
    // 0xFF + 0x02 = 0x101, which wraps to 0x01.
    (&[0xFF, 0x02], 0x01),
];

fn checksum_failures() -> Vec<(Vec<u8>, u8, u8)> {
    CHECKSUMS
        .iter()
        .map(|(data, want)| (data.to_vec(), *want, checksum(data)))
        .filter(|(_, want, got)| want != got)
        .collect()
}

#[test]
fn given_known_payloads_when_checksummed_then_each_is_the_byte_sum_modulo_256() {
    assert_eq!(checksum_failures(), Vec::<(Vec<u8>, u8, u8)>::new());
}

// ---- frame ----

/// Payloads and the exact bytes on the wire. The escapes are `}` then the byte XOR 0x20, and the
/// checksum covers the escaped bytes.
const FRAMES: [(&[u8], &[u8]); 7] = [
    (b"g", b"$g#67"),
    (b"", b"$#00"),
    // `$` 0x24 -> 0x7D 0x04, sum 0x81.
    (b"$", b"$}\x04#81"),
    // `#` 0x23 -> 0x7D 0x03, sum 0x80.
    (b"#", b"$}\x03#80"),
    // `}` 0x7D -> 0x7D 0x5D, sum 0xDA.
    (b"}", b"$}]#da"),
    // `*` 0x2A -> 0x7D 0x0A, sum 0x87.
    (b"*", b"$}\n#87"),
    // Plain bytes are not escaped, even `+` and `-`: 0x2B + 0x2D = 0x58.
    (b"+-", b"$+-#58"),
];

fn frame_failures() -> Vec<(Vec<u8>, Vec<u8>)> {
    FRAMES
        .iter()
        .map(|(payload, want)| (want.to_vec(), frame(payload)))
        .filter(|(want, got)| want != got)
        .collect()
}

#[test]
fn given_payloads_with_the_four_special_bytes_when_framed_then_each_is_escaped_and_summed() {
    assert_eq!(frame_failures(), Vec::<(Vec<u8>, Vec<u8>)>::new());
}

// ---- decode ----

/// Wire data and what it decodes to.
const DECODES: [(&[u8], &[u8]); 8] = [
    (b"abc", b"abc"),
    (b"", b""),
    (b"}]", b"}"),
    (b"}\x04}\x03}\x0A", b"$#*"),
    // `0* ` is `0` repeated: ' ' is 32, so 3 more zeros.
    (b"0* ", b"0000"),
    // '"' is 34, so 5 more.
    (b"x*\"", b"xxxxxx"),
    // A count of exactly 29 repeats nothing more.
    (b"a*\x1D", b"a"),
    // `!` is 33, so 4 more `b`: five in all.
    (b"ab*!c", b"abbbbbc"),
];

fn decode_failures() -> Vec<(Vec<u8>, Vec<u8>)> {
    DECODES
        .iter()
        .map(|(wire, want)| (want.to_vec(), decode(wire).unwrap_or_default()))
        .filter(|(want, got)| want != got)
        .collect()
}

#[test]
fn given_escapes_and_run_lengths_when_decoded_then_they_expand_to_the_plain_bytes() {
    assert_eq!(decode_failures(), Vec::<(Vec<u8>, Vec<u8>)>::new());
}

#[test]
fn given_a_dangling_escape_when_decoded_then_it_is_malformed() {
    assert!(matches!(
        decode(b"ab}"),
        Err(RspError::Malformed("dangling escape"))
    ));
}

#[test]
fn given_a_dangling_run_length_when_decoded_then_it_is_malformed() {
    assert!(matches!(
        decode(b"a*"),
        Err(RspError::Malformed("dangling run length"))
    ));
}

#[test]
fn given_a_run_length_before_any_byte_when_decoded_then_it_is_malformed() {
    assert!(matches!(
        decode(b"* "),
        Err(RspError::Malformed("run length with no byte"))
    ));
}

#[test]
fn given_run_lengths_that_expand_past_the_limit_when_decoded_then_it_is_too_long() {
    // Each `0*~` is 1 + (126 - 29) = 98 bytes; 11 000 of them pass 1 MiB.
    let wire = b"0*~".repeat(11_000);
    assert!(matches!(decode(&wire), Err(RspError::TooLong)));
}

// ---- Reader ----

#[test]
fn given_acks_before_a_packet_when_read_then_they_are_skipped() {
    assert_eq!(read_packet(b"+-+$g#67").unwrap(), b"g");
}

#[test]
fn given_noise_before_the_dollar_when_read_then_it_is_skipped() {
    assert_eq!(read_packet(b"xyz\n$OK#9a").unwrap(), b"OK");
}

#[test]
fn given_uppercase_checksum_digits_when_read_then_the_packet_is_accepted() {
    assert_eq!(read_packet(b"$S05#B8").unwrap(), b"S05");
}

#[test]
fn given_two_packets_in_one_stream_when_read_twice_then_each_comes_back_in_order() {
    let mut reader = Reader::new(Cursor::new(b"$a#61$b#62".to_vec()));
    let both = (reader.packet().unwrap(), reader.packet().unwrap());
    assert_eq!(both, (b"a".to_vec(), b"b".to_vec()));
}

#[test]
fn given_a_wrong_checksum_when_read_then_it_reports_both_sums() {
    assert!(matches!(
        read_packet(b"$g#00"),
        Err(RspError::Checksum {
            expected: 0x00,
            actual: 0x67
        })
    ));
}

#[test]
fn given_a_non_hex_low_checksum_digit_when_read_then_it_is_malformed() {
    assert!(matches!(
        read_packet(b"$g#6z"),
        Err(RspError::Malformed("bad checksum digit"))
    ));
}

#[test]
fn given_a_non_hex_high_checksum_digit_when_read_then_it_is_malformed() {
    assert!(matches!(
        read_packet(b"$g#z7"),
        Err(RspError::Malformed("bad checksum digit"))
    ));
}

#[test]
fn given_a_good_checksum_over_a_dangling_escape_when_read_then_it_is_malformed() {
    // `}` alone sums to 0x7D.
    assert!(matches!(
        read_packet(b"$}#7d"),
        Err(RspError::Malformed("dangling escape"))
    ));
}

#[test]
fn given_an_empty_stream_when_read_then_it_is_closed() {
    assert!(matches!(read_packet(b""), Err(RspError::Closed)));
}

#[test]
fn given_a_stream_that_ends_inside_the_data_when_read_then_it_is_closed() {
    assert!(matches!(read_packet(b"$abc"), Err(RspError::Closed)));
}

#[test]
fn given_a_stream_that_ends_inside_the_checksum_when_read_then_it_is_closed() {
    assert!(matches!(read_packet(b"$g#6"), Err(RspError::Closed)));
}

#[test]
fn given_acks_and_no_packet_when_read_then_it_is_closed() {
    assert!(matches!(read_packet(b"+++"), Err(RspError::Closed)));
}

#[test]
fn given_data_one_byte_past_the_limit_when_read_then_it_is_too_long_without_a_panic() {
    let mut wire = b"$".to_vec();
    wire.extend(std::iter::repeat_n(b'a', MAX_PACKET + 1));
    wire.extend(b"#00");
    assert!(matches!(read_packet(&wire), Err(RspError::TooLong)));
}

#[test]
fn given_a_runaway_stream_with_no_hash_when_read_then_it_is_too_long_not_closed() {
    let mut wire = b"$".to_vec();
    wire.extend(std::iter::repeat_n(b'a', MAX_PACKET * 2));
    assert!(matches!(read_packet(&wire), Err(RspError::TooLong)));
}

#[test]
fn given_data_of_exactly_the_limit_when_read_then_it_is_accepted() {
    // 97 * 2^20 is a multiple of 256, so the checksum is 00.
    let mut wire = b"$".to_vec();
    wire.extend(std::iter::repeat_n(b'a', MAX_PACKET));
    wire.extend(b"#00");
    assert_eq!(read_packet(&wire).unwrap().len(), MAX_PACKET);
}

#[test]
fn given_a_stream_that_gives_one_byte_per_read_when_read_then_the_packet_is_whole() {
    let mut reader = Reader::new(OneByte(Cursor::new(b"+$OK#9a".to_vec())));
    assert_eq!(reader.packet().unwrap(), b"OK");
}

#[test]
fn given_an_interrupted_read_when_read_then_it_is_retried() {
    let stream = InterruptedOnce {
        interrupted: false,
        rest: Cursor::new(b"$g#67".to_vec()),
    };
    assert_eq!(Reader::new(stream).packet().unwrap(), b"g");
}

#[test]
fn given_a_failing_stream_when_read_then_it_is_an_io_error() {
    let error = Reader::new(Failing(io::ErrorKind::PermissionDenied)).byte();
    assert!(matches!(error, Err(RspError::Io(_))));
}

#[test]
fn given_a_stream_that_reports_eof_as_an_error_when_read_then_it_is_closed() {
    let error = Reader::new(Failing(io::ErrorKind::UnexpectedEof)).byte();
    assert!(matches!(error, Err(RspError::Closed)));
}

// ---- frame then Reader ----

/// Payloads for the round trip, each with a byte the framing treats specially.
fn round_trip_payloads() -> Vec<Vec<u8>> {
    vec![
        b"".to_vec(),
        b"g".to_vec(),
        b"$".to_vec(),
        b"#".to_vec(),
        b"}".to_vec(),
        b"*".to_vec(),
        b"$#}*".to_vec(),
        b"}}}}".to_vec(),
        b"m2000000,4".to_vec(),
        b"+-$+-".to_vec(),
        b"0* ".to_vec(),
        (0..=255).collect(),
    ]
}

/// The payloads that did not survive `frame` then `Reader::packet`.
fn round_trip_failures() -> Vec<Vec<u8>> {
    round_trip_payloads()
        .into_iter()
        .filter(|payload| read_packet(&frame(payload)).ok().as_ref() != Some(payload))
        .collect()
}

#[test]
fn given_payloads_with_escaped_bytes_when_framed_and_read_then_each_comes_back_unchanged() {
    assert_eq!(round_trip_failures(), Vec::<Vec<u8>>::new());
}

// ---- hex_bytes ----

#[test]
fn given_byte_pairs_in_either_case_when_decoded_from_hex_then_they_are_the_bytes() {
    assert_eq!(hex_bytes(b"00fFa5").unwrap(), [0x00, 0xFF, 0xA5]);
}

#[test]
fn given_no_digits_when_decoded_from_hex_then_there_are_no_bytes() {
    assert_eq!(hex_bytes(b"").unwrap(), Vec::<u8>::new());
}

#[test]
fn given_an_odd_number_of_digits_when_decoded_from_hex_then_it_is_malformed() {
    assert!(matches!(
        hex_bytes(b"abc"),
        Err(RspError::Malformed("odd hex length"))
    ));
}

#[test]
fn given_a_non_hex_digit_when_decoded_from_hex_then_it_is_malformed() {
    assert!(matches!(
        hex_bytes(b"0g"),
        Err(RspError::Malformed("bad checksum digit"))
    ));
}

// ---- RspError ----

/// Each error's message, to pin what the tool prints.
fn messages() -> Vec<(String, String)> {
    let cases: [(RspError, &str); 8] = [
        (RspError::Closed, "the stub closed the connection"),
        (
            RspError::Checksum {
                expected: 0x0A,
                actual: 0xB0,
            },
            "bad checksum: packet says 0a, data sums to b0",
        ),
        (
            RspError::Malformed("odd hex length"),
            "malformed packet: odd hex length",
        ),
        (RspError::TooLong, "packet longer than 1048576 bytes"),
        (RspError::Stub(1), "the stub reported error 01"),
        (
            RspError::Unsupported,
            "the stub does not support the request",
        ),
        (RspError::Unexpected("zz".into()), "unexpected reply `zz`"),
        (
            RspError::Forbidden(0xFFFF_0018),
            "refusing to read forbidden address ffff0018",
        ),
    ];
    cases
        .into_iter()
        .map(|(error, want)| (want.to_string(), error.to_string()))
        .filter(|(want, got)| want != got)
        .collect()
}

#[test]
fn given_each_error_kind_when_displayed_then_the_message_names_the_problem() {
    assert_eq!(messages(), Vec::<(String, String)>::new());
}

#[test]
fn given_an_io_error_when_displayed_then_the_message_carries_its_text() {
    let error = RspError::from(io::Error::new(io::ErrorKind::BrokenPipe, "pipe gone"));
    assert_eq!(error.to_string(), "stream error: pipe gone");
}

#[test]
fn given_an_io_error_when_asked_for_its_source_then_it_is_the_io_error() {
    use std::error::Error;
    let error = RspError::from(io::Error::new(io::ErrorKind::BrokenPipe, "pipe gone"));
    assert_eq!(
        error.source().map(ToString::to_string),
        Some("pipe gone".into())
    );
}

#[test]
fn given_a_protocol_error_when_asked_for_its_source_then_there_is_none() {
    use std::error::Error;
    assert!(RspError::Closed.source().is_none());
}

#[test]
fn given_a_stream_of_junk_with_no_packet_start_when_read_then_it_is_malformed_not_a_hang() {
    // `io::repeat` never ends: an unbounded skip loop would never return.
    let mut reader = Reader::new(std::io::repeat(b'x'));
    assert!(matches!(
        reader.packet(),
        Err(RspError::Malformed("no packet start"))
    ));
}
