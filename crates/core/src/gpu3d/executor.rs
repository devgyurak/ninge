// SPDX-License-Identifier: Apache-2.0
//! How the rasterizer runs its scanlines (`docs/hw/rasterizer.md`, "When the hardware draws";
//! `docs/contributing/device-kinds.md`, "Why `rayon` is on the forbidden list").
//!
//! ninge-core defines the trait and the sequential implementation. A parallel executor lives
//! outside core (in the frontend, decided in #322) and is passed in as a generic parameter, so
//! there is no thread dependency here and no `dyn`. Each line reads only an immutable snapshot and
//! writes only its own slot, so how the lines are scheduled cannot change the result.

/// Runs a function once for each element of a slice of lines, in any order and on any thread.
pub trait ScanlineExecutor {
    /// Calls `draw(y, line)` exactly once for every `line` of `lines`, `y` being its index. The
    /// calls are independent: `draw` shares nothing mutable between lines.
    fn for_each_line<T, F>(&self, lines: &mut [T], draw: F)
    where
        T: Send,
        F: Fn(usize, &mut T) + Sync;
}

/// The executor core has: the lines in order, on the calling thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SequentialExecutor;

impl ScanlineExecutor for SequentialExecutor {
    fn for_each_line<T, F>(&self, lines: &mut [T], draw: F)
    where
        T: Send,
        F: Fn(usize, &mut T) + Sync,
    {
        lines
            .iter_mut()
            .enumerate()
            .for_each(|(y, line)| draw(y, line));
    }
}
