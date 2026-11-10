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
///
/// Every `ESC [ ?` is tried in turn: a private-mode sequence before the answer must not
/// hide it.
fn da1_end(buf: &[u8]) -> Option<usize> {
    buf.windows(3)
        .enumerate()
        .filter(|(_, w)| *w == b"\x1b[?")
        .find_map(|(at, _)| {
            let start = at + 3;
            let len = buf[start..]
                .iter()
                .position(|b| !b.is_ascii_digit() && *b != b';')?;
            (buf[start + len] == b'c').then_some(start + len + 1)
        })
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
mod tests;
