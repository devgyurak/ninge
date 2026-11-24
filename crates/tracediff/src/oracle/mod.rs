// SPDX-License-Identifier: Apache-2.0
//! The oracle side: a client for one CPU's GDB stub ([`GdbClient`]) and the adapter that turns its
//! single steps into trace records ([`OracleCpu`]), with BIOS-region execution collapsed
//! (`docs/contributing/trace-oracle.md`, "BIOS collapsing").

mod adapter;
mod client;

pub use adapter::{AdapterError, OracleCpu, RegisterLayout};
pub use client::{GdbClient, Stop};

#[cfg(test)]
pub(crate) mod tests;
