// SPDX-License-Identifier: Apache-2.0
//! A GDB remote client for one CPU: registers, single step, continue, software breakpoints and a
//! guarded memory read. Generic over `Read + Write`; the binary hands it a socket.

use std::io::{Read, Write};
use std::ops::Range;

use crate::rsp::{Reader, RspError, frame, hex_bytes};

/// How many times a request is sent again after a `-` or a corrupted reply.
const RETRIES: usize = 3;

/// Why the target stopped, from a stop reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Stopped with a signal (`S<nn>` or `T<nn>...`); 5 is a trap, the normal answer to a step.
    Signal(u8),
    /// The target exited (`W` or `X`).
    Exited,
}

/// One connection to a stub.
#[derive(Debug)]
pub struct GdbClient<T> {
    reader: Reader<T>,
    forbidden: Vec<Range<u32>>,
}

impl<T: Read + Write> GdbClient<T> {
    /// A client over `stream`. A memory read that overlaps a `forbidden` range is refused before
    /// any packet is sent: the BIOS areas go here, so no BIOS byte leaves the oracle.
    pub fn new(stream: T, forbidden: Vec<Range<u32>>) -> Self {
        Self {
            reader: Reader::new(stream),
            forbidden,
        }
    }

    /// Sends one request and returns the decoded reply, acknowledging it. A `-` or a corrupted
    /// reply sends the request again, up to [`RETRIES`] times.
    ///
    /// # Errors
    ///
    /// The [`RspError`] of the stream or the packet; [`RspError::Malformed`] if the stub neither
    /// acknowledges nor refuses.
    pub(crate) fn transact(&mut self, payload: &[u8]) -> Result<Vec<u8>, RspError> {
        let packet = frame(payload);
        for _ in 0..RETRIES {
            let stream = self.reader.get_mut();
            stream.write_all(&packet)?;
            stream.flush()?;
            match self.reader.byte()? {
                b'+' => return self.reply(),
                // The stub did not take the request: send it again.
                b'-' => {}
                _ => return Err(RspError::Malformed("expected an acknowledgement")),
            }
        }
        Err(RspError::Malformed("the stub keeps refusing the request"))
    }

