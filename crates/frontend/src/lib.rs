// SPDX-License-Identifier: Apache-2.0
//! ninge: the terminal frontend.
//!
//! One thread, the original vertical layout. Other layout modes are #62, adaptive
//! output rate #63, shared-memory transfer #6, and the emu/render/input threads #47.

pub mod app;
mod diacritics;
pub mod kitty;
pub mod layout;
pub mod pattern;
pub mod placeholder;
pub mod term;
