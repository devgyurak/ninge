// SPDX-License-Identifier: Apache-2.0
//! The GDB remote serial protocol's packet layer: framing `$<data>#<checksum>`, the escape and
//! run-length rules, and a bounded reader. The protocol is a public standard; nothing here knows
//! which emulator is on the other end.
//!
//! A malformed packet, a bad checksum or a closed stream is an [`RspError`], never a panic: the
//! bytes come from another process.

use std::fmt;
use std::io::{self, Read};

/// The longest packet payload the reader accepts, to bound memory on a runaway stream.
pub const MAX_PACKET: usize = 1 << 20;

/// What went wrong reading or writing a packet.
#[derive(Debug)]
pub enum RspError {
    /// The stream failed.
    Io(io::Error),
    /// The stream ended before a whole packet.
    Closed,
    /// The checksum of a packet did not match its data.
    Checksum {
        /// What the packet said.
        expected: u8,
        /// What its data sums to.
        actual: u8,
    },
    /// The bytes are not a packet: a bad checksum digit, a dangling escape or run length.
    Malformed(&'static str),
    /// A payload grew past [`MAX_PACKET`].
    TooLong,
    /// The stub replied `E<nn>`.
    Stub(u8),
    /// The stub does not support the request (an empty reply).
    Unsupported,
    /// The stub's reply is not what the request expects.
    Unexpected(String),
    /// The request would have read a forbidden address range (BIOS bytes never leave the
    /// oracle, `docs/contributing/trace-oracle.md`).
    Forbidden(u32),
}

impl fmt::Display for RspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "stream error: {error}"),
            Self::Closed => write!(f, "the stub closed the connection"),
            Self::Checksum { expected, actual } => {
                write!(
                    f,
                    "bad checksum: packet says {expected:02x}, data sums to {actual:02x}"
                )
            }
            Self::Malformed(why) => write!(f, "malformed packet: {why}"),
            Self::TooLong => write!(f, "packet longer than {MAX_PACKET} bytes"),
            Self::Stub(code) => write!(f, "the stub reported error {code:02x}"),
            Self::Unsupported => write!(f, "the stub does not support the request"),
            Self::Unexpected(reply) => write!(f, "unexpected reply `{reply}`"),
            Self::Forbidden(addr) => write!(f, "refusing to read forbidden address {addr:08x}"),
        }
    }
}

impl std::error::Error for RspError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for RspError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::UnexpectedEof => Self::Closed,
            _ => Self::Io(error),
        }
    }
}

/// The checksum of a packet's data: the sum of its bytes modulo 256.
pub fn checksum(data: &[u8]) -> u8 {
    data.iter().fold(0, |sum, byte| sum.wrapping_add(*byte))
}

/// Frames `payload` as `$<escaped data>#<checksum>`. The bytes `$`, `#`, `}` and `*` are escaped
/// as `}` followed by the byte XOR 0x20.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(payload.len());
    for &byte in payload {
        match byte {
            b'$' | b'#' | b'}' | b'*' => data.extend([b'}', byte ^ 0x20]),
            _ => data.push(byte),
        }
    }
    let sum = checksum(&data);
    let mut packet = Vec::with_capacity(data.len() + 4);
    packet.push(b'$');
    packet.extend(data);
    packet.extend(format!("#{sum:02x}").bytes());
    packet
}

/// Decodes `data` (the bytes between `$` and `#`): undoes the `}` escape and expands `*` run
/// lengths (the byte before `*` repeats `n - 29` more times, `n` being the byte after it).
///
/// # Errors
///
/// [`RspError::Malformed`] for a dangling escape or run length, [`RspError::TooLong`] for an
/// expansion past [`MAX_PACKET`].
pub fn decode(data: &[u8]) -> Result<Vec<u8>, RspError> {
    let mut out: Vec<u8> = Vec::with_capacity(data.len());
    let mut bytes = data.iter().copied();
    while let Some(byte) = bytes.next() {
        match byte {
            b'}' => {
                let escaped = bytes.next().ok_or(RspError::Malformed("dangling escape"))?;
                out.push(escaped ^ 0x20);
            }
            b'*' => {
                let count = bytes
                    .next()
                    .ok_or(RspError::Malformed("dangling run length"))?;
                let last = *out
                    .last()
                    .ok_or(RspError::Malformed("run length with no byte"))?;
                let repeat = usize::from(count.saturating_sub(29));
                out.extend(std::iter::repeat_n(last, repeat));
            }
            _ => out.push(byte),
        }
        if out.len() > MAX_PACKET {
            return Err(RspError::TooLong);
        }
    }
    Ok(out)
}

/// A byte stream with the one byte of lookahead the packet reader needs, buffered so a socket
/// is read in blocks.
#[derive(Debug)]
pub struct Reader<R> {
    inner: R,
    buffer: Vec<u8>,
    at: usize,
}

impl<R: Read> Reader<R> {
    /// Wraps `inner`.
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            at: 0,
        }
    }

    /// The wrapped stream, to write to it.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// The next byte.
    ///
    /// # Errors
    ///
    /// [`RspError::Closed`] at the end of the stream, [`RspError::Io`] for a stream error.
    pub fn byte(&mut self) -> Result<u8, RspError> {
        if self.at == self.buffer.len() {
            self.buffer.resize(4096, 0);
            let read = loop {
                match self.inner.read(&mut self.buffer) {
                    Ok(read) => break read,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        self.buffer.clear();
                        self.at = 0;
                        return Err(error.into());
                    }
                }
            };
            self.buffer.truncate(read);
            self.at = 0;
            if read == 0 {
                return Err(RspError::Closed);
            }
        }
        let byte = self.buffer[self.at];
        self.at += 1;
        Ok(byte)
    }

    /// Reads one packet and returns its decoded data. Acknowledgement bytes (`+`, `-`) and
    /// anything else before the `$` are skipped.
    ///
    /// # Errors
    ///
    /// [`RspError::Checksum`] if the checksum differs, [`RspError::Malformed`] for a bad checksum
    /// digit or a bad escape, [`RspError::TooLong`] past [`MAX_PACKET`], [`RspError::Closed`] or
    /// [`RspError::Io`] from the stream.
    pub fn packet(&mut self) -> Result<Vec<u8>, RspError> {
        // Bounded, like the data: a stub that streams junk forever must not hang the tool.
        let mut skipped = 0;
        while self.byte()? != b'$' {
            skipped += 1;
            if skipped > MAX_PACKET {
                return Err(RspError::Malformed("no packet start"));
            }
        }
        let mut data = Vec::new();
        loop {
            match self.byte()? {
                b'#' => break,
                byte => data.push(byte),
            }
            if data.len() > MAX_PACKET {
                return Err(RspError::TooLong);
            }
        }
        let high = digit(self.byte()?)?;
        let low = digit(self.byte()?)?;
        let expected = high << 4 | low;
        let actual = checksum(&data);
        if expected != actual {
            return Err(RspError::Checksum { expected, actual });
        }
        decode(&data)
    }
}

/// One hex digit, either case.
fn digit(byte: u8) -> Result<u8, RspError> {
    char::from(byte)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(RspError::Malformed("bad checksum digit"))
}

/// Decodes a hex string of byte pairs.
///
/// # Errors
///
/// [`RspError::Malformed`] for an odd length or a non-hex digit.
pub fn hex_bytes(text: &[u8]) -> Result<Vec<u8>, RspError> {
    if !text.len().is_multiple_of(2) {
        return Err(RspError::Malformed("odd hex length"));
    }
    text.chunks(2)
        .map(|pair| Ok(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests;
