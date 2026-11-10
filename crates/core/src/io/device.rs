// SPDX-License-Identifier: Apache-2.0
//! The interface every register device speaks: the access [`Width`], what an access asks of its
//! caller ([`Effect`], [`Fired`]), the [`Device`] trait and the byte-register helpers.
//!
//! A device whose registers own scheduler events (the timers, the DMA channels) names them through
//! the same interface: a slot is one event of the device, [`Device::slot_kind`] gives its
//! [`EventKind`], [`Device::slot_due`] the time it is next due and [`Device::slot_fire`] what
//! happens when it fires. `Io` replaces the events of the slots an [`Effect`] names and fires them
//! without knowing which device it is.

use crate::io::dma::Transfer;
use crate::io::irq::Requests;
use crate::scheduler::EventKind;

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

/// What a register access asks of its caller: the IF bits to raise, and which of the device's
/// event slots must be replaced. Every device returns this one type, so the caller handles an
/// access the same way whatever it reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Effect {
    /// The IF bits to raise.
    pub(crate) requests: Requests,
    /// Bit n: the device's event slot n changed and its scheduler event must be replaced. The
    /// four timers are slots 0-3 of their device, and so are the four DMA channels.
    pub(crate) reschedule: u8,
}

impl Effect {
    /// An effect that only raises `requests`.
    pub(crate) fn raising(requests: Requests) -> Self {
        Self {
            requests,
            reschedule: 0,
        }
    }

    /// Both effects in one: the requests and the slots to reschedule of each.
    pub(crate) fn merge(self, other: Self) -> Self {
        Self {
            requests: self.requests.merge(other.requests),
            reschedule: self.reschedule | other.reschedule,
        }
    }
}

/// A register device. `offset` is into the device's own register space ([`Port::base`] is
/// offset 0) and `now` is the master time of the access. `ARM9` is the CPU asking: a device that
/// serves one CPU (or two copies of one) is implemented for that CPU's value alone
/// (`AGENTS.md` §4, "Per-CPU devices").
pub(crate) trait Device<const ARM9: bool> {
    /// What a read would return, with no side effect: the debugger's and the fetch's view.
    fn peek(&self, offset: u32, width: Width, now: u64) -> u32;

    /// A CPU's read. A register that a read changes (a FIFO that pops) overrides this; the
    /// others read as they peek.
    fn read(&mut self, offset: u32, width: Width, now: u64) -> (u32, Effect) {
        (self.peek(offset, width, now), Effect::default())
    }

    /// A CPU's write of `width` at `offset`.
    fn write(&mut self, offset: u32, width: Width, value: u32, now: u64) -> Effect;

    /// The scheduler event that carries event slot `slot`, or `None` when the device has no such
    /// slot (a device with no events has none).
    fn slot_kind(&self, _slot: u8) -> Option<EventKind> {
        None
    }

    /// The master time slot `slot`'s event is next due, or `None` when it has none. `Io` never
    /// queues it before the earliest time the access allows ([`At::floor`]), so a time already
    /// past, or 0, means "as soon as allowed".
    fn slot_due(&self, _slot: u8) -> Option<u64> {
        None
    }

    /// Slot `slot`'s event fired at master time `now`. Whatever it changes comes back in the
    /// [`Fired`], the slot's own event included: `Io` does not assume it is replaced.
    fn slot_fire(&mut self, _slot: u8, _now: u64) -> Fired {
        Fired::default()
    }
}

/// What a fired event asks of its caller: an [`Effect`], like a register access, and for DMA the
/// transfer the machine must now move through the owning CPU's bus, which a device cannot reach.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fired {
    /// The IF bits to raise and the slots to reschedule.
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
