// SPDX-License-Identifier: Apache-2.0
//! How long each geometry command takes the engine (`docs/hw/geometry-engine.md`, "Executing a
//! command"). GBATEK counts bus cycles; the scheduler counts master cycles, twice as many.

use super::MODE_SIMULTANEOUS;

/// Master cycles in one bus cycle (`scheduler.md`, "Clocks").
const MASTER_PER_BUS: u64 = 2;
/// The extra bus cycles a MULT or TRANS takes in matrix mode 2: the position matrix and then the
/// vector matrix.
const SIMULTANEOUS_MODE_EXTRA: u64 = 30;
/// NORMAL takes 9 bus cycles and one more for each enabled light, up to 12.
const NORMAL_LIGHT_CYCLES_CAP: u64 = 3;
/// SWAP_BUFFERS' own execution once the wait for VBlank is over.
const SWAP_BUFFERS_BUS_CYCLES: u64 = 392;

/// Master cycles command `id` occupies the engine, with the matrix mode at its start. An ID that
/// is not a command (the decoder never queues one) costs nothing.
// TODO(hw-unverified): which starred commands take the mode-2 extra, and NORMAL's cost with its
// light count: one more bus cycle for each enabled light up to GBATEK's 12
// (`geometry-engine.md`, "Unverified").
pub(super) fn command_cycles(id: u8, matrix_mode: u8, lights: u32) -> u64 {
    let extra = if matrix_mode == MODE_SIMULTANEOUS {
        SIMULTANEOUS_MODE_EXTRA
    } else {
        0
    };
    let bus = match id {
        0x10 | 0x20 | 0x22 | 0x29..=0x2B | 0x33 | 0x40 | 0x41 | 0x60 => 1,
        0x11 | 0x13 => 17,
        0x12 | 0x14 => 36,
        0x15 => 19,
        0x16 => 34,
        0x17 => 30,
        0x18 => 35 + extra,
        0x19 => 31 + extra,
        0x1A => 28 + extra,
        0x1B => 22,
        0x1C => 22 + extra,
        0x21 => 9 + NORMAL_LIGHT_CYCLES_CAP.min(lights as u64),
        0x23 => 9,
        0x24..=0x28 => 8,
        0x30 | 0x31 => 4,
        0x32 => 6,
        0x34 => 32,
        0x50 => SWAP_BUFFERS_BUS_CYCLES,
        0x70 => 103,
        0x71 => 9,
        0x72 => 5,
        _ => 0,
    };
    bus * MASTER_PER_BUS
}
