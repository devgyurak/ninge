// SPDX-License-Identifier: Apache-2.0
//! One CPU's four DMA channels (`docs/hw/dma.md`).
//!
//! Each CPU owns one set at the same addresses; they never see each other: the "an independent
//! copy for each CPU" class of `AGENTS.md` §4 ("Per-CPU devices"), two instances of
//! `Dma<const ARM9: bool>`. A channel's start is a scheduler event on its slot, like a timer's
//! overflow. This module cannot reach the scheduler, the interrupt controllers or the bus: a
//! register write, a trigger or an event firing hands back what the machine must do, and the
//! machine moves the data ([`Transfer::run`]) through the owning CPU's bus view when the event
//! fires (`dma.md`, "Model").

use crate::cpu::Bus;
use crate::io::{Device, Effect, Fired, Width, read_bytes, write_bytes};
use crate::irq::{DMA0_IRQ, Requests};
use crate::scheduler::EventKind;
use serde::{Deserialize, Serialize};

/// DMA0SAD, at `0400_00B0h` (`dma.md`, "Registers"). Channel *n* is `12n` bytes above it.
pub const DMA0SAD_ADDR: u32 = 0x0400_00B0;
/// DMA0FILL, at `0400_00E0h`: the ARM9's fill registers (`dma.md`, "Registers").
pub const DMA0FILL_ADDR: u32 = 0x0400_00E0;
/// Channels per CPU (`dma.md`).
pub const CHANNEL_COUNT: usize = 4;
/// Bytes of one channel's registers: source, destination, count and control.
const CHANNEL_BYTES: u32 = 12;
/// Bytes of the four channels' registers.
pub(crate) const CHANNELS_BYTES: u32 = CHANNEL_COUNT as u32 * CHANNEL_BYTES;
/// Bytes of the four fill registers.
pub(crate) const FILL_BYTES: u32 = 4 * CHANNEL_COUNT as u32;
/// The fill registers' offset in the device's register space, which starts at DMA0SAD.
const FILL_OFFSET: u32 = DMA0FILL_ADDR - DMA0SAD_ADDR;

/// DMAnCNT control bits (`dma.md`, "DMAnCNT control bits"). Bit 0-20 are the word count.
const CONTROL_DEST_SHIFT: u32 = 21;
const CONTROL_SOURCE_SHIFT: u32 = 23;
const CONTROL_REPEAT: u32 = 1 << 25;
const CONTROL_WORD: u32 = 1 << 26;
/// The start-timing field: three bits from bit 27 on the ARM9, two from bit 28 on the ARM7.
const CONTROL_TIMING_SHIFT_ARM9: u32 = 27;
const CONTROL_TIMING_SHIFT_ARM7: u32 = 28;
const CONTROL_IRQ: u32 = 1 << 30;
const CONTROL_ENABLE: u32 = 1 << 31;
/// The control bits that exist on each CPU; the others read 0.
// TODO(hw-unverified): the unused bits are dropped on write and read 0 (`dma.md`, "Unverified").
const CONTROL_BITS_ARM9: u32 = 0xFFE0_0000;
const CONTROL_BITS_ARM7: u32 = 0xF7E0_0000;

/// A start timing as the model sees it (`dma.md`, "Start timing").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    Immediate,
    VBlank,
    HBlank,
    /// A start timing no phase implements yet: the channel is enabled and busy but never starts.
    // TODO(phase4): the ARM9's start-of-display and main-memory-display starts.
    // TODO(phase5): the DS cartridge slot start of both CPUs (#37).
    // TODO(phase7): the ARM9's geometry command FIFO start (#48).
    // The GBA slot and the ARM7's wireless start have no phase.
    Unimplemented,
}

/// What starts the channels that wait for it: the two start timings the machine raises
/// (`dma.md`, "Starting").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// The start of VBlank, line 192.
    VBlank,
    /// The start of HBlank in a visible line.
    HBlank,
}

impl Trigger {
    fn start(self) -> Start {
        match self {
            Self::VBlank => Start::VBlank,
            Self::HBlank => Start::HBlank,
        }
    }
}

/// How an address moves after each unit (`dma.md`, "Transferring").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Increment,
    Decrement,
    Fixed,
}

