// SPDX-License-Identifier: Apache-2.0
//! Savestates (#19): the whole [`Machine`] as bytes, and back.
//!
//! This is the emulator's own container, not a hardware feature, so it has no `docs/hw/` note;
//! the format is specified here.
//!
//! # Layout
//!
//! ```text
//! offset  size  field
//! 0       8     magic, the ASCII bytes "NINGESTA"
//! 8       4     format version, u32 little-endian
//! 12      8     ROM length in bytes, u64 little-endian
//! 20      8     ROM fingerprint, 64-bit FNV-1a of the ROM bytes, u64 little-endian
//! 28      ..    postcard encoding of the Machine
//! ```
//!
//! The encoding is the decided binary serde format (AGENTS.md §2 rule 5). The machine's body
//! is positional, so it has no field names to survive a change: any change to the saved state
//! changes the layout.
//!
//! # Version policy
//!
//! [`FORMAT_VERSION`] started at 1; version 2 adds engine A's display registers (#20); version 3 adds the ARM7, its
//! clock and the scheduler (#21); version 4 adds shared WRAM and WRAMCNT (#22); version 5 adds both CPUs' interrupt controllers (#23); version 6 adds VCOUNT, the display flags and both CPUs' DISPSTAT (#104); version 7 adds IPCSYNC and both IPC FIFOs (#24). Before
//! 1.0 there is no backward compatibility: a state whose version differs from this build's is refused with
//! [`SavestateError::VersionMismatch`], never half-loaded. Whoever changes the saved state
//! bumps the version; the pinned power-on hash test fails when the layout changes without it.
//!
//! # The ROM is not in the state
//!
//! The ROM is an input, and may be a user's commercial image: a savestate that embedded it
//! would be a copy of it, and would spread through anything that shares savestates. So the
//! state carries only a length and a hash, and loads onto a machine that was handed the same
//! ROM again. The hash is a non-cryptographic guard against loading a state into the wrong
//! game, not against a deliberate collision.
//!
//! # Checked on load
//!
//! Memory buffer sizes, the header/ROM presence rule, both CPUs' banks against their CPSR
//! modes, and the scheduler (`docs/hw/scheduler.md`, "Savestate checks"): no queued event before
//! `now`, `next_seq` above every queued sequence number, exactly one line start and one HBlank start
//! due within a line of `now`, VCOUNT below 263 with the VBlank flag agreeing, DISPSTAT holding only writable bits, IPC FIFO lengths and heads in range (`docs/hw/ipc.md`), and the ARM7 clock at `now` or one master cycle past it (booted machines).
//! Other decoded values (CP15 masked fields, header contents, the zero padding of
//! unowned register banks) are taken as written: they are
//! plain data the accessors handle totally, so a hand-made state can be odd but cannot panic.
//! The only supported load path is [`Machine::load_state`]; deserializing a `Machine` directly
//! skips every check here.
//!
//! # Not saved
//!
//! The frame buffers are derived output: a load leaves both screens black until the next
//! frame is drawn. The two memory page tables are derived from VRAMCNT_A and WRAMCNT and are rebuilt on load.

use crate::Machine;
use crate::scheduler::{ARM7_INSTRUCTION_COST, EventKind, LINE_CYCLES};

/// The first eight bytes of every savestate. Not a trademarked name.
pub const MAGIC: [u8; 8] = *b"NINGESTA";
/// The savestate layout version this build reads and writes (see the module docs).
pub const FORMAT_VERSION: u32 = 7;

/// Bytes before the encoded machine: magic, version, ROM length, ROM fingerprint.
const HEADER_LEN: usize = 8 + 4 + 8 + 8;

/// FNV-1a 64 offset basis and prime, the published constants.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Why a savestate was refused. The machine is untouched in every case.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SavestateError {
    /// The input does not start with the savestate magic number.
    #[error("not a savestate: bad magic number")]
    BadMagic,
    /// The state was written by a build with a different layout version.
    #[error(
        "savestate format version {found} is not supported (this build reads version {expected})"
    )]
    VersionMismatch {
        /// The version in the input.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
    /// The state was taken with a different ROM than the one this machine holds.
    #[error("savestate was taken with a different ROM than the one loaded")]
    RomMismatch,
    /// The input is truncated, has trailing bytes, or its body does not decode.
    #[error("savestate is corrupt")]
    Corrupt,
}

