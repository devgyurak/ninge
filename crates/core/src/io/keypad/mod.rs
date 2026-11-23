// SPDX-License-Identifier: Apache-2.0
//! The keypad registers: KEYINPUT and KEYCNT on both CPUs, EXTKEYIN on the ARM7. Note:
//! `docs/hw/keypad.md`.
//!
//! An independent copy for each CPU (`AGENTS.md` §4, "Per-CPU devices"): each CPU has its own
//! KEYCNT. The key registers read the buttons and the hinge of the latched [`Input`]
//! (`Keypad::latch`). EXTKEYIN's pen bit reads the panel of the latched input and the touch
//! controller's pen output, which `Io` hands over in `Cx`. The keypad IRQ is checked when `Io`
//! latches a frame's input and when KEYCNT is written; the hinge IRQ, on the ARM7's copy, when the
//! latch finds the hinge went from closed to open (`keypad.md`, "The keypad IRQ").

use crate::input::Input;
use crate::io::device::{Checked, Cx, Device, Effect, Width, read_bytes, write_bytes};
use crate::io::irq::{KEYPAD_IRQ, Requests, UNFOLD_IRQ};
use serde::{Deserialize, Serialize};

/// KEYINPUT, at `0400_0130h`; the device's register space starts here. Both CPUs.
pub(crate) const KEYINPUT_ADDR: u32 = 0x0400_0130;
/// Bytes of the ARM9's block: KEYINPUT and KEYCNT.
pub(crate) const ARM9_BYTES: u32 = 4;
/// Bytes of the ARM7's block: KEYINPUT, KEYCNT, RCNT (not modeled) and EXTKEYIN.
pub(crate) const ARM7_BYTES: u32 = 8;

/// KEYCNT's offset from KEYINPUT.
const KEYCNT_OFFSET: u32 = 2;
/// EXTKEYIN's offset from KEYINPUT (ARM7 only).
const EXTKEYIN_OFFSET: u32 = 6;

/// KEYINPUT with nothing pressed: the ten key bits set, the rest read 0 (`keypad.md`, "KEYINPUT
/// bits").
// TODO(hw-unverified): bits 10-15 read 0 here; GBATEK only says "not used" (`keypad.md`,
// "Unverified").
const KEYINPUT_RELEASED: u16 = 0x03FF;
/// EXTKEYIN with nothing pressed and the hinge open: X, Y, the debug button and the pen
/// released (bits 0, 1, 3, 6), the unknown bits 2, 4 and 5 set, the hinge (bit 7) clear
/// (`keypad.md`, "EXTKEYIN bits").
// TODO(hw-unverified): the unknown bits and the debug button (`keypad.md`, "Unverified").
const EXTKEYIN_RELEASED: u16 = 0x007F;
/// X and Y sit at bits 10 and 11 of [`crate::input::Buttons`] and at bits 0 and 1 of EXTKEYIN.
const EXT_BUTTON_SHIFT: u32 = 10;
const EXT_BUTTON_MASK: u16 = 0b11;
/// The hinge is bit 7 of EXTKEYIN (`keypad.md`, "EXTKEYIN bits").
const EXT_HINGE_SHIFT: u32 = 7;
/// The pen is bit 6 of EXTKEYIN: 0 while the panel is touched and the touch controller's pen
/// output is enabled (`keypad.md`, "EXTKEYIN bits"; `touch-power.md`, "The pen bit").
const EXT_PEN: u16 = 1 << 6;
/// The KEYCNT bits that are kept: the ten key selects, the IRQ enable (14) and the condition
/// (15) (`keypad.md`, "KEYCNT bits").
// TODO(hw-unverified): whether bits 10-13 are writable (`keypad.md`, "Unverified").
const KEYCNT_MASK: u16 = 0xC3FF;
/// KEYCNT bits 0-9 select the keys that take part, in KEYINPUT order (`keypad.md`, "KEYCNT bits").
/// X and Y are not among them, so they never meet the condition.
const KEYCNT_KEYS: u16 = 0x03FF;
/// KEYCNT bit 14: the keypad IRQ enable.
const KEYCNT_IRQ_ENABLE: u16 = 1 << 14;
/// KEYCNT bit 15: 0 = any selected key held (OR), 1 = every selected key held (AND).
const KEYCNT_AND: u16 = 1 << 15;

/// One CPU's keypad registers: KEYCNT. The key bits read the host's input through the access's
/// context (`device::Cx`), which `Io` latched as the frame began: no keypad keeps a copy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keypad<const ARM9: bool> {
    keycnt: u16,
}

impl<const ARM9: bool> Keypad<ARM9> {
    /// The registers at reset: KEYCNT clear.
    // TODO(hw-unverified): GBATEK states no reset value for KEYCNT (`keypad.md`, "Unverified").
    pub fn new() -> Self {
        Self::default()
    }

    /// KEYCNT as last written, for the debugger.
    pub fn keycnt(&self) -> u16 {
        self.keycnt
    }

