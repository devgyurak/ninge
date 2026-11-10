// SPDX-License-Identifier: Apache-2.0
//! Tests of `transfer`.

use super::*;

#[test]
fn given_probe_results_when_choosing_transfer_then_only_supported_picks_shm() {
    let got = [
        Transfer::from_probe(Probe::Supported),
        Transfer::from_probe(Probe::Unsupported("EINVAL:nope".to_string())),
        Transfer::from_probe(Probe::NoReply),
    ];
    assert_eq!(
        got,
        [
            Transfer::Shm,
            Transfer::direct("terminal refused t=s: EINVAL:nope"),
            Transfer::direct("no reply to the t=s probe"),
        ]
    );
}

#[test]
fn given_each_transfer_when_labelling_then_the_panel_line_names_medium_and_reason() {
    let got = [
        Transfer::Shm.label(),
        Transfer::Direct { reason: None }.label(),
        Transfer::direct("why").label(),
    ];
    assert_eq!(
        got,
        [
            "transfer: shm (t=s)",
            "transfer: zlib (t=d)",
            "transfer: zlib (t=d)\nwhy",
        ]
    );
}

#[test]
fn given_no_shm_sink_when_putting_then_it_reports_unsupported() {
    let kind = NoShm.put(&[1]).map_err(|e| e.kind());
    assert_eq!(kind, Err(io::ErrorKind::Unsupported));
}
