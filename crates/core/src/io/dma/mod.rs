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
use crate::io::device::{
    Checked, Cx, Device, Effect, Fired, SlotMask, Width, read_bytes, write_bytes,
};
use crate::io::irq::{DMA0_IRQ, Requests};
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

/// The requests of one line in the main-memory display: 256 pixels, 8 to a request of 4 words
/// (`display-capture.md`, "The main-memory display").
const MAIN_MEMORY_REQUESTS: u32 = 32;

/// The words a mode 7 channel moves each time the geometry FIFO is less than half full
/// (`geometry-engine.md`, "The FIFO IRQ, the DMA start and the stall").
const GEOMETRY_BURST_WORDS: u32 = 112;
/// Master cycles a DMA word takes in the model: one bus cycle, two master cycles (`scheduler.md`,
/// "Clocks"). Only the pause before a channel's next burst uses it.
const WORD_CYCLES: u64 = 2;

/// A start timing as the model sees it (`dma.md`, "Start timing").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    Immediate,
    VBlank,
    HBlank,
    /// The ARM9's start of display, mode 3 (`display-capture.md`, "ARM9 DMA start modes 3 and 4").
    Display,
    /// The ARM9's main-memory display, mode 4: the display FIFO's requests for a line.
    MainMemory,
    /// The DS cartridge slot, ARM9 mode 5 and ARM7 mode 2: one unit for each word the card makes
    /// ready (`cart-protocol.md`, "DMA start").
    Card,
    /// The ARM9's geometry command FIFO, mode 7: a burst of up to 112 words each time the FIFO is
    /// less than half full (`geometry-engine.md`, "The FIFO IRQ, the DMA start and the stall").
    GeometryFifo,
    /// A start timing no phase implements: the channel is enabled and busy but never starts.
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
    /// The start of line 0, the start of display.
    StartOfDisplay,
    /// The main-memory display wants a line's pixels: raised in the last line of a frame
    /// (`frame_start`, which also ends the wait of a channel enabled during the frame) and in every
    /// visible line but the last, each for the line after it (`display-capture.md`, "The
    /// main-memory display").
    MainMemory { frame_start: bool },
    /// The cartridge slot has a word ready for the CPU that owns the slot (`arm9`): only that
    /// CPU's channels see it (`cart-protocol.md`, "DMA start").
    CardWord { arm9: bool },
    /// The geometry command FIFO is less than half full (the ARM9's mode 7 channels).
    GeometryFifo,
}

impl Trigger {
    fn start(self) -> Start {
        match self {
            Self::VBlank => Start::VBlank,
            Self::HBlank => Start::HBlank,
            Self::StartOfDisplay => Start::Display,
            Self::MainMemory { .. } => Start::MainMemory,
            Self::CardWord { .. } => Start::Card,
            Self::GeometryFifo => Start::GeometryFifo,
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
    // A DMA-start event is queued for this channel. Enable stays set while it is true and
    // while the transfer runs, so a polling CPU waits (`dma.md`, "Enabling").
    pending: bool,
    // The master time the waiting start is due: 0 for "at once" (the caller raises it to the time
    // the start was raised), later for the pause between two geometry bursts. Kept in the channel so
    // a load rebuilds the start event at the same time (`dma.md`, "Savestate").
    due: u64,
    // The channel's transfer is running (between `begin` and `finish`), and its own enable went
    // 0 -> 1 meanwhile. Neither outlives the atomic transfer, so a savestate never holds them.
    #[serde(skip)]
    running: bool,
    #[serde(skip)]
    rearmed: bool,
    // The channel was enabled since the last frame start, so a main-memory display start waits for
    // the next frame ("Transfer starts at next frame", `display-capture.md`).
    // TODO(hw-unverified): a channel enabled mid-frame waits for the last line of that frame, and
    // enabling it again waits again (`display-capture.md`, "Unverified").
    frame_wait: bool,
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

    /// Whether channel `index` has a start waiting to run: its DMA-start event is queued.
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
            (3, true) => Start::Display,
            (4, true) => Start::MainMemory,
            (5, true) | (2, false) => Start::Card,
            (7, true) => Start::GeometryFifo,
            _ => Start::Unimplemented,
        }
    }