    /// Whether the keys KEYCNT selects meet its condition with `input` held: any of them in OR
    /// mode, all of them in AND mode. The IRQ enable is not looked at, so the debugger can show the
    /// condition on its own.
    // TODO(hw-unverified): with no key selected the condition is never met, in OR or AND mode
    // (`keypad.md`, "Unverified").
    pub fn condition_met(&self, input: &Input) -> bool {
        let selected = self.keycnt & KEYCNT_KEYS;
        let held = input.buttons.bits() & selected;
        if self.keycnt & KEYCNT_AND != 0 {
            selected != 0 && held == selected
        } else {
            held != 0
        }
    }

    /// The keypad IF bit for this CPU when the IRQ is enabled and the condition holds with `input`.
    fn keypad_request(&self, input: &Input) -> Requests {
        let raised = self.keycnt & KEYCNT_IRQ_ENABLE != 0 && self.condition_met(input);
        Requests::for_cpu::<ARM9>(u32::from(raised) * KEYPAD_IRQ)
    }

    /// The requests a frame-boundary latch raises on this CPU: the keypad bit when the condition
    /// holds with `new`, and on the ARM7 the unfolding bit when the hinge went from closed in `old`
    /// to open in `new`. Both are sampled here and never again until the next latch or KEYCNT write.
    // TODO(hw-unverified): the sampling points, a request on every latch while the condition
    // holds, and no request when the hinge closes (`keypad.md`, "Unverified").
    pub(crate) fn latch_requests(&self, old: &Input, new: &Input) -> Requests {
        let unfolded = !ARM9 && old.hinge_closed && !new.hinge_closed;
        self.keypad_request(new)
            .merge(Requests::for_cpu::<ARM9>(u32::from(unfolded) * UNFOLD_IRQ))
    }

    /// KEYINPUT: a held key reads 0 (`keypad.md`, "KEYINPUT bits").
    fn keyinput(input: &Input) -> u16 {
        KEYINPUT_RELEASED & !input.buttons.bits()
    }

    /// EXTKEYIN: X and Y read 0 while held, the hinge bit is 1 while closed, and the pen bit reads 0
    /// while the panel is touched and the touch controller's pen output is enabled; the other bits
    /// keep their released values (`keypad.md`, "EXTKEYIN bits").
    // TODO(hw-unverified): the pen bit follows the latched touch state, not the analog line
    // (`touch-power.md`, "Unverified").
    fn extkeyin(input: &Input, pen_irq_enabled: bool) -> u16 {
        let held_xy = (input.buttons.bits() >> EXT_BUTTON_SHIFT) & EXT_BUTTON_MASK;
        let hinge = u16::from(input.hinge_closed) << EXT_HINGE_SHIFT;
        let pen_down = if input.touch.is_some() && pen_irq_enabled {
            EXT_PEN
        } else {
            0
        };
        (EXTKEYIN_RELEASED & !held_xy & !pen_down) | hinge
    }

    /// The byte of the register space at `offset`: a hole, and on the ARM9 EXTKEYIN, read 0.
    fn byte(&self, offset: u32, input: &Input, pen_irq_enabled: bool) -> u8 {
        let keycnt = self.keycnt.to_le_bytes();
        let keys = Self::keyinput(input).to_le_bytes();
        let ext = Self::extkeyin(input, pen_irq_enabled).to_le_bytes();
        match offset {
            0 | 1 => keys[offset as usize],
            KEYCNT_OFFSET | 3 => keycnt[(offset - KEYCNT_OFFSET) as usize],
            EXTKEYIN_OFFSET | 7 if !ARM9 => ext[(offset - EXTKEYIN_OFFSET) as usize],
            _ => 0,
        }
    }
}

impl<const ARM9: bool> Checked for Keypad<ARM9> {}

impl<const ARM9: bool> Device<ARM9> for Keypad<ARM9> {
    fn peek(&self, offset: u32, width: Width, cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| {
            self.byte(at, cx.host, cx.lines.pen_irq_enabled)
        })
    }

    /// Each byte of the write goes to the register at its own offset; only KEYCNT keeps anything,
    /// and only its defined bits. KEYINPUT and EXTKEYIN are read-only. A KEYCNT write requests the
    /// keypad IRQ if its condition already holds.
    // TODO(hw-unverified): a KEYCNT write is a sampling point (`keypad.md`, "Unverified").
    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        let mut keycnt_written = false;
        let effect = write_bytes(offset, width, value, |at, byte| {
            let shift = match at {
                KEYCNT_OFFSET => Some(0),
                3 => Some(8),
                _ => None,
            };
            if let Some(shift) = shift {
                keycnt_written = true;
                let kept = self.keycnt & !(0xFF_u16 << shift);
                self.keycnt = kept | ((u16::from(byte) << shift) & KEYCNT_MASK);
            }
            Effect::default()
        });
        // The condition is checked again as KEYCNT lands, with the buttons latched for the frame
        // (`keypad.md`, "The keypad IRQ").
        if keycnt_written {
            effect.merge(Effect::raising(self.keypad_request(cx.host)))
        } else {
            effect
        }
    }
}

#[cfg(test)]
mod tests;