impl Step {
    /// The control field of the source: 3 is prohibited and acts as increment.
    // TODO(hw-unverified): source control 3 acts as increment (`dma.md`, "Unverified").
    fn of_source(field: u32) -> Self {
        match field & 0b11 {
            1 => Self::Decrement,
            2 => Self::Fixed,
            _ => Self::Increment,
        }
    }

    /// The control field of the destination: 3 steps like increment and reloads at a repeat.
    fn of_destination(field: u32) -> Self {
        Self::of_source(field)
    }

    /// `addr` after one unit of `size` bytes, within `mask`.
    fn advance(self, addr: u32, size: u32, mask: u32) -> u32 {
        let next = match self {
            Self::Increment => addr.wrapping_add(size),
            Self::Decrement => addr.wrapping_sub(size),
            Self::Fixed => addr,
        };
        next & mask
    }
}

/// One channel: its registers, the internal copies a transfer works on, and whether a start is
/// waiting to run. Plain data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Channel {
    // DMAnSAD and DMAnDAD as written, masked to the CPU's address width.
    source: u32,
    destination: u32,
    // DMAnCNT: the word count in the low bits and the control above, masked to the bits that exist.
    control: u32,
    // The internal copies, latched when enable goes from 0 to 1 and moved on by each transfer.
    source_now: u32,
    destination_now: u32,
    count_now: u32,
    // A `DmaStart` event is queued for this channel. Enable stays set while it is true and
    // while the transfer runs, so a polling CPU waits (`dma.md`, "Enabling").
    pending: bool,
    // The channel's transfer is running (between `begin` and `finish`), and its own enable went
    // 0 -> 1 meanwhile. Neither outlives the atomic transfer, so a savestate never holds them.
    #[serde(skip)]
    running: bool,
    #[serde(skip)]
    rearmed: bool,
}

/// The four channels of one CPU: the ARM9's when `ARM9` is true, the ARM7's otherwise. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dma<const ARM9: bool> {
    channels: [Channel; CHANNEL_COUNT],
    // DMAnFILL. The ARM7 has none; they stay 0 there.
    fill: [u32; CHANNEL_COUNT],
}

/// Which register of a channel a byte offset falls in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Register {
    Source,
    Destination,
    Control,
    Fill,
}

impl<const ARM9: bool> Dma<ARM9> {
    /// Channels in their power-on state: disabled, everything 0.
    // TODO(hw-unverified): the reset values are not stated (`dma.md`, "Registers").
    pub fn new() -> Self {
        Self {
            channels: [Channel::default(); CHANNEL_COUNT],
            fill: [0; CHANNEL_COUNT],
        }
    }

    /// Channel `index`'s source address register. An `index` past the last channel reads 0.
    pub fn source(&self, index: usize) -> u32 {
        self.channels.get(index).map_or(0, |channel| channel.source)
    }

    /// Channel `index`'s destination address register. An `index` past the last channel reads 0.
    pub fn destination(&self, index: usize) -> u32 {
        self.channels
            .get(index)
            .map_or(0, |channel| channel.destination)
    }

    /// Channel `index`'s DMAnCNT: the word count and the control bits, enable (busy) included.
    /// An `index` past the last channel reads 0.
    pub fn control(&self, index: usize) -> u32 {
        self.channels
            .get(index)
            .map_or(0, |channel| channel.control)
    }

    /// Channel `index`'s fill register; 0 on the ARM7, which has none.
    pub fn fill(&self, index: usize) -> u32 {
        self.fill.get(index).copied().unwrap_or(0)
    }

    /// Whether channel `index` has a start waiting to run: its `DmaStart` event is queued.
    pub fn pending(&self, index: usize) -> bool {
        self.channels
            .get(index)
            .is_some_and(|channel| channel.pending)
    }

    /// The mask of the word count of channel `index`: 21 bits on the ARM9, 14 on ARM7 channels
    /// 0-2 and 16 on ARM7 channel 3 (`dma.md`, "Word count").
    const fn count_mask(index: usize) -> u32 {
        match (ARM9, index) {
            (true, _) => 0x1F_FFFF,
            (false, 3) => 0xFFFF,
            (false, _) => 0x3FFF,
        }
    }

    /// The mask of the source address of channel `index` (`dma.md`, "Address masks").
    // TODO(hw-unverified): the ARM7's widths are the GBA's (`dma.md`, "Unverified").
    const fn source_mask(index: usize) -> u32 {
        match (ARM9, index) {
            (false, 0) => 0x07FF_FFFF,
            _ => 0x0FFF_FFFF,
        }
    }