    /// Marks every enabled channel that waits for `trigger` and has no start queued yet, and
    /// returns their slot mask. The machine schedules them in ascending channel order, so equal
    /// times fire 0 to 3 (`dma.md`, "Starting").
    // TODO(hw-unverified): channels that start together go in ascending order (`dma.md`,
    // "Unverified").
    pub(crate) fn trigger(&mut self, trigger: Trigger, time: u64) -> SlotMask {
        let frame_start = matches!(trigger, Trigger::MainMemory { frame_start: true });
        let main_memory = matches!(trigger, Trigger::MainMemory { .. });
        self.channels
            .iter_mut()
            .enumerate()
            .fold(0, |mask, (index, channel)| {
                let waiting = channel.control & CONTROL_ENABLE != 0
                    && Self::start_of(channel.control) == trigger.start();
                if waiting && frame_start {
                    channel.frame_wait = false;
                }
                let held = main_memory && channel.frame_wait;
                if waiting && !held && !channel.pending {
                    channel.pending = true;
                    channel.due = time;
                    mask | 1 << index
                } else {
                    mask
                }
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
        channel.due = 0;
        channel.running = true;
        channel.rearmed = false;
        let word = channel.control & CONTROL_WORD != 0;
        // A main-memory display start moves a whole line: `MAIN_MEMORY_REQUESTS` requests of the
        // programmed count in one transfer (`display-capture.md`, "The main-memory display"). A
        // guest count of 0 or the largest one makes a transfer of tens of millions of units inside
        // one event: it stalls, it does not panic.
        // TODO(hw-unverified): without repeat the transfer is still 32 requests and raises one IRQ per
        // line (`display-capture.md`, "Unverified").
        // A cartridge start moves one unit for each word the card makes ready (`dma.md`, "Starting").
        // TODO(hw-unverified): a count above 1 is paced one unit for each word (`dma.md`, "Unverified").
        let count = match Self::start_of(channel.control) {
            Start::MainMemory => channel.count_now.saturating_mul(MAIN_MEMORY_REQUESTS),
            Start::Card => channel.count_now.min(1),
            // A geometry start moves a burst; the channel waits for the FIFO again with the rest.
            Start::GeometryFifo => channel.count_now.min(GEOMETRY_BURST_WORDS),
            _ => channel.count_now,
        };
        // Addresses are aligned down to the unit (`dma.md`, "Transferring").
        let align = !(if word { 3 } else { 1 });
        let destination_field = channel.control >> CONTROL_DEST_SHIFT;
        Some(Transfer {
            arm9: ARM9,
            // The index is below 4 wherever it comes from a channel.
            channel: index as u8,
            source: channel.source_now & align,
            destination: channel.destination_now & align,
            count,
            word,
            source_step: Step::of_source(channel.control >> CONTROL_SOURCE_SHIFT),
            destination_step: Step::of_destination(destination_field),
            destination_reload: destination_field & 0b11 == 3,
            source_mask: Self::source_mask(index),
            destination_mask: Self::destination_mask(index),
            geometry_burst: Self::start_of(channel.control) == Start::GeometryFifo,
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
        // TODO(hw-unverified): a cartridge count above 1 is paced one unit for each word
        // (`dma.md`, "Unverified").
        // A cartridge request moves one unit. With units left the channel keeps its place and
        // waits for the next word; the last unit ends the transfer as any other does.
        if !rearmed && Self::start_of(control) == Start::Card && channel.count_now > 1 {
            channel.count_now -= 1;
            channel.source_now = end.source;
            channel.destination_now = end.destination;
            return Effect::default();
        }
        // A burst into the geometry FIFO leaves the rest of the count for the next burst, which
        // starts when the FIFO is less than half full again, and a repeating channel starts again
        // at each such level.
        // TODO(hw-unverified): repeat on in start mode 7 (`geometry-engine.md`, "Unverified").
        if !rearmed
            && Self::start_of(control) == Start::GeometryFifo
            && channel.count_now > GEOMETRY_BURST_WORDS
        {
            channel.count_now -= GEOMETRY_BURST_WORDS;
            channel.source_now = end.source;
            channel.destination_now = end.destination;
            return Effect::default();
        }
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
            reschedule: 1 << index,
            ..Effect::default()
        };
        match (old & CONTROL_ENABLE != 0, new & CONTROL_ENABLE != 0) {
            (false, true) => {
                channel.rearmed = channel.running;
                channel.source_now = channel.source;
                channel.destination_now = channel.destination;
                channel.count_now = Self::count_of(new, index);
                channel.frame_wait = true;
                // A geometry FIFO start is a level, not queued here: the write asks `Io` to recheck
                // what the devices hold, and the FIFO's "less than half full" starts the channel
                // then (`geometry-engine.md`, "The FIFO IRQ, the DMA start and the stall").
                if Self::start_of(new) == Start::Immediate {
                    channel.pending = true;
                    channel.due = 0;
                    slot
                } else {
                    Effect::default()
                }
            }
            (true, false) if channel.pending => {
                channel.pending = false;
                channel.due = 0;
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
                // A start's due time is a master time a state can hold, and only a waiting start
                // has one.
                && channel.due <= crate::scheduler::MAX_TIME
                && (channel.pending || channel.due == 0)
        }) && (ARM9 || self.fill == [0; CHANNEL_COUNT])
    }
}

impl<const ARM9: bool> Checked for Dma<ARM9> {
    fn well_formed(&self, _now: u64) -> bool {
        self.is_well_formed()
    }
}

/// An independent copy for each CPU (`AGENTS.md` §4). Its registers are bytes; a wider access is
/// split into them, and a read has no side effect.
impl<const ARM9: bool> Device<ARM9> for Dma<ARM9> {
    fn peek(&self, offset: u32, width: Width, _cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.read_byte(at))
    }

    /// A write to a channel's registers asks `Io` to recheck what the devices hold: a channel set
    /// to start on a condition that already holds takes it at once (`dma.md`, "Starting"). The
    /// fill registers do not start anything.
    // TODO(hw-unverified): the request is held while the word is ready (`dma.md`, "Unverified").
    fn write(&mut self, offset: u32, width: Width, value: u32, _cx: Cx<'_>) -> Effect {
        let effect = write_bytes(offset, width, value, |at, byte| self.write_byte(at, byte));
        Effect {
            recheck_held_signals: offset < CHANNELS_BYTES,
            ..effect
        }
    }

    const SLOTS: u8 = CHANNEL_COUNT as u8;

    /// A waiting start is due at once: the caller raises it to the time the start was raised.
    fn slot_due(&self, slot: u8) -> Option<u64> {
        self.channels
            .get(usize::from(slot))
            .filter(|channel| channel.pending)
            .map(|channel| channel.due)
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
    geometry_burst: bool,
}

/// Where a transfer left its two addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct End {
    source: u32,
    destination: u32,
}

impl Transfer {
    /// Whether the transfer is a burst into the geometry FIFO.
    pub(crate) fn is_geometry_burst(&self) -> bool {
        self.geometry_burst
    }

    /// Master cycles the transfer's words take, for the pause before the channel's next burst.
    pub(crate) fn duration(&self) -> u64 {
        u64::from(self.count) * WORD_CYCLES
    }

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
