// SPDX-License-Identifier: Apache-2.0
//! The display's position in the frame and the registers that report it: VCOUNT and each CPU's
//! DISPSTAT (`docs/hw/display-timing.md`).
//!
//! The scheduler fires one event at the start of every line and one at the point in the line
//! where HBlank begins; this module holds what those events change and says which interrupt
//! requests they raise. The machine delivers the requests.
//!
//! One instance serves both CPUs: VCOUNT is shared and DISPSTAT is one per CPU, so the accessors
//! are generic over which CPU asks. That is the "seen by both CPUs" class of `AGENTS.md` §4
//! ("Per-CPU devices"), whatever per-CPU registers sit inside.

use crate::io::device::{Device, Effect, Width, read_bytes, write_bytes};
use crate::io::irq::{HBLANK_IRQ, Requests, VBLANK_IRQ, VCOUNT_IRQ};
use serde::{Deserialize, Serialize};

/// DISPSTAT, at `0400_0004h` (`display-timing.md`, "Registers").
pub const DISPSTAT_ADDR: u32 = 0x0400_0004;
/// VCOUNT, at `0400_0006h` (`display-timing.md`, "Registers").
pub const VCOUNT_ADDR: u32 = 0x0400_0006;
/// VCOUNT's offset in the register space of [`Timing`], which starts at DISPSTAT.
const VCOUNT_OFFSET: u32 = VCOUNT_ADDR - DISPSTAT_ADDR;

/// Lines in a frame (`display-timing.md`, "Frame layout").
pub const LINES: u16 = 263;
/// The first VBlank line: lines 0-191 are visible (`display-timing.md`, "Frame layout").
pub const FIRST_VBLANK_LINE: u16 = 192;
/// The line at whose start the VBlank flag clears again (`display-timing.md`, "Flags").
// TODO(hw-unverified): the exact cycle within the line is not stated
// (`display-timing.md`, "Unverified").
pub const VBLANK_END_LINE: u16 = 262;

/// DISPSTAT bit 0: VBlank flag (`display-timing.md`, "DISPSTAT bits").
pub const VBLANK_FLAG: u16 = 1 << 0;
/// DISPSTAT bit 1: HBlank flag.
pub const HBLANK_FLAG: u16 = 1 << 1;
/// DISPSTAT bit 2: VCount-match flag.
pub const VCOUNT_FLAG: u16 = 1 << 2;
/// DISPSTAT bit 3: VBlank IRQ enable.
pub const VBLANK_IRQ_ENABLE: u16 = 1 << 3;
/// DISPSTAT bit 4: HBlank IRQ enable.
pub const HBLANK_IRQ_ENABLE: u16 = 1 << 4;
/// DISPSTAT bit 5: VCount IRQ enable.
pub const VCOUNT_IRQ_ENABLE: u16 = 1 << 5;
/// DISPSTAT bit 7: bit 8 of the VCount setting.
const SETTING_MSB: u16 = 1 << 7;
/// DISPSTAT bits 8-15: bits 0-7 of the VCount setting.
const SETTING_LOW_SHIFT: u32 = 8;
/// The DISPSTAT bits software can write: the enables, the setting's top bit and its low byte
/// (`display-timing.md`, "DISPSTAT bits").
pub const WRITABLE: u16 = VBLANK_IRQ_ENABLE
    | HBLANK_IRQ_ENABLE
    | VCOUNT_IRQ_ENABLE
    | SETTING_MSB
    | (0xFF << SETTING_LOW_SHIFT);

/// One CPU's DISPSTAT: what it wrote, and its own VCount-match flag, which follows its own
/// setting (`display-timing.md`, "Flags").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stat {
    /// The writable bits as written.
    control: u16,
    vcount_match: bool,
}

impl Stat {
    const fn new() -> Self {
        Self {
            control: 0,
            vcount_match: false,
        }
    }

    /// The 9-bit VCount setting: bit 7 is its top bit, bits 8-15 its low byte.
    fn setting(self) -> u16 {
        ((self.control >> SETTING_LOW_SHIFT) & 0xFF) | ((self.control & SETTING_MSB) << 1)
    }

    /// The IF mask for `enable` when this CPU has it set.
    fn if_enabled(self, enable: u16, request: u32) -> u32 {
        match self.control & enable {
            0 => 0,
            _ => request,
        }
    }
}

/// VCOUNT, the flags both CPUs share, and each CPU's DISPSTAT. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    vcount: u16,
    vblank: bool,
    hblank: bool,
    // TODO(hw-unverified): each CPU has its own enables and setting; GBATEK lists the register
    // for both but does not say they are separate (`display-timing.md`, "Unverified").
    arm9: Stat,
    arm7: Stat,
}

impl Timing {
    /// The display at the start of line 0 with everything else 0.
    // TODO(hw-unverified): the reset values are not stated (`display-timing.md`, "Unverified").
    pub fn new() -> Self {
        Self {
            vcount: 0,
            vblank: false,
            hblank: false,
            arm9: Stat::new(),
            arm7: Stat::new(),
        }
    }

    /// VCOUNT: the current line, 0-262.
    pub fn vcount(&self) -> u16 {
        self.vcount
    }

    /// DISPSTAT as the ARM9 reads it when `ARM9`, else as the ARM7 does.
    pub fn dispstat<const ARM9: bool>(&self) -> u16 {
        let stat = self.stat::<ARM9>();
        let flags = [
            (self.vblank, VBLANK_FLAG),
            (self.hblank, HBLANK_FLAG),
            (stat.vcount_match, VCOUNT_FLAG),
        ]
        .iter()
        .filter(|&&(set, _)| set)
        .fold(0, |bits, &(_, flag)| bits | flag);
        stat.control | flags
    }

