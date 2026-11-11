// SPDX-License-Identifier: Apache-2.0
//! Tests of `shm`.

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
