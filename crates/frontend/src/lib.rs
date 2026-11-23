// SPDX-License-Identifier: Apache-2.0
//! ninge: the terminal frontend.
//!
//! `ninge [<rom>]` runs a ROM and shows both of the machine's screens; without a ROM it
//! shows a dummy pattern that exercises the output path.
//!
//! One thread, the original vertical layout. Other layout modes are #62, adaptive output
//! rate #63, and the emu/render/input threads #47. Frames go over shared memory when the
//! terminal accepts it and as zlib otherwise ([`transfer`], #6).

pub mod app;
pub mod cli;
pub mod clock;
pub mod cursor;
mod diacritics;
pub mod gamepad;
pub mod input;
pub mod kitty;
pub mod layout;
pub mod pattern;
pub mod pen;
pub mod placeholder;
pub mod savefile;
#[cfg(unix)]
pub mod shm;
pub mod source;
pub mod term;
pub mod transfer;