    /// The mask of the destination address of channel `index`.
    // TODO(hw-unverified): the ARM7's widths are the GBA's (`dma.md`, "Unverified").
    const fn destination_mask(index: usize) -> u32 {
        match (ARM9, index) {
            (false, 0..=2) => 0x07FF_FFFF,
            _ => 0x0FFF_FFFF,
        }
    }

    /// The bits of DMAnCNT that exist on this CPU and channel.
    const fn control_mask(index: usize) -> u32 {
        Self::count_mask(index)
            | if ARM9 {
                CONTROL_BITS_ARM9
            } else {
                CONTROL_BITS_ARM7
            }
    }

    /// The word count `control` asks for: 0 means the field's top plus one.
    const fn count_of(control: u32, index: usize) -> u32 {
        match control & Self::count_mask(index) {
            0 => Self::count_mask(index) + 1,
            count => count,
        }
    }

    /// The start timing `control` selects.
    fn start_of(control: u32) -> Start {
        let (shift, width_mask) = if ARM9 {
            (CONTROL_TIMING_SHIFT_ARM9, 0b111)
        } else {
            (CONTROL_TIMING_SHIFT_ARM7, 0b11)
        };
        match ((control >> shift) & width_mask, ARM9) {
            (0, _) => Start::Immediate,
            (1, _) => Start::VBlank,
            (2, true) => Start::HBlank,
            _ => Start::Unimplemented,
        }
    }

    /// The event that carries channel `index`'s start.
    pub(crate) fn event_kind(index: usize) -> EventKind {
        EventKind::DmaStart {
            arm9: ARM9,
            // The index is below 4 wherever it comes from a channel.
            channel: index as u8,
        }
    }

    /// Marks every enabled channel that waits for `trigger` and has no start queued yet, and
    /// returns their slot mask. The machine schedules them in ascending channel order, so equal
    /// times fire 0 to 3 (`dma.md`, "Starting").
    // TODO(hw-unverified): channels that start together go in ascending order (`dma.md`,
    // "Unverified").
    pub(crate) fn trigger(&mut self, trigger: Trigger) -> u8 {
        self.channels
            .iter_mut()
            .enumerate()
            .filter(|(_, channel)| {
                channel.control & CONTROL_ENABLE != 0
                    && !channel.pending
                    && Self::start_of(channel.control) == trigger.start()
            })
            .fold(0, |mask, (index, channel)| {
                channel.pending = true;
                mask | 1 << index
            })
    }

    /// Channel `index`'s start event fired: the transfer to run, or `None` when the start was
    /// cancelled (the channel is not waiting any more).
    pub(crate) fn begin(&mut self, index: usize) -> Option<Transfer> {
        let channel = self
            .channels
            .get_mut(index)
            .filter(|channel| channel.pending)?;
        channel.pending = false;
        channel.running = true;
        channel.rearmed = false;
        let word = channel.control & CONTROL_WORD != 0;
        // Addresses are aligned down to the unit (`dma.md`, "Transferring").
        let align = !(if word { 3 } else { 1 });
        let destination_field = channel.control >> CONTROL_DEST_SHIFT;
        Some(Transfer {
            arm9: ARM9,
            // The index is below 4 wherever it comes from a channel.
            channel: index as u8,
            source: channel.source_now & align,
            destination: channel.destination_now & align,
            count: channel.count_now,
            word,
            source_step: Step::of_source(channel.control >> CONTROL_SOURCE_SHIFT),
            destination_step: Step::of_destination(destination_field),
            destination_reload: destination_field & 0b11 == 3,
            source_mask: Self::source_mask(index),
            destination_mask: Self::destination_mask(index),
        })
    }

