// SPDX-License-Identifier: Apache-2.0
//! Per-frame cost of the two transfer paths, and the bytes each puts on the pty.
//!
//! AGENTS.md §2 rule 10 asks for a measurement behind `unsafe`: the shared-memory path
//! needs `mmap`, so this compares it with the zlib path it replaces. One "frame" is both
//! screens, as the app sends them.

use criterion::{Criterion, criterion_group, criterion_main};
use ninge::kitty::{rgb24, transmit, transmit_shm};
use ninge::pattern::dummy_frame;
use ninge::shm::ShmWriter;
use ninge::transfer::ShmSink;
use std::hint::black_box;

const COLS: u16 = 60;
const ROWS: u16 = 22;

fn zlib_frame(top: &[u32], bottom: &[u32]) -> usize {
    transmit(1, top, COLS, ROWS).len() + transmit(2, bottom, COLS, ROWS).len()
}

fn shm_frame(writer: &mut ShmWriter, top: &[u32], bottom: &[u32]) -> usize {
    let mut bytes = 0;
    for (id, pixels) in [(1, top), (2, bottom)] {
        let name = writer.put(&rgb24(pixels)).expect("put");
        bytes += transmit_shm(id, &name, COLS, ROWS).len();
        // The terminal unlinks after reading; here nothing reads, so do it ourselves.
        writer.release();
    }
    bytes
}

fn bench(c: &mut Criterion) {
    let top = dummy_frame(30, 0);
    let bottom = dummy_frame(30, 1);
    println!(
        "bytes to the pty per frame: zlib = {}, shm = {}",
        zlib_frame(&top, &bottom),
        shm_frame(&mut ShmWriter::new(), &top, &bottom)
    );
    let mut group = c.benchmark_group("frame_transfer");
    group.bench_function("zlib_t=d", |b| {
        b.iter(|| black_box(zlib_frame(&top, &bottom)));
    });
    let mut writer = ShmWriter::new();
    group.bench_function("shm_t=s", |b| {
        b.iter(|| black_box(shm_frame(&mut writer, &top, &bottom)));
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
