// SPDX-License-Identifier: Apache-2.0
//! ninge-core: the emulator itself. Pure logic with no IO; inputs in, frames out.
//!
//! # What SemVer covers
//!
//! Before 1.0 a minor release may break any of this, but these are the names a host is meant to
//! build on, and the ones whose change the release notes call out:
//!
//! - the machine and its host data: [`Machine`], [`Input`], [`Buttons`], [`Touch`], [`Screen`],
//!   [`RomError`], [`SCREEN_WIDTH`], [`SCREEN_HEIGHT`] and [`FRAME_CYCLES`];
//! - [`savestate`]: [`Machine::save_state`], [`Machine::load_state`] and
//!   [`savestate::SavestateError`]. The byte layout is versioned by [`savestate::FORMAT_VERSION`]
//!   and a mismatch is refused, never migrated;
//! - what a host needs from [`cart`]: [`cart::Header`], [`cart::parse`], [`cart::HeaderError`]
//!   and the save-file types [`cart::SaveType`], [`cart::SaveError`] and [`cart::resolve_save`].
//!
//! # What is public but unstable until 1.0
//!
//! [`debug`] (the read-only [`Machine::inspect`] view, [`debug::Inspect`] included, and the
//! device types it hands out), [`cpu`] (the interpreter, its registers and the disassemblers) and
//! the rest of [`cart`] (the header's detail types [`cart::Binary`], [`cart::BinarySpec`],
//! [`cart::Field`], [`cart::HEADER_LEN`] and [`cart::crc16`], and [`cart::SaveChip`]) are public
//! so a debugger can name what it reads. Their type names and structure may change in a minor release. Of [`cpu`],
//! the disassemblers and the types [`debug::Inspect`] returns are the part a debugger panel is
//! expected to use; [`cpu::Bus`], [`cpu::Bank`] and the `test-support` setters exist for the
//! runner's CPU-suite harness and are not for a host that drives a [`Machine`].

mod bus;
pub mod cart;
pub mod cpu;
pub mod debug;
mod gpu2d;
mod hle;
pub mod input;
mod io;
mod machine;
pub mod savestate;
mod scheduler;

pub use input::{Buttons, Input, Touch};
pub use machine::{Machine, RomError, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
pub use scheduler::FRAME_CYCLES;
