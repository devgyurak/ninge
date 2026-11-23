// SPDX-License-Identifier: Apache-2.0
//! ninge: the terminal frontend.
//!
//! `ninge [<rom>]` runs a ROM and shows both of the machine's screens; without a ROM it
//! shows a dummy pattern that exercises the output path.
//!
//! The original vertical layout; other layout modes are #62 and adaptive output rate #63. Three
//! threads (#218): emu runs the machine, render writes the frames and draws, input reads the
//! terminal and the pads ([`threads`]). Frames pass from emu to render through a triple buffer
//! ([`handoff`]), and the emu thread's rate comes from a [`pacing::Pacer`], which audio pacing
//! (#47) will replace. Frames go over shared memory when the terminal accepts it and as zlib
//! otherwise ([`transfer`], #6).

pub mod app;
pub mod cli;
pub mod clock;
pub mod cursor;
mod diacritics;
pub mod gamepad;
pub mod handoff;
pub mod input;
pub mod kitty;
pub mod layout;
pub mod pacing;
pub mod pattern;
pub mod pen;
pub mod placeholder;
pub mod render;
pub mod savefile;
#[cfg(unix)]
pub mod shm;
pub mod source;
pub mod term;
pub mod threads;
pub mod transfer;
