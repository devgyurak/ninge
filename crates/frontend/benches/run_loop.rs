// SPDX-License-Identifier: Apache-2.0
//! Cost of `Machine::run_frame` on our own `smoke` test ROM, where both CPUs spin.
//!
//! The scheduler (#21) replaced a fixed ARM9 step loop with rounds that also run the ARM7,
//! so the frame now executes more instructions. Criterion reports the time per frame; compare
//! per instruction (`docs/hw/scheduler.md`: 1,120,380 ARM9 instructions per frame,
//! plus 560,190 ARM7 instructions once the ARM7 runs).

use criterion::{Criterion, criterion_group, criterion_main};
use ninge_core::Machine;
use std::hint::black_box;
use std::path::PathBuf;

fn smoke_rom() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds");
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}; run ./scripts/build_testroms.sh first",
            path.display()
        )
    })
}

fn bench(c: &mut Criterion) {
    let mut machine = Machine::new();
    machine
        .load_rom(smoke_rom())
        .expect("smoke ROM has a valid header");
    let mut group = c.benchmark_group("run_loop");
    group.bench_function("run_frame_smoke", |b| {
        b.iter(|| {
            machine.run_frame();
            black_box(machine.frame_count())
        });
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
