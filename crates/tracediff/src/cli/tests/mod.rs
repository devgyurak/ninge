// SPDX-License-Identifier: Apache-2.0
//! Tests of the command line as logic: parsing, the three commands against in-memory traces and
//! the fake GDB stub, and the usage text. Trace values are hand-built from the trace format
//! (`docs/contributing/trace-oracle.md`); the record builders are shared with the diff tests.

mod boundaries;
mod files;
mod live;
mod parse;

use std::ffi::OsString;

use ninge_core::trace::TraceCpu;

use crate::cli::{LiveArgs, Report};

/// The arguments as `OsString`s.
pub(super) fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Live arguments for `cpu` with every option at its default.
pub(super) fn live_args(cpu: TraceCpu) -> LiveArgs {
    LiveArgs {
        ninge: "ninge.trace".into(),
        port: 1,
        cpu,
        breakpoint: None,
        from_hit: 0,
        hits: u32::MAX,
        max: None,
    }
}

/// The report's text and flag as a pair, for one assertion.
pub(super) fn parts(report: Report) -> (String, bool) {
    (report.text, report.diverged)
}