    fn stat<const ARM9: bool>(&self) -> &Stat {
        match ARM9 {
            true => &self.arm9,
            false => &self.arm7,
        }
    }

    fn stat_mut<const ARM9: bool>(&mut self) -> &mut Stat {
        match ARM9 {
            true => &mut self.arm9,
            false => &mut self.arm7,
        }
    }

    /// A new line begins: VCOUNT advances (262 wraps to 0), the HBlank flag clears, the VBlank
    /// flag follows the line, and each CPU's VCount-match flag is compared again
    /// (`display-timing.md`, "Flags"). Returns the requests of the flags that rose.
    // TODO(hw-unverified): the VCount-match flag is evaluated only here, not on a write to the
    // setting; the VBlank IRQ is raised at the line start (`display-timing.md`, "Unverified").
    pub(crate) fn line_started(&mut self) -> Requests {
        self.vcount = (self.vcount + 1) % LINES;
        self.hblank = false;
        let was_vblank = self.vblank;
        self.vblank = (FIRST_VBLANK_LINE..VBLANK_END_LINE).contains(&self.vcount);
        let vblank_rose = self.vblank && !was_vblank;
        let vcount = self.vcount;
        let mut requests = Requests::default();
        [
            (&mut self.arm9, &mut requests.arm9),
            (&mut self.arm7, &mut requests.arm7),
        ]
        .into_iter()
        .for_each(|(stat, mask)| {
            let matched = stat.setting() == vcount;
            let rose = matched && !stat.vcount_match;
            stat.vcount_match = matched;
            *mask |= match vblank_rose {
                true => stat.if_enabled(VBLANK_IRQ_ENABLE, VBLANK_IRQ),
                false => 0,
            } | match rose {
                true => stat.if_enabled(VCOUNT_IRQ_ENABLE, VCOUNT_IRQ),
                false => 0,
            };
        });
        requests
    }

    /// HBlank begins in the current line: the flag sets and each CPU with the enable gets a
    /// request (`display-timing.md`, "Flags", "Interrupt requests").
    // TODO(hw-unverified): the HBlank flag and request also occur in the VBlank lines
    // (`display-timing.md`, "Unverified").
    pub(crate) fn hblank_started(&mut self) -> Requests {
        self.hblank = true;
        Requests {
            arm9: self.arm9.if_enabled(HBLANK_IRQ_ENABLE, HBLANK_IRQ),
            arm7: self.arm7.if_enabled(HBLANK_IRQ_ENABLE, HBLANK_IRQ),
        }
    }

    /// The byte at `offset` of DISPSTAT or VCOUNT for the ARM9 when `ARM9`, else the ARM7; 0
    /// where no register is.
    fn read_byte<const ARM9: bool>(&self, offset: u32) -> u8 {
        let (value, base) = match offset {
            0 | 1 => (self.dispstat::<ARM9>(), 0),
            VCOUNT_OFFSET | 3 => (self.vcount, VCOUNT_OFFSET),
            _ => return 0,
        };
        (value >> (8 * (offset - base))) as u8
    }

    /// Writes a byte of DISPSTAT; the flags and the unused bit 6 ignore it. A byte of VCOUNT
    /// is ignored.
    // TODO(hw-unverified): a write to a flag bit or bit 6 changes nothing
    // (`display-timing.md`, "Unverified").
    // TODO(hw-unverified): GBATEK says VCOUNT is writable but not what a write does; it is
    // dropped here (`display-timing.md`, "Unverified").
    fn write_byte<const ARM9: bool>(&mut self, offset: u32, value: u8) {
        let stat = self.stat_mut::<ARM9>();
        match offset {
            0 => {
                stat.control = (stat.control & 0xFF00) | (u16::from(value) & WRITABLE & 0x00FF);
            }
            1 => {
                stat.control = (stat.control & 0x00FF) | (u16::from(value) << SETTING_LOW_SHIFT);
            }
            _ => {}
        }
    }

    /// Whether decoded state could have been reached: VCOUNT is a line, the VBlank flag agrees
    /// with it, and each DISPSTAT holds only writable bits. A savestate that decodes to
    /// anything else is corrupt (`savestate`).
    pub(crate) fn is_well_formed(&self) -> bool {
        self.vcount < LINES
            && self.vblank == (FIRST_VBLANK_LINE..VBLANK_END_LINE).contains(&self.vcount)
            && (self.arm9.control | self.arm7.control) & !WRITABLE == 0
    }
}

#[cfg(test)]
impl Timing {
    /// Sets VCOUNT unchecked, to build a corrupt state in a test.
    pub(crate) fn set_vcount_for_test(&mut self, vcount: u16) {
        self.vcount = vcount;
    }

    /// Sets the VBlank flag unchecked, to build a corrupt state in a test.
    pub(crate) fn set_vblank_for_test(&mut self, vblank: bool) {
        self.vblank = vblank;
    }

    /// Sets the ARM7's stored DISPSTAT bits unchecked, to build a corrupt state in a test.
    pub(crate) fn set_arm7_control_for_test(&mut self, control: u16) {
        self.arm7.control = control;
    }
}

/// One instance serves both CPUs ("seen by both CPUs", `AGENTS.md` §4). Its registers are bytes;
/// a wider access is split into them.
impl<const ARM9: bool> Device<ARM9> for Timing {
    fn peek(&self, offset: u32, width: Width, _now: u64) -> u32 {
        read_bytes(offset, width, |at| self.read_byte::<ARM9>(at))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, _now: u64) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte::<ARM9>(at, byte);
            Effect::default()
        })
    }
}

impl Default for Timing {
    fn default() -> Self {
        Self::new()
    }
}
