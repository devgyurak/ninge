// SPDX-License-Identifier: Apache-2.0
//! POSIX shared-memory objects that carry frames to the terminal (`t=s`).
//!
//! The only OS-specific frame code, kept here so a Windows backend is an addition
//! (AGENTS.md §2). Measured on macOS (docs/STATUS.md, "Phase 0 checks"): a name is at
//! most 31 bytes including the leading `/`, `write()` on the object's descriptor fails
//! with `ENXIO` so data goes in through `mmap`, and `ftruncate` works once.

use crate::transfer::ShmSink;
use rustix::io::Errno;
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};
use rustix::shm::{self, Mode, OFlags};
use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;
use std::ptr;

/// Longest object name macOS accepts, the leading `/` included.
pub const MAX_NAME_LEN: usize = 31;

/// Names remembered for unlinking at exit. The terminal unlinks each object after it
/// reads it, so only the last few frames can still exist.
const REMEMBERED: usize = 16;

/// How many names are tried when one already exists (a leftover of an earlier process
/// with the same pid).
const CREATE_TRIES: u32 = 8;

/// The object name for process `pid` and counter `seq`: `/ninge-<pid>-<seq>`.
///
/// At most 28 bytes for any `u32` pair, under [`MAX_NAME_LEN`].
pub fn object_name(pid: u32, seq: u32) -> String {
    format!("/ninge-{pid}-{seq}")
}

/// Creates one shared-memory object per frame and unlinks what is left at exit.
pub struct ShmWriter {
    pid: u32,
    seq: u32,
    remembered: VecDeque<String>,
}

impl ShmWriter {
    /// A writer that names its objects after this process.
    pub fn new() -> Self {
        Self::with_pid(std::process::id())
    }

    /// A writer that names its objects after `pid`.
    pub fn with_pid(pid: u32) -> Self {
        Self {
            pid,
            seq: 0,
            remembered: VecDeque::new(),
        }
    }

    /// Unlinks every remembered object. `ENOENT` is the normal answer: the terminal
    /// already unlinked what it read. Any other failure leaves nothing to do at exit.
    pub fn release(&mut self) {
        for name in self.remembered.drain(..) {
            let _ = shm::unlink(&name);
        }
    }

    fn create(&mut self) -> io::Result<(String, OwnedFd)> {
        for _ in 0..CREATE_TRIES {
            let name = object_name(self.pid, self.seq);
            self.seq = self.seq.wrapping_add(1);
            match shm::open(
                &name,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => return Ok((name, fd)),
                Err(Errno::EXIST) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "every shared-memory name tried already exists",
        ))
    }

    fn remember(&mut self, name: String) {
        self.remembered.push_back(name);
        if self.remembered.len() > REMEMBERED
            && let Some(oldest) = self.remembered.pop_front()
        {
            // A frame the terminal never read (a silent `q=2` error) would otherwise
            // outlive us; `ENOENT` means it was read, which is the usual case.
            let _ = shm::unlink(&oldest);
        }
    }
}

impl Default for ShmWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ShmWriter {
    fn drop(&mut self) {
        self.release();
    }
}

impl ShmSink for ShmWriter {
    fn put(&mut self, data: &[u8]) -> io::Result<String> {
        if data.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an empty shared-memory object cannot be mapped",
            ));
        }
        let (name, fd) = self.create()?;
        let file = File::from(fd);
        let filled = file
            .set_len(data.len() as u64)
            .and_then(|()| copy_in(&file, data));
        match filled {
            Ok(()) => {
                self.remember(name.clone());
                Ok(name)
            }
            Err(e) => {
                let _ = shm::unlink(&name);
                Err(e)
            }
        }
    }
}

/// Maps `file` (already `data.len()` bytes long), copies `data` in, and unmaps.
///
/// The object is ours alone at this point: created here with `O_EXCL`, sized to
/// `data.len()`, and its name not yet sent to the terminal. The name is guessable, so a
/// process of the same user could still open and truncate it, and the next access would
/// raise `SIGBUS`; that is outside the threat model, since such a process already holds
/// the user's own rights.
#[expect(
    unsafe_code,
    reason = "mmap and a raw copy: there is no safe API for mapping a file"
)]
fn copy_in(file: &File, data: &[u8]) -> io::Result<()> {
    // SAFETY: `file` is `data.len()` bytes and ours (O_EXCL), so the mapping covers the
    // copy; it is unmapped before returning and no reference escapes. A truncation by
    // another process (SIGBUS) is outside the threat model, see the fn docs.
    unsafe {
        let map = mmap(
            ptr::null_mut(),
            data.len(),
            ProtFlags::READ | ProtFlags::WRITE,
            MapFlags::SHARED,
            file,
            0,
        )?;
        ptr::copy_nonoverlapping(data.as_ptr(), map.cast::<u8>(), data.len());
        munmap(map, data.len())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
