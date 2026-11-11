// SPDX-License-Identifier: Apache-2.0
//! ninge-core: the emulator itself. Pure logic with no IO; inputs in, frames out.

mod bus;
pub mod cart;
pub mod cpu;
pub mod debug;
mod gpu2d;
mod hle;
mod io;
mod machine;
pub mod savestate;
mod scheduler;

pub use machine::{Machine, RomError, SCREEN_HEIGHT, SCREEN_WIDTH, Screen};
pub use scheduler::FRAME_CYCLES;
