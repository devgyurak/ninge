// SPDX-License-Identifier: Apache-2.0
//! Tests of `kitty`.

use super::*;
use flate2::read::ZlibDecoder;
use std::io::Read as _;

/// Splits a stream into `(keys, payload)` per sequence; `None` unless the stream is
/// exactly a series of `ESC _G keys ; payload ESC \` sequences.
fn parse(bytes: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.strip_suffix("\x1b\\")?
        .split("\x1b\\")
        .map(|seq| {
            let body = seq.strip_prefix("\x1b_G")?;
            let (keys, payload) = body.split_once(';')?;
            Some((keys.to_string(), payload.as_bytes().to_vec()))
        })
        .collect()
}

fn seqs(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    parse(bytes).expect("stream is well-formed ESC _G ... ESC \\ sequences")
}

fn gradient() -> Vec<u32> {
    (0..(SCREEN_WIDTH * SCREEN_HEIGHT) as u32)
        .map(|i| i.wrapping_mul(2_654_435_761) & 0x00FF_FFFF)
        .collect()
}

fn decoded(bytes: &[u8]) -> Vec<u8> {
    let payload: Vec<u8> = seqs(bytes).into_iter().flat_map(|(_, p)| p).collect();
    let raw = STANDARD.decode(payload).expect("valid base64");
    let mut out = Vec::new();
    ZlibDecoder::new(raw.as_slice())
        .read_to_end(&mut out)
        .expect("valid zlib");
    out
}

#[test]
fn given_pixel_with_high_byte_when_converting_then_rgb_order_and_high_byte_dropped() {
    assert_eq!(
        rgb24(&[0xFF12_3456, 0x0000_00FF]),
        [0x12, 0x34, 0x56, 0, 0, 0xFF]
    );
}

#[test]
fn given_frame_when_transmitting_then_first_chunk_carries_all_keys() {
    let seqs = seqs(&transmit(7, &gradient(), 40, 30));
    assert!(
        seqs[0]
            .0
            .starts_with("a=T,U=1,c=40,r=30,f=24,s=256,v=192,i=7,t=d,o=z,q=2,m=")
    );
}

#[test]
fn given_large_frame_when_transmitting_then_later_chunks_carry_only_m_and_q_flags() {
    let seqs = seqs(&transmit(7, &gradient(), 40, 30));
    let later: Vec<&str> = seqs[1..].iter().map(|s| s.0.as_str()).collect();
    let expected: Vec<String> = (2..=seqs.len())
        .map(|n| format!("m={},q=2", u8::from(n != seqs.len())))
        .collect();
    assert_eq!(
        (seqs.len() > 1, later),
        (true, expected.iter().map(String::as_str).collect())
    );
}

#[test]
fn given_large_frame_when_transmitting_then_every_payload_fits_the_chunk_limit() {
    let largest = seqs(&transmit(7, &gradient(), 40, 30))
        .iter()
        .map(|s| s.1.len())
        .max();
    assert!(largest.is_some_and(|n| n <= CHUNK_SIZE));
}

#[test]
fn given_frame_when_transmitting_then_payload_round_trips_to_rgb24() {
    let frame = gradient();
    assert_eq!(decoded(&transmit(7, &frame, 40, 30)), rgb24(&frame));
}

#[test]
fn given_small_input_when_transmitting_then_single_chunk_has_m_zero() {
    let seqs = seqs(&transmit(1, &vec![0; SCREEN_WIDTH * SCREEN_HEIGHT], 40, 30));
    assert_eq!((seqs.len(), seqs[0].0.ends_with("m=0")), (1, true));
}

#[test]
fn given_transmit_when_parsing_strictly_then_stream_is_exact_sequences() {
    assert!(parse(&transmit(7, &gradient(), 40, 30)).is_some());
}

#[test]
fn given_broken_terminator_when_parsing_strictly_then_stream_is_rejected() {
    assert_eq!(parse(b"\x1b_Gm=0;abc\x1bX"), None);
}

#[test]
fn given_stray_bytes_between_sequences_when_parsing_strictly_then_stream_is_rejected() {
    assert_eq!(parse(b"\x1b_Gm=1;a\x1b\\zz\x1b_Gm=0;b\x1b\\"), None);
}
fn reply(keys_and_message: &str) -> Vec<u8> {
    format!("\x1b_G{keys_and_message}\x1b\\").into_bytes()
}

