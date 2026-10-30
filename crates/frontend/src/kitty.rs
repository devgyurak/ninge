// SPDX-License-Identifier: Apache-2.0
//! Kitty graphics protocol encoder: pure functions from pixels to escape bytes.
//!
//! Frames go over as direct (`t=d`) zlib-compressed (`o=z`) 24-bit RGB data under one
//! image id. Re-transmitting an id deletes the existing image and all its placements
//! (protocol doc), so every transmit also creates the virtual placement (`a=T,U=1`).
//! Shared memory (`t=s`) is #6.

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
}