    /// Reads the reply to an accepted request. A corrupted one is answered with `-`, which makes
    /// the stub send it again, so the same read is repeated rather than the request.
    fn reply(&mut self) -> Result<Vec<u8>, RspError> {
        let mut last = RspError::Malformed("no reply");
        for _ in 0..RETRIES {
            match self.reader.packet() {
                Ok(reply) => {
                    self.reader.get_mut().write_all(b"+")?;
                    self.reader.get_mut().flush()?;
                    return Ok(reply);
                }
                Err(error @ (RspError::Checksum { .. } | RspError::Malformed(_))) => {
                    self.reader.get_mut().write_all(b"-")?;
                    self.reader.get_mut().flush()?;
                    last = error;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    /// A request whose reply is `OK`.
    fn expect_ok(&mut self, payload: &[u8]) -> Result<(), RspError> {
        match self.transact(payload)?.as_slice() {
            b"OK" => Ok(()),
            other => Err(reply_error(other)),
        }
    }

    /// The registers as little-endian 32-bit words, from a `g` packet.
    ///
    /// # Errors
    ///
    /// An [`RspError`] if the request fails or the reply is not whole words of hex.
    pub fn registers(&mut self) -> Result<Vec<u32>, RspError> {
        let reply = self.transact(b"g")?;
        if is_error(&reply) {
            return Err(reply_error(&reply));
        }
        let bytes = hex_bytes(&reply)?;
        if !bytes.len().is_multiple_of(4) {
            return Err(RspError::Malformed("register block is not whole words"));
        }
        let (words, _) = bytes.as_chunks::<4>();
        Ok(words.iter().map(|word| u32::from_le_bytes(*word)).collect())
    }

    /// Steps one instruction (`s`).
    ///
    /// # Errors
    ///
    /// An [`RspError`] if the request fails or the reply is not a stop reply.
    pub fn step(&mut self) -> Result<Stop, RspError> {
        let reply = self.transact(b"s")?;
        stop_reply(&reply)
    }

    /// Continues (`c`) until the target stops.
    ///
    /// # Errors
    ///
    /// An [`RspError`] if the request fails or the reply is not a stop reply.
    pub fn resume(&mut self) -> Result<Stop, RspError> {
        let reply = self.transact(b"c")?;
        stop_reply(&reply)
    }

    /// Sets a software breakpoint at `addr`; `kind` is 4 for ARM state, 2 for Thumb.
    ///
    /// # Errors
    ///
    /// An [`RspError`]: [`RspError::Unsupported`] if the stub has no breakpoints.
    pub fn set_breakpoint(&mut self, addr: u32, kind: u32) -> Result<(), RspError> {
        self.expect_ok(format!("Z0,{addr:x},{kind:x}").as_bytes())
    }

    /// Removes the software breakpoint at `addr`.
    ///
    /// # Errors
    ///
    /// An [`RspError`] if the request fails.
    pub fn clear_breakpoint(&mut self, addr: u32, kind: u32) -> Result<(), RspError> {
        self.expect_ok(format!("z0,{addr:x},{kind:x}").as_bytes())
    }

    /// Reads `len` bytes at `addr` (`m`), unless the range overlaps a forbidden one; then nothing
    /// is sent.
    ///
    /// # Errors
    ///
    /// [`RspError::Forbidden`] for a forbidden range, otherwise the [`RspError`] of the request
    /// or of a reply of the wrong length.
    pub fn read_memory(&mut self, addr: u32, len: u32) -> Result<Vec<u8>, RspError> {
        let end = u64::from(addr) + u64::from(len);
        // A read that wraps past the top of the address space would reach the area at 0 without
        // the range check seeing it, so it is refused outright.
        if end > 1 << 32 {
            return Err(RspError::Forbidden(addr));
        }
        let overlaps = |range: &Range<u32>| {
            u64::from(addr) < u64::from(range.end) && u64::from(range.start) < end
        };
        if self.forbidden.iter().any(overlaps) {
            return Err(RspError::Forbidden(addr));
        }
        let reply = self.transact(format!("m{addr:x},{len:x}").as_bytes())?;
        if is_error(&reply) {
            return Err(reply_error(&reply));
        }
        let bytes = hex_bytes(&reply)?;
        match bytes.len() == len as usize {
            true => Ok(bytes),
            false => Err(RspError::Unexpected(
                String::from_utf8_lossy(&reply).into_owned(),
            )),
        }
    }
}

/// Whether `reply` is an empty (unsupported) or an `E<nn>` error rather than data. Data replies
/// are whole bytes in hex, so an even length: a three-byte `E` reply cannot be one.
fn is_error(reply: &[u8]) -> bool {
    reply.is_empty() || (reply.len() == 3 && reply[0] == b'E')
}

/// The error a non-`OK`, non-data reply means: empty is unsupported, `E<nn>` is a stub error.
fn reply_error(reply: &[u8]) -> RspError {
    match reply {
        [] => RspError::Unsupported,
        [b'E', high, low] => hex_bytes(&[*high, *low])
            .map(|code| RspError::Stub(code[0]))
            .unwrap_or_else(|_| RspError::Unexpected(String::from_utf8_lossy(reply).into_owned())),
        other => RspError::Unexpected(String::from_utf8_lossy(other).into_owned()),
    }
}

/// Parses a stop reply: `S<nn>`, `T<nn>...`, `W<nn>` or `X<nn>`.
fn stop_reply(reply: &[u8]) -> Result<Stop, RspError> {
    match reply {
        [b'S' | b'T', high, low, ..] => hex_bytes(&[*high, *low])
            .map(|signal| Stop::Signal(signal[0]))
            .map_err(|_| RspError::Unexpected(String::from_utf8_lossy(reply).into_owned())),
        [b'W' | b'X', ..] => Ok(Stop::Exited),
        [b'E', ..] | [] => Err(reply_error(reply)),
        other => Err(RspError::Unexpected(
            String::from_utf8_lossy(other).into_owned(),
        )),
    }
}
