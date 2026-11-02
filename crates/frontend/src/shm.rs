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
mod tests {
    use super::*;

    /// Pids no real process uses, one per test, so parallel tests never share names.
    const PID_ROUND_TRIP: u32 = 4_000_000_001;
    const PID_DISTINCT: u32 = 4_000_000_002;
    const PID_TAKEN: u32 = 4_000_000_003;
    const PID_RELEASE: u32 = 4_000_000_004;
    const PID_DROP: u32 = 4_000_000_005;
    const PID_EMPTY: u32 = 4_000_000_006;
    const PID_EXHAUSTED: u32 = 4_000_000_007;

    /// Reads `len` bytes of the object back through a mapping, as the terminal does.
    #[expect(unsafe_code, reason = "mmap to read an object back")]
    fn read_back(name: &str, len: usize) -> Vec<u8> {
        let fd = shm::open(name, OFlags::RDONLY, Mode::empty()).expect("object exists");
        // SAFETY: the object is `len` bytes (set by the writer under test) and the mapping
        // is read-only, copied out, and unmapped before returning. As in `copy_in`, a
        // truncation by another process (SIGBUS) is outside the threat model.
        unsafe {
            let map = mmap(
                ptr::null_mut(),
                len,
                ProtFlags::READ,
                MapFlags::SHARED,
                &fd,
                0,
            )
            .expect("map");
            let bytes = std::slice::from_raw_parts(map.cast::<u8>(), len).to_vec();
            munmap(map, len).expect("unmap");
            bytes
        }
    }

    fn exists(name: &str) -> Result<(), Errno> {
        shm::open(name, OFlags::RDONLY, Mode::empty()).map(drop)
    }

    /// A writer for `pid` after removing objects an earlier, killed run may have left
    /// under the names it would use. The pids above belong to no real process (above the
    /// kernel's pid range), and each test has its own, so tests running in parallel never
    /// share a name; two `cargo test` runs at once would, which this does not guard.
    fn writer_for(pid: u32) -> ShmWriter {
        for seq in 0..CREATE_TRIES {
            let _ = shm::unlink(object_name(pid, seq));
        }
        ShmWriter::with_pid(pid)
    }

    fn create_raw(name: &str) {
        let _ = shm::unlink(name);
        shm::open(
            name,
            OFlags::CREATE | OFlags::EXCL | OFlags::RDWR,
            Mode::RUSR | Mode::WUSR,
        )
        .expect("create the squatting object");
    }

    fn create_all(names: &[String]) {
        for name in names {
            create_raw(name);
        }
    }

    fn unlink_all(names: &[String]) {
        for name in names {
            shm::unlink(name).expect("clean up");
        }
    }

    /// Returns the `(pid, seq)` pairs whose name exceeds the limit.
    fn too_long(pairs: &[(u32, u32)]) -> Vec<(u32, u32)> {
        pairs
            .iter()
            .copied()
            .filter(|&(pid, seq)| object_name(pid, seq).len() > MAX_NAME_LEN)
            .collect()
    }

    #[test]
    fn given_extreme_pid_and_counter_when_naming_then_the_name_fits_the_macos_limit() {
        let pairs = [
            (0, 0),
            (u32::MAX, 0),
            (0, u32::MAX),
            (u32::MAX, u32::MAX),
            (4_194_304, 1_000_000),
        ];
        assert_eq!(too_long(&pairs), Vec::<(u32, u32)>::new());
    }

    #[test]
    fn given_pid_and_counter_when_naming_then_the_format_is_ninge_pid_seq() {
        assert_eq!(object_name(42, 7), "/ninge-42-7");
    }

    #[test]
    fn given_bytes_when_putting_then_mapping_the_named_object_reads_them_back() {
        let mut writer = writer_for(PID_ROUND_TRIP);
        let name = writer.put(&[1, 2, 3, 250]).expect("put");
        assert_eq!(read_back(&name, 4), vec![1, 2, 3, 250]);
    }

