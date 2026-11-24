// SPDX-License-Identifier: Apache-2.0
//! The interface every register device speaks: the access [`Width`], what an access asks of its
//! caller ([`Effect`], [`Fired`]), the [`Device`] trait and the byte-register helpers.
//!
//! A device whose registers own scheduler events (the timers, the DMA channels) names them through
//! the same interface: a slot is one event of the device, `Io` builds its
//! [`EventKind::Slot`] from the device's [`DeviceId`], [`Device::slot_due`] the time it is next
//! due and [`Device::slot_fire`] what happens when it fires. `Io` replaces the events of the
//! slots an [`Effect`] names and fires them without knowing which device it is.

use crate::input::Input;
use crate::io::dma::Transfer;
use crate::io::irq::Requests;

/// The width of one bus access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Width {
    Byte,
    Half,
    Word,
}

impl Width {
    /// The width of an access of `N` bytes. The bus only makes 1, 2 and 4.
    pub(crate) const fn of<const N: usize>() -> Self {
        const { assert!(N == 1 || N == 2 || N == 4) };
        match N {
            1 => Self::Byte,
            2 => Self::Half,
            _ => Self::Word,
        }
    }

    /// Bytes in the access.
    pub(crate) const fn bytes(self) -> u32 {
        match self {
            Self::Byte => 1,
            Self::Half => 2,
            Self::Word => 4,
        }
    }

    /// The bits of a `u32` the access carries.
    pub(crate) const fn mask(self) -> u32 {
        match self {
            Self::Byte => 0xFF,
            Self::Half => 0xFFFF,
            Self::Word => u32::MAX,
        }
    }
}

/// A set of one device's event slots, bit n for slot n. 32 bits: the SPU will want 16 slots, and
/// a `u8` was already the cap for two devices.
pub(crate) type SlotMask = u32;

/// One thing a device tells the others: an edge or a level that another device waits for. A
/// device raises it in its [`Effect`] and never names the device that listens; `Io` holds the one
/// table that says who does (`Io::route`), so a device that sends a signal registers it once
/// (`cart-protocol.md`, "DMA start").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Signal {
    /// A word of the cartridge slot is ready for the ARM9.
    CardWordArm9,
    /// A word of the cartridge slot is ready for the ARM7.
    CardWordArm7,
    /// The geometry command FIFO is less than half full: the ARM9's mode 7 DMA channels take a
    /// burst (`geometry-engine.md`, "The FIFO IRQ, the DMA start and the stall").
    GeometryFifoHalf,
}

impl Signal {
    /// Every signal, in the fixed order `Io` routes a set of them: the bit order of [`Signals`].
    pub(crate) const ALL: [Self; 3] = [
        Self::CardWordArm9,
        Self::CardWordArm7,
        Self::GeometryFifoHalf,
    ];

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of [`Signal`]s, one bit each: plain, `Copy` and allocation-free. It lives only inside an
/// [`Effect`], never in a savestate, so it needs no serde and no format version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Signals(u8);

impl Signals {
    /// The set holding only `signal`.
    pub(crate) const fn of(signal: Signal) -> Self {
        Self(signal.bit())
    }

    /// Whether `signal` is in the set.
    pub(crate) const fn contains(self, signal: Signal) -> bool {
        self.0 & signal.bit() != 0
    }

    /// Both sets in one.
    pub(crate) const fn merge(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// What a register access asks of its caller: the IF bits to raise, which of the device's
/// event slots must be replaced, and what it tells the other devices. Every device returns this
/// one type, so the caller handles an access the same way whatever it reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effect {
    /// The IF bits to raise.
    pub(crate) requests: Requests,
    /// Bit n: the device's event slot n changed and its scheduler event must be replaced. The
    /// four timers are slots 0-3 of their device, and so are the four DMA channels.
    pub(crate) reschedule: SlotMask,
    /// What the device tells the others; `Io` routes it after the IF bits and the slots.
    pub(crate) signals: Signals,
    /// The access may have enabled a device that waits for a signal another holds, so `Io` asks
    /// every device of the CPU that wrote for its [`Device::held_signals`] and routes them: a DMA
    /// write may enable a channel whose start condition already holds (`dma.md`, "Starting").
    pub(crate) recheck_held_signals: bool,
    /// The access may have changed what an IF bit that is a level needs, so `Io` asks every device
    /// of the CPU that wrote for its [`Device::held_requests`] and raises them: a write to the
    /// geometry block or to the interrupt controller may leave the FIFO IRQ's condition true after
    /// an acknowledge (`gx-stub.md`, "GXSTAT and the FIFO IRQ"). Kept apart from the signals so a
    /// write that cannot change a signal does not route it again (an acknowledge would advance a
    /// stalled card DMA channel).
    pub(crate) recheck_held_requests: bool,
    /// Master cycles the access held its CPU: a write to a full geometry FIFO waits for the engine
    /// (`geometry-engine.md`, "The FIFO IRQ, the DMA start and the stall"). The caller adds them
    /// to the access's CPU as part of its cost.
    pub(crate) stall: u64,
}

impl Effect {
    /// An effect that only raises `requests`.
    pub(crate) fn raising(requests: Requests) -> Self {
        Self {
            requests,
            ..Self::default()
        }
    }

