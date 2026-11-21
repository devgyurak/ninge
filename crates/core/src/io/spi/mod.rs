// SPDX-License-Identifier: Apache-2.0
//! The ARM7's SPI bus: SPICNT and SPIDATA, the power-management device on select 0, the firmware
//! flash on select 1 and the touch-screen controller on select 2. Note: `docs/hw/spi.md`; the
//! firmware is `docs/hw/firmware.md`, the other two `docs/hw/touch-power.md`.
//!
//! A plain device for one CPU (`AGENTS.md` §4, "Per-CPU devices"): only the ARM7 reaches the
//! registers. Like the cartridge's SPI side, a unit takes a time that follows from the clock, so
//! the busy flag is computed from the access time, with no event; the device's one event slot is
//! the completion interrupt, queued only when SPICNT bit 14 is set.
//!
//! The firmware chip is the save chip's FLASH model (`cart::save`) holding the synthetic image
//! (`spi::firmware`). The touch controller reads the panel from the access's host data (`Cx`), and
//! `Io` reads its pen output to hand to the keypad, so no device reaches another.

use crate::cart::save::{SaveChip, SaveType};
use crate::input::Touch;
use crate::io::device::{Checked, Cx, Device, Effect, Fired, Width, read_bytes, write_bytes};
use crate::io::irq::{Requests, SPI_IRQ};
use crate::spi::{FIRMWARE_LEN, FirmwareSettings, PowerManagement, Tsc, build};
use serde::{Deserialize, Serialize};

/// SPICNT, at `0400_01C0h`; the device's register space starts here.
pub(crate) const SPICNT_ADDR: u32 = 0x0400_01C0;
/// Bytes of the block the device owns: SPICNT and SPIDATA.
pub(crate) const BLOCK_BYTES: u32 = 4;
/// The byte offset of SPIDATA's low byte, the one whose write starts a transfer.
const SPIDATA_OFFSET: u32 = 2;

/// SPICNT bits 0-1: the baud rate, 4 MHz, 2 MHz, 1 MHz, 512 kHz.
const BAUD_MASK: u16 = 0b11;
/// SPICNT bit 7: a unit is on the wire. Read-only, computed from the time.
const BUSY: u16 = 1 << 7;
/// SPICNT bits 8-9: the device select.
const DEVICE_SHIFT: u32 = 8;
const DEVICE_MASK: u16 = 0b11;
/// Device select 0: the power-management device.
const DEVICE_POWER: u16 = 0;
/// Device select 1: the firmware flash.
const DEVICE_FIRMWARE: u16 = 1;
/// Device select 2: the touch-screen controller.
const DEVICE_TOUCH: u16 = 2;
/// SPICNT bit 10: the 16-bit ("bugged") transfer size.
const WIDE: u16 = 1 << 10;
/// SPICNT bit 11: keep the chip selected after the transfer.
const HOLD: u16 = 1 << 11;
/// SPICNT bit 14: interrupt request.
const IRQ: u16 = 1 << 14;
/// SPICNT bit 15: bus enable.
const ENABLE: u16 = 1 << 15;
/// The SPICNT bits that are kept: baud rate (0-1), device (8-9), size (10), hold (11), IRQ (14)
/// and enable (15) (`spi.md`, "SPICNT bits"). Bits 2-6 and 12-13 read zero.
const SPICNT_MASK: u16 = 0xCF03;

/// Master cycles of an 8-bit unit at baud 0: 8 bits of 8 bus clocks, two master cycles a bus
/// clock; each baud step doubles it, and a 16-bit unit doubles it again (`spi.md`, "Baud rate and
/// the busy time").
// TODO(hw-unverified): the divisor behind "4 MHz" and the rest is the bus clock over 8, 16, 32, 64
// (`spi.md`, "Unverified").
const UNIT_CYCLES: u64 = 128;
/// The longest a unit takes: baud 3, 16 bits.
const LONGEST_UNIT: u64 = UNIT_CYCLES << (BAUD_MASK + 1);

/// What a device answers when nothing drives the output line: it floats high.
// TODO(hw-unverified): what select 3 answers (`spi.md`, "Unverified").
const FLOATING: u8 = 0xFF;

