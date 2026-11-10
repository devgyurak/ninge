// SPDX-License-Identifier: Apache-2.0
//! The LZ77 decompression SWIs (`docs/hw/hle-swi.md`, "The copy, math and decompression SWIs").
//!
//! [`decode`] is the pure decoder: it knows the format and nothing about memory, and talks to a
//! [`Stream`] that supplies the next input byte, takes an output byte and looks one back. The two
//! bus streams below are the two forms of the SWI: byte writes ("WRAM") and halfword writes
//! ("VRAM").

use crate::cpu::Bus;

/// The header's size field: bits 8-31 of the first word.
const SIZE_SHIFT: u32 = 8;
/// A block's length is `(b0 >> 4) + 3`; its distance is the low nibble of `b0` and all of `b1`,
/// plus one.
const LENGTH_SHIFT: u32 = 4;
const MIN_LENGTH: u32 = 3;
const DISTANCE_HIGH_MASK: u8 = 0x0F;

/// Where the decoder reads from and writes to.
pub(super) trait Stream {
    /// The next byte of compressed input.
    fn next(&mut self) -> u8;
    /// Appends one output byte.
    fn put(&mut self, byte: u8);
    /// The output byte `distance` bytes behind the next output position (1 = the last one).
    fn back(&mut self, distance: u32) -> u8;
}

/// Decodes `size` bytes of output from the compressed data the stream supplies, which follows the
/// header. Flag bytes are used from bit 7 down; a 0 bit is a literal, a 1 bit a two-byte block that
/// is copied byte by byte, so it may overlap its own output. A block that would pass `size` is cut.
pub(super) fn decode<S: Stream>(size: u32, stream: &mut S) {
    let mut written = 0u32;
    while written < size {
        let flags = stream.next();
        for bit in (0..8).rev() {
            if written >= size {
                break;
            }
            match flags >> bit & 1 {
                0 => {
                    let byte = stream.next();
                    stream.put(byte);
                    written += 1;
                }
                _ => {
                    let first = stream.next();
                    let second = stream.next();
                    let length = u32::from(first >> LENGTH_SHIFT) + MIN_LENGTH;
                    let distance =
                        (u32::from(first & DISTANCE_HIGH_MASK) << 8 | u32::from(second)) + 1;
                    for _ in 0..length.min(size - written) {
                        let byte = stream.back(distance);
                        stream.put(byte);
                    }
                    written += length.min(size - written);
                }
            }
        }
    }
}

/// Input read from the bus at `at`, then the output position; shared by the two forms.
struct Reader<'a, B> {
    bus: &'a mut B,
    at: u32,
}

impl<B: Bus> Reader<'_, B> {
    fn next(&mut self) -> u8 {
        let byte = self.bus.read8(self.at);
        self.at = self.at.wrapping_add(1);
        byte
    }
}

/// The "WRAM" form: each output byte is a byte write to `dst`.
struct ByteStream<'a, B> {
    input: Reader<'a, B>,
    dst: u32,
    written: u32,
}

impl<B: Bus> Stream for ByteStream<'_, B> {
    fn next(&mut self) -> u8 {
        self.input.next()
    }

    fn put(&mut self, byte: u8) {
        self.input
            .bus
            .write8(self.dst.wrapping_add(self.written), byte);
        self.written = self.written.wrapping_add(1);
    }

    fn back(&mut self, distance: u32) -> u8 {
        self.input
            .bus
            .read8(self.dst.wrapping_add(self.written).wrapping_sub(distance))
    }
}

/// The "VRAM" form: output bytes are collected in pairs, each pair written as one halfword, low
/// byte first.
struct HalfwordStream<'a, B> {
    input: Reader<'a, B>,
    dst: u32,
    written: u32,
    /// The low byte of the halfword being assembled, when `written` is odd.
    pending: u8,
}

impl<B: Bus> Stream for HalfwordStream<'_, B> {
    fn next(&mut self) -> u8 {
        self.input.next()
    }

    // TODO(hw-unverified): an odd size leaves the last byte unwritten (`hle-swi.md`, "Unverified").
    fn put(&mut self, byte: u8) {
        match self.written & 1 {
            0 => self.pending = byte,
            _ => {
                let halfword = u16::from(self.pending) | u16::from(byte) << 8;
                self.input
                    .bus
                    .write16(self.dst.wrapping_add(self.written & !1), halfword);
            }
        }
        self.written = self.written.wrapping_add(1);
    }

    // TODO(hw-unverified): a block at distance 1 reads the byte still waiting in the pair
    // (`hle-swi.md`, "Unverified").
    fn back(&mut self, distance: u32) -> u8 {
        let index = self.written.wrapping_sub(distance);
        match self.written & 1 == 1 && index == self.written - 1 {
            true => self.pending,
            false => self.input.bus.read8(self.dst.wrapping_add(index)),
        }
    }
}

/// SWI 11h: decodes the stream at `src` to byte writes at `dst`.
// TODO(hw-unverified): the type nibble of the header is not checked (`hle-swi.md`, "Unverified").
pub(super) fn decode_wram<B: Bus>(bus: &mut B, src: u32, dst: u32) {
    let size = bus.read32(src) >> SIZE_SHIFT;
    let mut stream = ByteStream {
        input: Reader {
            bus,
            at: src.wrapping_add(4),
        },
        dst,
        written: 0,
    };
    decode(size, &mut stream);
}

/// SWI 12h: decodes the stream at `src` to halfword writes at `dst`.
// TODO(hw-unverified): the callbacks the DS form takes in R3 are not used (`hle-swi.md`,
// "Unverified").
pub(super) fn decode_vram<B: Bus>(bus: &mut B, src: u32, dst: u32) {
    let size = bus.read32(src) >> SIZE_SHIFT;
    let mut stream = HalfwordStream {
        input: Reader {
            bus,
            at: src.wrapping_add(4),
        },
        dst,
        written: 0,
        pending: 0,
    };
    decode(size, &mut stream);
}
