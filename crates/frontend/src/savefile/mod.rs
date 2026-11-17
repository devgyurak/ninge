// SPDX-License-Identifier: Apache-2.0
//! The save chip's file: where it is, when to write it, and the thin file IO around that. Note:
//! `docs/hw/auxspi-saves.md` ("Save files").
//!
//! The file is the chip's memory as raw bytes. The core says when the memory changed
//! (`Machine::save_generation`); [`SaveFlusher`] decides when that is worth a write, so a game that
//! saves in a burst costs one write and not one per byte; the IO is two small functions.

use ninge_core::Machine;
use std::io;
use std::path::{Path, PathBuf};

/// Frames the save memory must hold still before the file is written: a second at 60 Hz. A game
/// saves in a burst of page writes, and the file follows once the burst is over.
pub const QUIET_FRAMES: u32 = 60;

/// When to write the save file, from the change counter of the save memory. Pure, so the policy is
/// tested without a file or a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveFlusher {
    /// The counter at the last write (or at the start).
    flushed: u64,
    /// The counter at the last tick.
    seen: u64,
    /// Ticks the counter has held still since it last moved.
    quiet: u32,
}

impl SaveFlusher {
    /// A flusher for a file that holds the memory at `generation`.
    pub fn new(generation: u64) -> Self {
        Self {
            flushed: generation,
            seen: generation,
            quiet: 0,
        }
    }

    /// One frame went by with the counter at `generation`: whether to write the file now.
    pub fn tick(&mut self, generation: u64) -> bool {
        if generation == self.flushed || generation != self.seen {
            self.seen = generation;
            self.quiet = 0;
            return false;
        }
        self.quiet += 1;
        self.quiet >= QUIET_FRAMES
    }

    /// Whether the memory differs from what the file holds (the exit check: write whatever is left).
    pub fn pending(&self, generation: u64) -> bool {
        generation != self.flushed
    }

    /// The file was written with the memory at `generation`.
    pub fn wrote(&mut self, generation: u64) {
        self.flushed = generation;
        self.seen = generation;
        self.quiet = 0;
    }

    /// A write failed: try again after another quiet spell, not on every frame.
    pub fn failed(&mut self) {
        self.quiet = 0;
    }
}

/// A save file the running machine writes back to.
#[derive(Clone, Debug)]
pub struct SaveFile {
    path: PathBuf,
    flusher: SaveFlusher,
}

impl SaveFile {
    /// The file at `path`, which holds the memory as `machine` has it now.
    pub fn new(path: PathBuf, machine: &Machine) -> Self {
        Self {
            path,
            flusher: SaveFlusher::new(machine.save_generation()),
        }
    }

    /// The path of the file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Called after each frame: writes the file once the memory has been still for
    /// [`QUIET_FRAMES`]. A failed write is tried again later; the exit flush reports it.
    pub fn after_frame(&mut self, machine: &Machine) {
        if self.flusher.tick(machine.save_generation()) {
            // The error is not lost: the memory stays pending, so `finish` tries again and
            // returns it.
            let _ = self.write(machine);
        }
    }

    /// Called at exit: writes whatever is not in the file yet.
    pub fn finish(&mut self, machine: &Machine) -> io::Result<()> {
        if self.flusher.pending(machine.save_generation()) {
            self.write(machine)?;
        }
        Ok(())
    }

    fn write(&mut self, machine: &Machine) -> io::Result<()> {
        let generation = machine.save_generation();
        match write_atomic(&self.path, machine.save_data()) {
            Ok(()) => {
                self.flusher.wrote(generation);
                Ok(())
            }
            Err(error) => {
                self.flusher.failed();
                Err(error)
            }
        }
    }
}

/// The save file beside a ROM: its path with the extension `.sav`.
pub fn default_path(rom: &Path) -> PathBuf {
    rom.with_extension("sav")
}

/// Reads a save file; a file that is not there is `None`, not an error.
pub fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes `bytes` to `<path>.tmp` and renames it over `path`, so a crash leaves the old file or the
/// new one, never half of one. A failed write removes its temp file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)
        .and_then(|()| std::fs::rename(&temp, path))
        .inspect_err(|_| {
            // Best effort: nothing more can be done about a second failure.
            let _ = std::fs::remove_file(&temp);
        })
}

#[cfg(test)]
mod tests;