    /// Both effects in one: the requests, the slots to reschedule and the signals of each.
    pub(crate) fn merge(self, other: Self) -> Self {
        Self {
            requests: self.requests.merge(other.requests),
            reschedule: self.reschedule | other.reschedule,
            signals: self.signals.merge(other.signals),
            recheck_held_signals: self.recheck_held_signals || other.recheck_held_signals,
            recheck_held_requests: self.recheck_held_requests || other.recheck_held_requests,
            stall: self.stall.saturating_add(other.stall),
        }
    }
}

/// What an access carries besides its address: the master time of the access, the host's data, the
/// cartridge ROM and the pen output of the touch controller.
///
/// `host` is the [`Input`] `Io` latched as the frame began: stored once, in `Io`, and handed to
/// whichever device an access reaches. This is the path a device takes host data in: the keypad
/// reads its buttons here, and the RTC reads the host's time here (`wall_time`), and the touch controller
/// reads the touch point, with no copy of their own and no latch call. `rom` is the image the
/// machine was loaded with, handed over the same way: the cartridge device reads it here and keeps
/// no copy, so the savestate stays free of ROM bytes (`cart-protocol.md`, "Savestate"). `Copy`, and
/// built per access on the I/O slow path only.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cx<'a> {
    /// Master time of the access, as the CPU's own clock has it.
    pub(crate) now: u64,
    /// The host's data, as latched when the frame began.
    pub(crate) host: &'a Input,
    /// The cartridge image, empty before a ROM is loaded.
    pub(crate) rom: &'a [u8],
    /// The signals one device presents to another, read by `Io` when the access began.
    pub(crate) lines: Lines,
}

/// The current state of one device that another device reads: wires between them, as opposed to
/// events (`Signal`). A device never reaches another, so `Io` reads each wire from its source and
/// hands it over here; a new pair adds a field, not a field on [`Cx`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lines {
    /// The touch controller's `/PENIRQ` output is enabled: the keypad's EXTKEYIN bit 6 needs it
    /// (`touch-power.md`, "The pen bit").
    pub(crate) pen_irq_enabled: bool,
    /// POWCNT1 bit 3 powers the geometry engine: with it clear its ports (`0400_0400h`-`0400_06FFh`)
    /// are read-only (`gx-stub.md`, "Power control").
    pub(crate) geometry_powered: bool,
    /// POWCNT1 bit 2 powers the rendering engine: with it clear its registers (`0400_0320h`-
    /// `0400_03BFh`) are read-only.
    pub(crate) rendering_powered: bool,
}

/// The wires at power-on: the touch controller starts in power-down mode 0, so the pen output is
/// enabled (`touch-power.md`, "Unverified").
impl Default for Lines {
    fn default() -> Self {
        Self {
            pen_irq_enabled: true,
            geometry_powered: true,
            rendering_powered: true,
        }
    }
}

/// The context of an access at master time `now` with no host input held and no ROM, for device
/// tests.
#[cfg(test)]
pub(crate) fn cx_at(now: u64) -> Cx<'static> {
    cx_with_rom(now, &[])
}

/// The context of an access at master time `now` with no host input held and `rom` loaded.
#[cfg(test)]
pub(crate) fn cx_with_rom(now: u64, rom: &[u8]) -> Cx<'_> {
    const NOTHING_HELD: Input = Input::new();
    Cx {
        now,
        host: &NOTHING_HELD,
        rom,
        lines: Lines::default(),
    }
}

