// SPDX-License-Identifier: Apache-2.0
//! The sequential executor: every line is drawn once, with its own index.

use super::*;

#[test]
fn given_192_lines_when_each_is_drawn_then_every_line_holds_its_own_index() {
    let mut lines = vec![usize::MAX; 192];
    SequentialExecutor.for_each_line(&mut lines, |y, line| *line = y);
    assert_eq!(lines, (0..192).collect::<Vec<_>>());
}

#[test]
fn given_no_lines_when_the_executor_runs_then_nothing_is_drawn() {
    let mut lines: Vec<usize> = Vec::new();
    SequentialExecutor.for_each_line(&mut lines, |_, line| *line = 1);
    assert_eq!(lines, Vec::<usize>::new());
}

#[test]
fn given_a_line_that_sums_its_neighbours_snapshot_when_run_twice_then_the_results_match() {
    let snapshot: Vec<u32> = (0..64).collect();
    let run = || {
        let mut lines = vec![0u32; 64];
        SequentialExecutor.for_each_line(&mut lines, |y, line| {
            *line = snapshot[y] + snapshot[(y + 1) % 64];
        });
        lines
    };
    assert_eq!(run(), run());
}
