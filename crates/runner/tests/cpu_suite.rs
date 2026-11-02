// SPDX-License-Identifier: Apache-2.0
//! The fetched ARM7TDMI JSON suite: every file must decode, and its case count must match
//! the header. Needs `./scripts/fetch_cpu_suite.sh` (verify.sh runs it first).

use ninge_runner::cpu_suite::{decode, suite_dir};

/// Suite files that fail to decode or whose case count differs from the header's, as text.
/// An empty or missing directory is itself a failure, not a pass.
fn undecodable_files() -> Vec<String> {
    let mut paths: Vec<_> = std::fs::read_dir(suite_dir())
        .expect("the suite is missing — run ./scripts/fetch_cpu_suite.sh")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|p| p.to_string_lossy().ends_with(".json.bin"))
        .collect();
    paths.sort();
    let mut problems = Vec::new();
    if paths.len() != 45 {
        problems.push(format!("expected 45 suite files, found {}", paths.len()));
    }
    for path in paths {
        let bytes = std::fs::read(&path).expect("suite file is readable");
        let header = u32::from_le_bytes(bytes[4..8].try_into().expect("header holds a count"));
        match decode(&bytes) {
            Ok(cases) if cases.len() == header as usize => {}
            Ok(cases) => problems.push(format!(
                "{}: header says {header}, decoded {}",
                path.display(),
                cases.len()
            )),
            Err(error) => problems.push(format!("{}: {error}", path.display())),
        }
    }
    problems
}

#[test]
fn given_every_fetched_suite_file_when_decoded_then_each_decodes_with_its_header_count() {
    assert_eq!(undecodable_files(), Vec::<String>::new());
}
