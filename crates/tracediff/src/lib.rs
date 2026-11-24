// SPDX-License-Identifier: Apache-2.0
//! ninge-tracediff: trace comparison against a reference oracle
//! (`docs/contributing/trace-oracle.md`).
//!
//! - [`rsp`]: the GDB remote serial protocol's packets, generic over `Read + Write`.
//! - [`oracle`]: a client for one CPU's stub and the adapter that turns its steps into
//!   [`ninge_core::trace::TraceRecord`]s, with BIOS-region execution collapsed.
//!
//! Sockets are opened only by the binary; everything here is tested against in-memory stubs.

pub mod oracle;
pub mod rsp;
