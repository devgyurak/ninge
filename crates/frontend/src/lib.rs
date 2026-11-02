// SPDX-License-Identifier: Apache-2.0
//! ninge: the terminal frontend.
//!
//! One thread, the original vertical layout. Other layout modes are #62, adaptive
//! output rate #63, and the emu/render/input threads #47. Frames go over shared memory
//! when the terminal accepts it and as zlib otherwise ([`transfer`], #6).

pub mod app;
mod diacritics;
pub mod kitty;
pub mod layout;
pub mod pattern;
pub mod placeholder;
#[cfg(unix)]
pub mod shm;
pub mod term;
pub mod transfer;
