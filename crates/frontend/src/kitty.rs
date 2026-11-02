// SPDX-License-Identifier: Apache-2.0
//! Kitty graphics protocol encoder: pure functions from pixels to escape bytes.
//!
//! Frames go over either as shared memory (`t=s`, [`transmit_shm`]: only the object's
//! name is sent) or as direct (`t=d`) zlib-compressed (`o=z`) 24-bit RGB data
//! ([`transmit`]), under one image id. Which one is used is decided at startup by a `t=s`
//! query ([`probe_query`], [`parse_probe_reply`]). Re-transmitting an id deletes the
//! existing image and all its placements (protocol doc), so every transmit also creates
//! the virtual placement (`a=T,U=1`).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::ZlibEncoder;
use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use std::io::Write as _;

/// Largest base64 payload per escape sequence, fixed by the protocol.
pub const CHUNK_SIZE: usize = 4096;

const APC_START: &[u8] = b"\x1b_G";
const APC_END: &[u8] = b"\x1b\\";

/// Image id of the support query; no placement is made, so it never shows.
const PROBE_IMAGE_ID: u32 = 31;

/// Primary device attributes request. Every terminal answers it, and it answers after
/// any graphics reply, so its answer marks the end of the wait.
const DA1_REQUEST: &[u8] = b"\x1b[c";

/// Bytes of a frame in the shared-memory object: RGB24 for the whole screen.
pub const FRAME_BYTES: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 3;

/// Bytes of the object the support query names.
pub const PROBE_BYTES: usize = 3;

/// Converts xRGB8888 pixels (`0x00RRGGBB`, high byte ignored) to packed RGB24 bytes.
pub fn rgb24(frame: &[u32]) -> Vec<u8> {
    frame
        .iter()
        .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8])
        .collect()
}

fn push_chunk(out: &mut Vec<u8>, keys: &str, payload: &[u8]) {
    out.extend_from_slice(APC_START);
    out.extend_from_slice(keys.as_bytes());
    out.push(b';');
    out.extend_from_slice(payload);
    out.extend_from_slice(APC_END);
}

/// Encodes one frame as the transmit-and-place sequence for `image_id`.
///
/// The image is placed virtually over `cols` x `rows` cells, which Unicode placeholder
/// cells then reference. `frame` must hold `SCREEN_WIDTH * SCREEN_HEIGHT` pixels. The
/// first chunk carries every key; later chunks carry `m` and `q`. `q=2` suppresses
/// terminal replies on every chunk, because a leaked reply starts with ESC and the input
/// reader would take it for the Esc key.
pub fn transmit(image_id: u32, frame: &[u32], cols: u16, rows: u16) -> Vec<u8> {
    let mut zlib = ZlibEncoder::new(Vec::new(), Compression::fast());
    zlib.write_all(&rgb24(frame))
        .expect("writing to a Vec cannot fail");
    let compressed = zlib.finish().expect("finishing a Vec writer cannot fail");
    let payload = STANDARD.encode(compressed);
    let chunks: Vec<&[u8]> = payload.as_bytes().chunks(CHUNK_SIZE).collect();
    let last = chunks.len() - 1;
    let mut out = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i != last);
        let keys = if i == 0 {
            format!(
                "a=T,U=1,c={cols},r={rows},f=24,s={SCREEN_WIDTH},v={SCREEN_HEIGHT},i={image_id},t=d,o=z,q=2,m={more}"
            )
        } else {
            format!("m={more},q=2")
        };
        push_chunk(&mut out, &keys, chunk);
    }
    out
}

/// Encodes one frame that already sits in the shared-memory object `name`.
///
/// Same placement as [`transmit`]; only the medium differs. The payload is the object's
/// name (base64), `S` its size, and the terminal reads the object and then unlinks it.
/// One chunk always suffices: a name is far below [`CHUNK_SIZE`].
pub fn transmit_shm(image_id: u32, name: &str, cols: u16, rows: u16) -> Vec<u8> {
    let keys = format!(
        "a=T,U=1,c={cols},r={rows},f=24,s={SCREEN_WIDTH},v={SCREEN_HEIGHT},i={image_id},t=s,S={FRAME_BYTES},q=2"
    );
    let mut out = Vec::new();
    push_chunk(&mut out, &keys, STANDARD.encode(name).as_bytes());
    out
}

/// The query that asks whether the terminal accepts `t=s`, followed by a DA1 request.
///
/// `name` is an object of [`PROBE_BYTES`] bytes. `q=0` asks for a reply either way; it
/// arrives before the DA1 answer.
pub fn probe_query(name: &str) -> Vec<u8> {
    let keys = format!("a=q,i={PROBE_IMAGE_ID},t=s,f=24,s=1,v=1,S={PROBE_BYTES},q=0");
    let mut out = Vec::new();
    push_chunk(&mut out, &keys, STANDARD.encode(name).as_bytes());
    out.extend_from_slice(DA1_REQUEST);
    out
}

/// What the terminal said to [`probe_query`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The terminal replied `OK`.
    Supported,
    /// The terminal replied with something else; the text is its message.
    Unsupported(String),
    /// No graphics reply: the terminal ignores the graphics protocol, or the reply is
    /// incomplete.
    NoReply,
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// End offset of the first DA1 answer (`ESC [ ? digits and ; c`) in `buf`.
fn da1_end(buf: &[u8]) -> Option<usize> {
    let start = find(buf, b"\x1b[?")? + 3;
    let len = buf[start..]
        .iter()
        .position(|b| !b.is_ascii_digit() && *b != b';')?;
    (buf[start + len] == b'c').then_some(start + len + 1)
}

/// Whether the DA1 answer has arrived, so nothing more is coming for the query.
pub fn reply_complete(buf: &[u8]) -> bool {
    da1_end(buf).is_some()
}

/// Reads the terminal's answer to [`probe_query`] out of the bytes received.
///
/// Only a complete `ESC _G ... ESC \` reply counts. Anything else, including a DA1
/// answer alone, is [`Probe::NoReply`].
pub fn parse_probe_reply(buf: &[u8]) -> Probe {
    let reply = find(buf, APC_START).and_then(|start| {
        let body = &buf[start + APC_START.len()..];
        find(body, APC_END).map(|end| &body[..end])
    });
    let Some(body) = reply else {
        return Probe::NoReply;
    };
    match body.splitn(2, |&b| b == b';').nth(1) {
        Some(b"OK") => Probe::Supported,
        Some(message) => Probe::Unsupported(String::from_utf8_lossy(message).into_owned()),
        None => Probe::Unsupported("malformed reply".to_string()),
    }
}

#[cfg(test)]
mod tests {
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
    fn given_csi_that_is_not_da1_when_checking_completeness_then_it_is_not_complete() {
        assert_eq!(first_complete(b"\x1b[?25h"), None);
    }
}