    #[test]
    fn given_object_read_by_the_terminal_when_unlinked_then_reopening_gives_enoent() {
        let mut writer = writer_for(PID_ROUND_TRIP + 100);
        let name = writer.put(&[9]).expect("put");
        shm::unlink(&name).expect("terminal unlinks");
        assert_eq!(exists(&name), Err(Errno::NOENT));
    }

    #[test]
    fn given_two_puts_when_comparing_names_then_each_frame_gets_a_fresh_object() {
        let mut writer = writer_for(PID_DISTINCT);
        let first = writer.put(&[1]).expect("put");
        let second = writer.put(&[2]).expect("put");
        assert_eq!(
            (first.as_str(), second.as_str()),
            ("/ninge-4000000002-0", "/ninge-4000000002-1")
        );
    }

    #[test]
    fn given_next_name_already_exists_when_putting_then_the_following_name_is_used() {
        let mut writer = writer_for(PID_TAKEN);
        create_raw("/ninge-4000000003-0");
        let name = writer.put(&[1]).expect("put");
        shm::unlink("/ninge-4000000003-0").expect("clean up the squatter");
        assert_eq!(name, "/ninge-4000000003-1");
    }

    #[test]
    fn given_every_candidate_name_taken_when_putting_then_it_fails_with_already_exists() {
        let names: Vec<String> = (0..CREATE_TRIES)
            .map(|seq| object_name(PID_EXHAUSTED, seq))
            .collect();
        let mut writer = writer_for(PID_EXHAUSTED);
        create_all(&names);
        let kind = writer.put(&[1]).map_err(|e| e.kind());
        unlink_all(&names);
        assert_eq!(kind, Err(io::ErrorKind::AlreadyExists));
    }

    #[test]
    fn given_unread_objects_when_releasing_then_they_are_unlinked() {
        let mut writer = writer_for(PID_RELEASE);
        let name = writer.put(&[1]).expect("put");
        writer.release();
        assert_eq!(exists(&name), Err(Errno::NOENT));
    }

    #[test]
    fn given_object_the_terminal_already_unlinked_when_releasing_then_enoent_is_ignored() {
        let mut writer = writer_for(PID_RELEASE + 100);
        let name = writer.put(&[1]).expect("put");
        shm::unlink(&name).expect("terminal unlinks");
        writer.release();
        assert_eq!(exists(&name), Err(Errno::NOENT));
    }

    #[test]
    fn given_unread_objects_when_the_writer_drops_then_they_are_unlinked() {
        let mut writer = writer_for(PID_DROP);
        let name = writer.put(&[1]).expect("put");
        drop(writer);
        assert_eq!(exists(&name), Err(Errno::NOENT));
    }

    #[test]
    fn given_more_puts_than_remembered_when_the_oldest_is_evicted_then_it_is_unlinked_at_once() {
        let mut writer = writer_for(PID_DROP + 100);
        let names: Vec<String> = (0..=REMEMBERED)
            .map(|_| writer.put(&[1]).expect("put"))
            .collect();
        let evicted = exists(&names[0]);
        let kept = exists(&names[1]);
        writer.release();
        assert_eq!((evicted, kept), (Err(Errno::NOENT), Ok(())));
    }

    #[test]
    fn given_empty_data_when_putting_then_it_is_rejected_without_creating_an_object() {
        let mut writer = writer_for(PID_EMPTY);
        let kind = writer.put(&[]).map_err(|e| e.kind());
        assert_eq!(
            (kind, exists("/ninge-4000000006-0")),
            (Err(io::ErrorKind::InvalidInput), Err(Errno::NOENT))
        );
    }

    #[test]
    fn given_default_writer_when_putting_then_the_name_carries_this_process_id() {
        let mut writer = ShmWriter::default();
        let name = writer.put(&[1]).expect("put");
        assert!(name.starts_with(&format!("/ninge-{}-", std::process::id())));
    }
}