/// A register device. `offset` is into the device's own register space ([`Port::base`] is
/// offset 0) and `cx` is the access's time and the host's data. `ARM9` is the CPU asking: a device that
/// serves one CPU (or two copies of one) is implemented for that CPU's value alone
/// (`AGENTS.md` §4, "Per-CPU devices").
pub(crate) trait Device<const ARM9: bool> {
    /// What a read would return, with no side effect: the debugger's and the fetch's view.
    fn peek(&self, offset: u32, width: Width, cx: Cx<'_>) -> u32;

    /// A CPU's read. A register that a read changes (a FIFO that pops) overrides this; the
    /// others read as they peek.
    fn read(&mut self, offset: u32, width: Width, cx: Cx<'_>) -> (u32, Effect) {
        (self.peek(offset, width, cx), Effect::default())
    }

    /// A CPU's write of `width` at `offset`.
    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect;

    /// How many event slots the device has, at most [`SlotMask::BITS`]; 0 for a device with no
    /// events. The scheduler's queue capacity is the sum of these over the registered devices
    /// ([`crate::io::EVENT_SLOTS`]).
    const SLOTS: u8 = 0;
    // A device cannot declare more slots than a mask holds; `Io` checks it at compile time.

    /// The master time slot `slot`'s event is next due, or `None` when it has none. `Io` never
    /// queues it before the earliest time the access allows ([`At::floor`]), so a time already
    /// past, or 0, means "as soon as allowed".
    fn slot_due(&self, _slot: u8) -> Option<u64> {
        None
    }

    /// The signals the device holds up as a level right now, for the CPU `ARM9` that just wrote a
    /// register: a condition that stays true until something takes it, as a word waiting in the
    /// cartridge slot. `Io` asks only when an [`Effect`] sets [`Effect::recheck_held_signals`]. A device
    /// that holds none has none.
    fn held_signals(&self) -> Signals {
        Signals::default()
    }

    /// The IF bits the device holds up as a level while its condition is true, for the CPU `ARM9`
    /// that just wrote a register: an acknowledge clears the bit, and the device raises it again
    /// at once while the condition holds. `Io` asks only when an [`Effect`] sets
    /// [`Effect::recheck_held_requests`], and raising a bit that is already set changes nothing. A device
    /// with no level IRQ has none.
    fn held_requests(&self) -> Requests {
        Requests::default()
    }

    /// Slot `slot`'s event fired at master time `now`. Whatever it changes comes back in the
    /// [`Fired`], the slot's own event included: `Io` does not assume it is replaced.
    fn slot_fire(&mut self, _slot: u8, _now: u64) -> Fired {
        Fired::default()
    }
}

/// Whether a device holds only values that exist, for the check a savestate load runs. The
/// default is yes: a device whose registers take every bit pattern has nothing to check.
pub(crate) trait Checked {
    /// Whether the state is one the device can reach, at master time `now`.
    fn well_formed(&self, _now: u64) -> bool {
        true
    }
}

/// What a fired event asks of its caller: an [`Effect`], like a register access, and for DMA the
/// transfer the machine must now move through the owning CPU's bus, which a device cannot reach.
/// The transfer is not a [`Signal`]: it is a move of bytes over a bus the device does not own, so
/// it comes back whole and the machine runs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fired {
    /// The IF bits to raise, the slots to reschedule and the signals to route.
    pub(crate) effect: Effect,
    /// A transfer to run, from a DMA start.
    pub(crate) transfer: Option<Transfer>,
}

/// A read of `width` at `offset` built from a device that holds registers one byte at a time:
/// each byte comes from the register at its own address, so a wide read spans neighbouring
/// registers. The default helper a byte-register device opts into.
// TODO(hw-unverified): a wider read is split into bytes and each comes from the register at its
// own address (`arm9-memory-map.md`, "Unverified").
pub(crate) fn read_bytes(offset: u32, width: Width, byte: impl Fn(u32) -> u8) -> u32 {
    (0..width.bytes()).fold(0, |value, i| {
        value | u32::from(byte(offset.wrapping_add(i))) << (8 * i)
    })
}

/// A write of `width` at `offset` split into bytes, each going to the register at its own
/// address in ascending order, and the effects merged. The counterpart of [`read_bytes`].
// TODO(hw-unverified): a wider write is split into bytes and each goes to the register at its
// own address (`arm9-memory-map.md`, "Unverified").
pub(crate) fn write_bytes(
    offset: u32,
    width: Width,
    value: u32,
    mut byte: impl FnMut(u32, u8) -> Effect,
) -> Effect {
    (0..width.bytes()).fold(Effect::default(), |effect, i| {
        effect.merge(byte(offset.wrapping_add(i), (value >> (8 * i)) as u8))
    })
}