const DA1_ANSWER: &[u8] = b"\x1b[?62;22;52c";

/// Probes with `(label, bytes, expected)`; returns the labels that differ.
fn probe_failures(cases: &[(&str, Vec<u8>, Probe)]) -> Vec<String> {
    cases
        .iter()
        .filter(|(_, bytes, expected)| parse_probe_reply(bytes) != *expected)
        .map(|(label, ..)| (*label).to_string())
        .collect()
}

fn joined(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

#[test]
fn given_shm_name_when_transmitting_shm_then_keys_name_the_medium_and_size() {
    let seqs = seqs(&transmit_shm(7, "/ninge-1-2", 40, 30));
    assert_eq!(
        seqs[0].0,
        "a=T,U=1,c=40,r=30,f=24,s=256,v=192,i=7,t=s,S=147456,q=2"
    );
}

#[test]
fn given_shm_name_when_transmitting_shm_then_payload_decodes_to_the_name_in_one_chunk() {
    let seqs = seqs(&transmit_shm(7, "/ninge-1-2", 40, 30));
    let name = STANDARD.decode(&seqs[0].1).expect("valid base64");
    assert_eq!((seqs.len(), name), (1, b"/ninge-1-2".to_vec()));
}

#[test]
fn given_name_when_building_probe_query_then_query_keys_payload_and_da1_follow_in_order() {
    let query = probe_query("/ninge-1-2");
    let (graphics, da1) = query.split_at(query.len() - 3);
    let seqs = seqs(graphics);
    assert_eq!(
        (seqs[0].0.as_str(), STANDARD.decode(&seqs[0].1).ok(), da1),
        (
            "a=q,i=31,t=s,f=24,s=1,v=1,S=3,q=0",
            Some(b"/ninge-1-2".to_vec()),
            b"\x1b[c".as_slice()
        )
    );
}

#[test]
fn given_assorted_replies_when_parsing_probe_then_each_is_classified() {
    let ok = reply("i=31;OK");
    let refused = reply("i=31;EINVAL:unsupported transmission medium");
    let cases = [
        ("ok alone", ok.clone(), Probe::Supported),
        ("ok then da1", joined(&[&ok, DA1_ANSWER]), Probe::Supported),
        (
            "error text",
            refused.clone(),
            Probe::Unsupported("EINVAL:unsupported transmission medium".to_string()),
        ),
        (
            "error then da1",
            joined(&[&refused, DA1_ANSWER]),
            Probe::Unsupported("EINVAL:unsupported transmission medium".to_string()),
        ),
        ("da1 only", DA1_ANSWER.to_vec(), Probe::NoReply),
        ("nothing", Vec::new(), Probe::NoReply),
        (
            "ok split before terminator",
            b"\x1b_Gi=31;OK".to_vec(),
            Probe::NoReply,
        ),
        (
            "reply with no message",
            reply("i=31"),
            Probe::Unsupported("malformed reply".to_string()),
        ),
        (
            "echoed noise before the reply",
            joined(&[b"junk", &ok]),
            Probe::Supported,
        ),
    ];
    assert_eq!(probe_failures(&cases), Vec::<String>::new());
}

/// Feeds `bytes` one at a time and returns the prefix lengths at which the reply
/// first reads as complete.
fn first_complete(bytes: &[u8]) -> Option<usize> {
    (0..=bytes.len()).find(|&n| reply_complete(&bytes[..n]))
}

#[test]
fn given_da1_answer_arriving_in_pieces_when_checking_completeness_then_it_completes_at_the_final_byte()
 {
    let bytes = joined(&[&reply("i=31;OK"), DA1_ANSWER]);
    assert_eq!(first_complete(&bytes), Some(bytes.len()));
}

#[test]
fn given_graphics_reply_without_da1_when_checking_completeness_then_it_is_not_complete() {
    assert_eq!(first_complete(&reply("i=31;OK")), None);
}

#[test]
fn given_private_csi_before_da1_when_checking_completeness_then_the_da1_still_completes() {
    let bytes = joined(&[b"\x1b[?25h", DA1_ANSWER]);
    assert_eq!(first_complete(&bytes), Some(bytes.len()));
}

#[test]
fn given_csi_that_is_not_da1_when_checking_completeness_then_it_is_not_complete() {
    assert_eq!(first_complete(b"\x1b[?25h"), None);
}