    /// The transfer of `transfer` ended with the addresses in `end` (`dma.md`, "The end of a
    /// transfer"): the IRQ is requested if control bit 30 is set; a repeating channel waits for
    /// its next trigger with the count (and, for destination control 3, the destination) reloaded;
    /// any other channel is disabled.
    pub(crate) fn finish(&mut self, transfer: &Transfer, end: End) -> Effect {
        let index = usize::from(transfer.channel);
        let Some(channel) = self.channels.get_mut(index) else {
            return Effect::default();
        };
        // TODO(hw-unverified): the live control register decides the repeat and the IRQ, so a
        // write to the channel's own registers mid-transfer, or an enable cleared then, changes
        // the outcome (`dma.md`, "Unverified").
        let control = channel.control;
        // A transfer can write its own channel's registers through the bus. If it cleared and set
        // enable again, the channel was re-armed with new latched copies (and a queued start, for
        // Immediate): the repeat and disable rules must not touch it, or they would overwrite the
        // copies or disable a channel with a start queued, whatever its start timing.
        // TODO(hw-unverified): a channel re-armed by its own transfer keeps the new start
        // (`dma.md`, "Unverified").
        let rearmed = channel.rearmed;
        channel.running = false;
        channel.rearmed = false;
        // TODO(hw-unverified): repeat is ignored for an Immediate start (`dma.md`, "Unverified").
        let repeats = control & CONTROL_REPEAT != 0
            && control & CONTROL_ENABLE != 0
            && Self::start_of(control) != Start::Immediate;
        if rearmed {
            // Nothing to change: the new start stands.
        } else if repeats {
            channel.count_now = Self::count_of(control, index);
            channel.source_now = end.source;
            channel.destination_now = if transfer.destination_reload {
                channel.destination
            } else {
                end.destination
            };
        } else {
            channel.control &= !CONTROL_ENABLE;
        }
        let raised = if control & CONTROL_IRQ != 0 {
            // The channel index is below 4, so the shift stays in IF's DMA bits.
            DMA0_IRQ << index
        } else {
            0
        };
        Effect::raising(Requests::for_cpu::<ARM9>(raised))
    }

    /// The register a byte offset falls in, the index of its channel and the bit position of
    /// the byte within the 32-bit register.
    fn register_at(offset: u32) -> Option<(usize, Register, u32)> {
        let shift = (offset % 4) * 8;
        if offset < CHANNELS_BYTES {
            let register = match (offset % CHANNEL_BYTES) / 4 {
                0 => Register::Source,
                1 => Register::Destination,
                _ => Register::Control,
            };
            return Some(((offset / CHANNEL_BYTES) as usize, register, shift));
        }
        // The ARM7 has no fill registers: the block is not on its bus.
        let within = offset
            .checked_sub(FILL_OFFSET)
            .filter(|&d| d < FILL_BYTES)?;
        ARM9.then_some(((within / 4) as usize, Register::Fill, shift))
    }

    /// The byte at `offset` of the register block; 0 where no register holds it.
    fn read_byte(&self, offset: u32) -> u8 {
        let Some((index, register, shift)) = Self::register_at(offset) else {
            return 0;
        };
        // TODO(hw-unverified): the address registers read back what was written, masked
        // (`dma.md`, "Unverified").
        let value = match register {
            Register::Source => self.source(index),
            Register::Destination => self.destination(index),
            Register::Control => self.control(index),
            Register::Fill => self.fill(index),
        };
        (value >> shift) as u8
    }

    /// Writes a byte at `offset`; a byte no register holds is ignored. The write that sets
    /// enable (bit 31, in the top byte of DMAnCNT) latches the internal copies and, for an
    /// Immediate start, queues the start; one that clears it cancels a start that was waiting.
    // TODO(hw-unverified): a byte write acts on its own byte, and the enable edge is seen when
    // the byte holding bit 31 is written (`dma.md`, "Unverified").
    fn write_byte(&mut self, offset: u32, value: u8) -> Effect {
        let Some((index, register, shift)) = Self::register_at(offset) else {
            return Effect::default();
        };
        let merge = |old: u32| (old & !(0xFF << shift)) | (u32::from(value) << shift);
        if register == Register::Fill {
            if let Some(fill) = self.fill.get_mut(index) {
                *fill = merge(*fill);
            }
            return Effect::default();
        }
        let Some(channel) = self.channels.get_mut(index) else {
            return Effect::default();
        };
        match register {
            Register::Source => channel.source = merge(channel.source) & Self::source_mask(index),
            Register::Destination => {
                channel.destination = merge(channel.destination) & Self::destination_mask(index);
            }
            Register::Control => return Self::write_control(channel, index, shift, merge),
            Register::Fill => {}
        }
        Effect::default()
    }