/// 64-bit FNV-1a over `bytes`.
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
}

/// Reads a little-endian `u32` at `offset`, or `None` outside `bytes`.
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
}

/// Reads a little-endian `u64` at `offset`, or `None` outside `bytes`.
fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    bytes
        .get(offset..offset + 8)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_le_bytes)
}

impl Machine {
    /// Whether the queue holds exactly one line start and one HBlank start, each due within one
    /// line of `now` (`scheduler.md`, "Savestate checks"). Anything else would skip a line or
    /// wait on a far-off one. The time limit is checked with the scheduler
    /// (`Scheduler::is_well_formed`), which keeps `now + LINE_CYCLES` from overflowing.
    fn line_events_are_sane(&self) -> bool {
        let now = self.scheduler.now();
        [EventKind::LineStart, EventKind::HBlankStart]
            .into_iter()
            .all(|kind| {
                matches!(
                    self.scheduler.times_of(kind).as_slice(),
                    [time] if now.checked_add(LINE_CYCLES).is_some_and(|end| (now..=end).contains(time))
                )
            })
    }

    /// Serializes the whole machine, ROM excluded, into a savestate.
    ///
    /// Saving is not on the hot path, so it allocates.
    pub fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.rom.len() as u64).to_le_bytes());
        out.extend_from_slice(&fnv1a64(&self.rom).to_le_bytes());
        // Encoding plain data into a `Vec` cannot fail: that is an invariant we own, and a
        // failure is a bug to surface here rather than hand back a header-only state.
        let body = postcard::to_allocvec(self).expect("a Machine is plain data and always encodes");
        out.extend_from_slice(&body);
        out
    }

    /// Replaces this machine's state with a savestate.
    ///
    /// The contract: the machine already holds the same ROM the state was taken with
    /// (`load_rom`), or none if the state was taken without one. Checks run in order: magic,
    /// version, ROM fingerprint, then the body is decoded into a temporary machine and
    /// trailing bytes are refused. Only when everything succeeded is `self` replaced, so a
    /// refused state leaves the machine exactly as it was. The input is untrusted bytes and
    /// never panics the core.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), SavestateError> {
        match bytes.get(..MAGIC.len()) {
            Some(magic) if magic == MAGIC => {}
            Some(_) => return Err(SavestateError::BadMagic),
            None => return Err(SavestateError::Corrupt),
        }
        let found = u32_at(bytes, 8).ok_or(SavestateError::Corrupt)?;
        if found != FORMAT_VERSION {
            return Err(SavestateError::VersionMismatch {
                found,
                expected: FORMAT_VERSION,
            });
        }
        let rom_len = u64_at(bytes, 12).ok_or(SavestateError::Corrupt)?;
        let rom_hash = u64_at(bytes, 20).ok_or(SavestateError::Corrupt)?;
        if rom_len != self.rom.len() as u64 || rom_hash != fnv1a64(&self.rom) {
            return Err(SavestateError::RomMismatch);
        }
        let body = bytes.get(HEADER_LEN..).ok_or(SavestateError::Corrupt)?;
        let (mut loaded, rest) =
            postcard::take_from_bytes::<Machine>(body).map_err(|_| SavestateError::Corrupt)?;
        // A booted program has a ROM and an unbooted machine has none (`load_rom`); a state
        // that says otherwise would run on a header alone, or hold a ROM it never booted. Memory
        // buffers of the wrong size would read as zeros rather than panic, but are not a machine.
        if !rest.is_empty()
            || loaded.header.is_some() != !self.rom.is_empty()
            || !loaded.memory.is_well_formed()
            || !loaded.arm9.is_well_formed()
            || !loaded.arm7.is_well_formed()
            || !loaded.scheduler.is_well_formed()
            || !loaded.line_events_are_sane()
            // A booted machine is always settled: the ARM7 is at `now` or one master cycle
            // past it. One that lags would spend the first round catching up.
            || (loaded.header.is_some() && loaded.arm7_clock < loaded.scheduler.now())
            || loaded.arm7_clock >= loaded.scheduler.now().saturating_add(ARM7_INSTRUCTION_COST)
        {
            return Err(SavestateError::Corrupt);
        }
        loaded.rom = std::mem::take(&mut self.rom);
        loaded.memory.build_page_table();
        loaded.scheduler.reserve();
        *self = loaded;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