/// SPICNT and SPIDATA, and the firmware chip behind them. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spi {
    /// SPICNT, the kept bits.
    spicnt: u16,
    /// The value the last transfer received: what SPIDATA reads.
    // TODO(hw-unverified): SPIDATA's reset value and its read while busy (`spi.md`, "Unverified").
    data: u8,
    /// The master time the unit on the wire is done; busy until then.
    done_at: u64,
    /// The master time the completion interrupt is due: the device's one event slot. `None` for a
    /// transfer that started with SPICNT bit 14 clear.
    irq_due: Option<u64>,
    /// The firmware flash.
    firmware: SaveChip,
    /// The power-management device (select 0).
    power: PowerManagement,
    /// The touch-screen controller (select 2).
    touch: Tsc,
}

impl Spi {
    /// The bus at power-on with the firmware built from our default settings.
    // TODO(hw-unverified): GBATEK states no reset values for the registers (`spi.md`, "Unverified").
    pub fn new() -> Self {
        Self::with_settings(&FirmwareSettings::default())
    }

    /// The bus at power-on with the firmware built from `settings`.
    pub fn with_settings(settings: &FirmwareSettings) -> Self {
        Self {
            spicnt: 0,
            data: 0,
            done_at: 0,
            irq_due: None,
            firmware: SaveChip::new(SaveType::Flash256K, build(settings)).expect(
                "the builder always makes a FIRMWARE_LEN image, the length of a 256K flash",
            ),
            power: PowerManagement::new(),
            touch: Tsc::new(),
        }
    }

    /// A bus holding `firmware`, whatever its type and size, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn with_firmware_for_test(firmware: SaveChip) -> Self {
        Self {
            firmware,
            ..Self::new()
        }
    }

    /// A bus holding `power` and `touch`, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn with_devices_for_test(power: PowerManagement, touch: Tsc) -> Self {
        Self {
            power,
            touch,
            ..Self::new()
        }
    }

    /// SPICNT as stored: without the busy bit, which depends on the time.
    // TODO(hw-unverified): bits 2-6 and 12-13 read zero although the BIOS writes bits 12-13
    // (`spi.md`, "Unverified").
    pub fn spicnt(&self) -> u16 {
        self.spicnt
    }

    /// SPICNT as a CPU reads it at master time `now`.
    pub fn spicnt_at(&self, now: u64) -> u16 {
        self.spicnt | if self.busy(now) { BUSY } else { 0 }
    }

    /// Whether a unit is still on the wire at master time `now`.
    pub fn busy(&self, now: u64) -> bool {
        now < self.done_at
    }

    /// The master time the unit on the wire is done (0 before any transfer).
    pub fn done_at(&self) -> u64 {
        self.done_at
    }

    /// The device SPICNT selects: 0 power, 1 firmware, 2 touch screen, 3 reserved.
    pub fn device(&self) -> u8 {
        ((self.spicnt >> DEVICE_SHIFT) & DEVICE_MASK) as u8
    }

    /// What SPIDATA holds: the value the last transfer received.
    pub fn data(&self) -> u8 {
        self.data
    }

    /// The firmware flash, for the debugger.
    pub fn firmware(&self) -> &SaveChip {
        &self.firmware
    }

    /// The power-management device, for the debugger.
    pub fn power(&self) -> &PowerManagement {
        &self.power
    }

    /// The touch-screen controller, for the debugger.
    pub fn touch(&self) -> &Tsc {
        &self.touch
    }

    /// Whether the touch controller's pen output is enabled: what EXTKEYIN bit 6 needs besides a
    /// touched panel (`touch-power.md`, "The pen bit"). `Io` hands it to the keypad through `Cx`.
    pub fn pen_irq_enabled(&self) -> bool {
        self.touch.pen_irq_enabled()
    }

    /// One byte exchanged with the device `select` names while the panel is `panel`: what comes
    /// back. Select 3 is unconnected.
    fn exchange(&mut self, select: u16, byte: u8, hold: bool, panel: Option<Touch>) -> u8 {
        match select {
            DEVICE_POWER => self.power.transfer(byte, hold),
            DEVICE_FIRMWARE => self.firmware.transfer(byte, hold),
            DEVICE_TOUCH => self.touch.transfer(byte, hold, panel),
            _ => FLOATING,
        }
    }

    /// The unit written at `now`, if the bus takes it: the selected device answers at once and the
    /// unit takes its time on the wire. Nothing happens with the bus disabled, and a write while
    /// a unit is on the wire is dropped.
    // TODO(hw-unverified): SPIDATA written with the bus disabled or while busy (`spi.md`,
    // "Unverified").
    fn start(&mut self, byte: u8, now: u64, panel: Option<Touch>) -> Effect {
        if self.spicnt & ENABLE == 0 || self.busy(now) {
            return Effect::default();
        }
        // TODO(hw-unverified): a change of the device select inside a held session leaves the first
        // device's session as it was and sends the byte to the new one (`spi.md`, "Unverified").
        let hold = self.spicnt & HOLD != 0;
        let wide = self.spicnt & WIDE != 0;
        let select = self.spicnt >> DEVICE_SHIFT & DEVICE_MASK;
        self.data = if wide {
            // TODO(hw-unverified): the bugged 16-bit mode sends the byte and then a zero, and
            // SPIDATA keeps the second reply, for every device (`spi.md`, "Unverified").
            self.exchange(select, byte, true, panel);
            self.exchange(select, 0, hold, panel)
        } else {
            self.exchange(select, byte, hold, panel)
        };
        let baud = u32::from(self.spicnt & BAUD_MASK) + u32::from(wide);
        self.done_at = now.saturating_add(UNIT_CYCLES << baud);
        // An earlier interrupt whose event has not fired yet is raised now, so a new transfer
        // cannot drop it; the queue's event for slot 0 is replaced either way.
        // TODO(hw-unverified): the late interrupt is raised here, not at the unit's end (`spi.md`,
        // "Unverified").
        let late = self.irq_due.take();
        // TODO(hw-unverified): SPICNT bit 14 as it stands when the unit starts decides the interrupt
        // (`spi.md`, "Unverified").
        self.irq_due = (self.spicnt & IRQ != 0).then_some(self.done_at);
        Effect {
            requests: late.map_or_else(Requests::default, |_| Requests::for_cpu::<false>(SPI_IRQ)),
            reschedule: u32::from(late.is_some() || self.irq_due.is_some()),
            ..Effect::default()
        }
    }

    /// The byte of the register space at `offset`.
    fn byte(&self, offset: u32, now: u64) -> u8 {
        match offset {
            0 | 1 => self.spicnt_at(now).to_le_bytes()[offset as usize],
            SPIDATA_OFFSET => self.data,
            _ => 0,
        }
    }

    /// A write of one byte at `offset`.
    fn write_byte(&mut self, offset: u32, byte: u8, now: u64, panel: Option<Touch>) -> Effect {
        match offset {
            0 => {
                self.spicnt = (self.spicnt & 0xFF00) | u16::from(byte) & (SPICNT_MASK & 0xFF);
                Effect::default()
            }
            1 => {
                self.spicnt = (self.spicnt & 0x00FF) | (u16::from(byte) << 8 & SPICNT_MASK);
                Effect::default()
            }
            SPIDATA_OFFSET => self.start(byte, now, panel),
            _ => Effect::default(),
        }
    }
}

