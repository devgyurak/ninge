// SPDX-License-Identifier: Apache-2.0
//! GetCRC16 (`docs/hw/hle-swi.md`, "The copy, math and decompression SWIs").

use crate::cart::crc16_update;
use crate::cpu::Bus;

/// GetCRC16: CRC-16/MODBUS continued from `initial` over `length / 2` halfwords at `address`, each
/// fed low byte first. An odd length leaves its last byte out.
// TODO(hw-unverified): the odd-length rule and the untouched `R1`-`R3` are the model's
// (`hle-swi.md`, "Unverified").
pub(super) fn crc16_halfwords<B: Bus>(bus: &mut B, initial: u16, address: u32, length: u32) -> u16 {
    (0..length / 2).fold(initial, |crc, i| {
        let [low, high] = bus.read16(address.wrapping_add(2 * i)).to_le_bytes();
        crc16_update(crc16_update(crc, low), high)
    })
}
