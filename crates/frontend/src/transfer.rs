// SPDX-License-Identifier: Apache-2.0
//! How frame pixels reach the terminal: shared memory when the terminal accepts it,
//! zlib over the wire otherwise.
//!
//! The choice is made once at startup from a `t=s` query, never from the terminal's name
//! (docs/STATUS.md, "Frame transfer"). A write to shared memory that fails while running
//! switches to the zlib path for good.

use crate::kitty::Probe;
use std::io;

/// The medium frames go over, and why it is not shared memory when it is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transfer {
    /// Shared memory (`t=s`).
    Shm,
    /// Direct zlib (`t=d,o=z`); `reason` says why shared memory is not in use.
    Direct {
        /// Why shared memory was not chosen or was given up; `None` when never wanted.
        reason: Option<String>,
    },
}

impl Transfer {
    /// Direct transfer for the stated `reason`.
    pub fn direct(reason: impl Into<String>) -> Self {
        Self::Direct {
            reason: Some(reason.into()),
        }
    }

    /// The choice that follows the terminal's answer to the support query.
    pub fn from_probe(probe: Probe) -> Self {
        match probe {
            Probe::Supported => Self::Shm,
            Probe::Unsupported(message) => Self::direct(format!("terminal refused t=s: {message}")),
            Probe::NoReply => Self::direct("no reply to the t=s probe"),
        }
    }

    /// The status panel text: the medium on the first line, the reason (if any) below,
    /// because the panel is narrow.
    pub fn label(&self) -> String {
        match self {
            Self::Shm => "transfer: shm (t=s)".to_string(),
            Self::Direct { reason: None } => "transfer: zlib (t=d)".to_string(),
            Self::Direct {
                reason: Some(reason),
            } => format!("transfer: zlib (t=d)\n{reason}"),
        }
    }
}

/// Somewhere to put a frame's bytes so the terminal can map them.
pub trait ShmSink {
    /// Copies `data` into a fresh shared-memory object and returns its name.
    fn put(&mut self, data: &[u8]) -> io::Result<String>;
}

/// A sink for builds and tests that never use shared memory.
pub struct NoShm;

impl ShmSink for NoShm {
    fn put(&mut self, _data: &[u8]) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "shared memory is not available",
        ))
    }
}

#[cfg(test)]
mod tests {
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
}