    /// A write to one byte of DMAnCNT; see [`Dma::write_byte`].
    fn write_control(
        channel: &mut Channel,
        index: usize,
        shift: u32,
        merge: impl Fn(u32) -> u32,
    ) -> Effect {
        let old = channel.control;
        let new = merge(old) & Self::control_mask(index);
        channel.control = new;
        // Only the top byte holds enable.
        if shift != 24 {
            return Effect::default();
        }
        let slot = Effect {
            requests: Requests::default(),
            reschedule: 1 << index,
        };
        match (old & CONTROL_ENABLE != 0, new & CONTROL_ENABLE != 0) {
            (false, true) => {
                channel.rearmed = channel.running;
                channel.source_now = channel.source;
                channel.destination_now = channel.destination;
                channel.count_now = Self::count_of(new, index);
                if Self::start_of(new) == Start::Immediate {
                    channel.pending = true;
                    slot
                } else {
                    Effect::default()
                }
            }
            (true, false) if channel.pending => {
                channel.pending = false;
                slot
            }
            _ => Effect::default(),
        }
    }

    /// Whether the decoded channels hold only bits that exist. A savestate that decodes to
    /// anything else is corrupt (`savestate`).
    pub(crate) fn is_well_formed(&self) -> bool {
        self.channels.iter().enumerate().all(|(index, channel)| {
            channel.control & !Self::control_mask(index) == 0
                && channel.source & !Self::source_mask(index) == 0
                && channel.destination & !Self::destination_mask(index) == 0
                && channel.source_now & !Self::source_mask(index) == 0
                && channel.destination_now & !Self::destination_mask(index) == 0
                && channel.count_now <= Self::count_mask(index) + 1
                && (!channel.pending || channel.control & CONTROL_ENABLE != 0)
        }) && (ARM9 || self.fill == [0; CHANNEL_COUNT])
    }
}

/// An independent copy for each CPU (`AGENTS.md` §4). Its registers are bytes; a wider access is
/// split into them, and a read has no side effect.
impl<const ARM9: bool> Device<ARM9> for Dma<ARM9> {
    fn peek(&self, offset: u32, width: Width, _now: u64) -> u32 {
        read_bytes(offset, width, |at| self.read_byte(at))
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, _now: u64) -> Effect {
        write_bytes(offset, width, value, |at, byte| self.write_byte(at, byte))
    }

    fn slot_kind(&self, slot: u8) -> Option<EventKind> {
        (usize::from(slot) < CHANNEL_COUNT).then(|| Self::event_kind(usize::from(slot)))
    }

    /// A waiting start is due at once: the caller raises it to the time the start was raised.
    fn slot_due(&self, slot: u8) -> Option<u64> {
        self.pending(usize::from(slot)).then_some(0)
    }

    fn slot_fire(&mut self, slot: u8, _now: u64) -> Fired {
        Fired {
            effect: Effect::default(),
            transfer: self.begin(usize::from(slot)),
        }
    }
}

impl<const ARM9: bool> Default for Dma<ARM9> {
    fn default() -> Self {
        Self::new()
    }
}

/// One transfer, taken from a channel when its start fires. It carries everything the move needs,
/// so running it borrows nothing of the device: the units go through the CPU's bus, which can
/// reach the DMA registers themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Transfer {
    /// The CPU that owns the channel.
    pub(crate) arm9: bool,
    /// The channel, 0-3.
    pub(crate) channel: u8,
    source: u32,
    destination: u32,
    count: u32,
    word: bool,
    source_step: Step,
    destination_step: Step,
    destination_reload: bool,
    source_mask: u32,
    destination_mask: u32,
}

/// Where a transfer left its two addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct End {
    source: u32,
    destination: u32,
}

impl Transfer {
    /// Moves every unit through `bus`: a read at the source, then a write at the destination,
    /// each address then stepping by the unit's size (`dma.md`, "Transferring"). Nothing in it
    /// allocates, and a guest value only ever picks addresses and counts, so nothing panics.
    pub(crate) fn run<B: Bus>(&self, bus: &mut B) -> End {
        let size = if self.word { 4 } else { 2 };
        let (mut source, mut destination) = (self.source, self.destination);
        for _ in 0..self.count {
            if self.word {
                let unit = bus.read32(source);
                bus.write32(destination, unit);
            } else {
                let unit = bus.read16(source);
                bus.write16(destination, unit);
            }
            source = self.source_step.advance(source, size, self.source_mask);
            destination = self
                .destination_step
                .advance(destination, size, self.destination_mask);
        }
        End {
            source,
            destination,
        }
    }
}

#[cfg(test)]
mod tests;