impl Default for Spi {
    fn default() -> Self {
        Self::new()
    }
}

impl Checked for Spi {
    fn well_formed(&self, now: u64) -> bool {
        self.spicnt & !SPICNT_MASK == 0
            && self.firmware.kind() == SaveType::Flash256K
            && self.firmware.data().len() == FIRMWARE_LEN
            && self.firmware.well_formed()
            && self.power.well_formed()
            && self.touch.well_formed()
            && self.done_at <= now.saturating_add(LONGEST_UNIT)
            && self.irq_due.is_none_or(|due| due == self.done_at)
    }
}

impl Device<false> for Spi {
    // TODO(hw-unverified): the ARM9 never reaches this device: its map has no entry here, so it reads
    // 0 and drops writes (`io/map.rs`; `spi.md`, "Unverified").
    fn peek(&self, offset: u32, width: Width, cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.byte(at, cx.now))
    }

    /// A byte write to SPIDATA starts a transfer; a wider write that covers it uses its low byte.
    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte(at, byte, cx.now, cx.host.touch)
        })
    }

    /// The one slot: the completion interrupt.
    const SLOTS: u8 = 1;

    fn slot_due(&self, _slot: u8) -> Option<u64> {
        self.irq_due
    }

    /// The unit is done: IF bit 23 on the ARM7.
    fn slot_fire(&mut self, _slot: u8, _now: u64) -> Fired {
        match self.irq_due.take() {
            Some(_) => Fired {
                effect: Effect::raising(Requests::for_cpu::<false>(SPI_IRQ)),
                ..Fired::default()
            },
            None => Fired::default(),
        }
    }
}

#[cfg(test)]
mod tests;
