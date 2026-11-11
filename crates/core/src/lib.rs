// SPDX-License-Identifier: Apache-2.0
//! ninge-core: the emulator itself. Pure logic with no IO; inputs in, frames out.

pub mod bus;
pub mod cart;
pub mod cpu;
pub mod debug;
pub mod gpu2d;
mod hle;
pub mod io;
mod machine;
pub mod savestate;
pub mod scheduler;

pub use machine::{Machine, RomError, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
