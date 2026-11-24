// SPDX-License-Identifier: Apache-2.0
//! The `ninge-tracediff` command line, as logic: argument parsing and the three commands, generic
//! over the stream so a test drives them with an in-memory stub. The binary opens the files and
//! the socket (`127.0.0.1` only) and prints [`Report::text`].
//!
//! ```text
//! ninge-tracediff files <ninge.trace> <oracle.trace> [--cpu 9|7]
//! ninge-tracediff live <ninge.trace> --port P --cpu 9|7 [--breakpoint ADDR --from-hit K] [--max N]
//! ninge-tracediff boundaries <ninge.trace> --port P --cpu 9|7 --breakpoint ADDR [--hits K]
//! ```

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{Read, Write};
use std::path::PathBuf;

use ninge_core::trace::{TraceCpu, TraceError, TraceRecord, parse_trace};

use crate::diff::{BoundaryOutcome, Outcome, compare, hit_records, walk_boundaries};
use crate::oracle::{AdapterError, OracleCpu, RegisterLayout};

/// The usage text, with the rule about traces of a user's ROM.
pub const USAGE: &str = "usage: ninge-tracediff files <ninge.trace> <oracle.trace> [--cpu 9|7]
       ninge-tracediff live <ninge.trace> --port P --cpu 9|7 [--breakpoint ADDR --from-hit K] [--max N]
       ninge-tracediff boundaries <ninge.trace> --port P --cpu 9|7 --breakpoint ADDR [--hits K]
ADDR is hex and an ARM-state address (breakpoints are set as 32-bit; Thumb is not supported yet).
A trace made with --from-frame starts after boot while the oracle starts at boot, so `live` and
`boundaries` compare different moments unless the oracle is first brought to the same one.
The oracle must be on 127.0.0.1. A trace of a ROM that is not yours to share holds
that ROM's bytes (the opcode column): run this tool on it only yourself, locally, and never
commit it, paste it into an issue or a PR, or hand it to an agent.";

/// ARM breakpoints are set with kind 4 (a 32-bit instruction). Thumb-state breakpoints (kind 2)
/// are not supported yet: the usage text says so.
const ARM_KIND: u32 = 4;

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Compare two trace files.
    Files {
        /// ninge's trace.
        ninge: PathBuf,
        /// The oracle's trace.
        oracle: PathBuf,
        /// Compare only this CPU; both when `None`.
        cpu: Option<TraceCpu>,
    },
    /// Compare ninge's trace with a live oracle, instruction by instruction.
    Live(LiveArgs),
    /// Compare registers at the hits of a breakpoint.
    Boundaries(LiveArgs),
}

/// The arguments of the two live commands.
#[derive(Debug, PartialEq, Eq)]
pub struct LiveArgs {
    /// ninge's trace.
    pub ninge: PathBuf,
    /// The oracle's port on `127.0.0.1`.
    pub port: u16,
    /// The CPU the port serves.
    pub cpu: TraceCpu,
    /// The breakpoint address.
    pub breakpoint: Option<u32>,
    /// Start the instruction comparison at this hit of the breakpoint (0: from the start).
    pub from_hit: u32,
    /// How many hits `boundaries` compares at most.
    pub hits: u32,
    /// Compare at most this many instructions.
    pub max: Option<u32>,
}

/// Why a command did not finish.
#[derive(Debug)]
pub enum CliError {
    /// The command line is malformed.
    Usage(String),
    /// A trace did not parse; `which` names the file.
    Trace {
        /// The file's role.
        which: &'static str,
        /// What is wrong.
        error: TraceError,
    },
    /// The oracle or the stream failed.
    Oracle(AdapterError),
    /// The trace has no record at the requested hit, or the oracle never got there.
    NoSuchHit(u32),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(why) => write!(f, "{why}\n{USAGE}"),
            Self::Trace { which, error } => write!(f, "the {which} trace: {error}"),
            Self::Oracle(error) => write!(f, "the oracle: {error}"),
            Self::NoSuchHit(hit) => write!(f, "no hit {hit} of the breakpoint on one of the sides"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<AdapterError> for CliError {
    fn from(error: AdapterError) -> Self {
        Self::Oracle(error)
    }
}

/// What a command found: the text to print and whether a divergence was found.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    /// The text for stdout.
    pub text: String,
    /// Whether the two sides differ; the binary exits 1 on it.
    pub diverged: bool,
}

fn usage(why: &str) -> CliError {
    CliError::Usage(why.to_string())
}

fn cpu_of(value: Option<&OsStr>) -> Result<TraceCpu, CliError> {
    match value.and_then(OsStr::to_str) {
        Some("9") => Ok(TraceCpu::Arm9),
        Some("7") => Ok(TraceCpu::Arm7),
        _ => Err(usage("--cpu needs 9 or 7")),
    }
}

fn number(value: Option<&OsStr>, flag: &str) -> Result<u32, CliError> {
    value
        .and_then(OsStr::to_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| usage(&format!("{flag} needs a number")))
}

fn address(value: Option<&OsStr>) -> Result<u32, CliError> {
    value
        .and_then(OsStr::to_str)
        .map(|text| text.trim_start_matches("0x"))
        .and_then(|text| u32::from_str_radix(text, 16).ok())
        .ok_or_else(|| usage("--breakpoint needs a hex address"))
}

/// Parses the arguments after the program name.
///
/// # Errors
///
/// [`CliError::Usage`] for an unknown command, a missing or repeated flag, or a bad value.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, CliError> {
    let mut args = args.into_iter();
    let command = args.next().ok_or_else(|| usage("missing command"))?;
    let first = args.next().ok_or_else(|| usage("missing trace file"))?;
    if first.as_encoded_bytes().first() == Some(&b'-') {
        return Err(usage("expected a trace file, found a flag"));
    }
    match command.to_str() {
        Some("files") => {
            let oracle = args
                .next()
                .ok_or_else(|| usage("missing oracle trace file"))?;
            let mut cpu = None;
            while let Some(flag) = args.next() {
                match flag.to_str() {
                    Some("--cpu") if cpu.is_none() => cpu = Some(cpu_of(args.next().as_deref())?),
                    _ => {
                        return Err(usage(&format!(
                            "unexpected argument {}",
                            flag.to_string_lossy()
                        )));
                    }
                }
            }
            Ok(Command::Files {
                ninge: first.into(),
                oracle: oracle.into(),
                cpu,
            })
        }
        Some(name @ ("live" | "boundaries")) => {
            let live = live_args(first.into(), args)?;
            match name == "live" {
                true => Ok(Command::Live(live)),
                false => match live.breakpoint {
                    Some(_) => Ok(Command::Boundaries(live)),
                    None => Err(usage("boundaries needs --breakpoint")),
                },
            }
        }
        _ => Err(usage(&format!(
            "unknown command {}",
            command.to_string_lossy()
        ))),
    }
}

fn live_args(
    ninge: PathBuf,
    mut args: impl Iterator<Item = OsString>,
) -> Result<LiveArgs, CliError> {
    let (mut port, mut cpu, mut breakpoint, mut from_hit, mut hits, mut max) =
        (None, None, None, None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next();
        let value = value.as_deref();
        match flag.to_str() {
            Some("--port") if port.is_none() => {
                port = Some(
                    value
                        .and_then(OsStr::to_str)
                        .and_then(|text| text.parse::<u16>().ok())
                        .ok_or_else(|| usage("--port needs a port number"))?,
                );
            }
            Some("--cpu") if cpu.is_none() => cpu = Some(cpu_of(value)?),
            Some("--breakpoint") if breakpoint.is_none() => breakpoint = Some(address(value)?),
            Some("--from-hit") if from_hit.is_none() => {
                from_hit = Some(number(value, "--from-hit")?)
            }
            Some("--hits") if hits.is_none() => hits = Some(number(value, "--hits")?),
            Some("--max") if max.is_none() => max = Some(number(value, "--max")?),
            _ => {
                return Err(usage(&format!(
                    "unexpected argument {}",
                    flag.to_string_lossy()
                )));
            }
        }
    }
    let from_hit = from_hit.unwrap_or(0);
    if from_hit > 0 && breakpoint.is_none() {
        return Err(usage("--from-hit needs --breakpoint"));
    }
    Ok(LiveArgs {
        ninge,
        port: port.ok_or_else(|| usage("--port is required"))?,
        cpu: cpu.ok_or_else(|| usage("--cpu is required"))?,
        breakpoint,
        from_hit,
        hits: hits.unwrap_or(u32::MAX),
        max,
    })
}

fn parse_side(text: &str, which: &'static str) -> Result<Vec<TraceRecord>, CliError> {
    parse_trace(text).map_err(|error| CliError::Trace { which, error })
}

/// The records of `cpu`, in order.
fn of_cpu(records: &[TraceRecord], cpu: TraceCpu) -> Vec<TraceRecord> {
    records.iter().filter(|r| r.cpu == cpu).copied().collect()
}

fn label(cpu: TraceCpu) -> &'static str {
    match cpu {
        TraceCpu::Arm9 => "ARM9",
        TraceCpu::Arm7 => "ARM7",
    }
}

/// Renders one CPU's outcome, and whether it diverged.
fn render(cpu: TraceCpu, outcome: &Outcome) -> (String, bool) {
    match outcome {
        Outcome::Match { compared } => (
            format!("{}: {compared} instructions match", label(cpu)),
            false,
        ),
        Outcome::Diverged(divergence) => (divergence.to_string(), true),
        Outcome::OracleEnded { compared } => (
            format!(
                "{}: the oracle ended after {compared} matching instructions",
                label(cpu)
            ),
            true,
        ),
    }
}

/// Compares two trace texts per CPU.
///
/// # Errors
///
/// [`CliError::Trace`] if either text does not parse.
pub fn run_files(
    ninge_text: &str,
    oracle_text: &str,
    cpu: Option<TraceCpu>,
) -> Result<Report, CliError> {
    let ninge = parse_side(ninge_text, "ninge")?;
    let oracle = parse_side(oracle_text, "oracle")?;
    let cpus = match cpu {
        Some(one) => vec![one],
        None => vec![TraceCpu::Arm9, TraceCpu::Arm7],
    };
    let mut lines = Vec::new();
    let mut diverged = false;
    for cpu in cpus {
        let mine = of_cpu(&ninge, cpu);
        if mine.is_empty() {
            lines.push(format!("{}: no ninge records", label(cpu)));
            continue;
        }
        let theirs = of_cpu(&oracle, cpu);
        let outcome = match compare::<std::convert::Infallible>(mine, theirs.into_iter().map(Ok)) {
            Ok(outcome) => outcome,
            Err(never) => match never {},
        };
        let (line, differs) = render(cpu, &outcome);
        lines.push(line);
        diverged |= differs;
    }
    Ok(Report {
        text: lines.join("\n"),
        diverged,
    })
}

/// ninge's records of `args.cpu`, from the `from_hit`-th hit of the breakpoint when asked.
fn ninge_side(text: &str, args: &LiveArgs) -> Result<Vec<TraceRecord>, CliError> {
    let records = of_cpu(&parse_side(text, "ninge")?, args.cpu);
    match (args.breakpoint, args.from_hit) {
        (Some(pc), hit) if hit > 0 => {
            let start = records
                .iter()
                .enumerate()
                .filter(|(_, r)| r.pc() == pc)
                .nth(hit as usize - 1)
                .map(|(index, _)| index)
                .ok_or(CliError::NoSuchHit(hit))?;
            Ok(records[start..].to_vec())
        }
        _ => Ok(records),
    }
}

/// Compares ninge's trace with a live oracle on `stream`, instruction by instruction.
///
/// # Errors
///
/// A [`CliError`] for a bad trace, a hit that does not exist, or an oracle failure.
pub fn run_live<T: Read + Write>(
    stream: T,
    ninge_text: &str,
    args: &LiveArgs,
) -> Result<Report, CliError> {
    let mine = ninge_side(ninge_text, args)?;
    let mut oracle = OracleCpu::connect(stream, args.cpu, RegisterLayout::default());
    if let (Some(pc), hit @ 1..) = (args.breakpoint, args.from_hit)
        && !oracle.run_to_hit(pc, ARM_KIND, hit)?
    {
        return Err(CliError::NoSuchHit(hit));
    }
    let limit = args.max.map_or(usize::MAX, |max| max as usize);
    let outcome = compare(
        mine.into_iter().take(limit),
        std::iter::from_fn(|| oracle.next_record().transpose()),
    )?;
    let (text, diverged) = render(args.cpu, &outcome);
    Ok(Report { text, diverged })
}

/// Walks the hits of a breakpoint and reports the first boundary whose registers differ.
///
/// # Errors
///
/// A [`CliError`] for a bad trace or an oracle failure.
pub fn run_boundaries<T: Read + Write>(
    stream: T,
    ninge_text: &str,
    args: &LiveArgs,
) -> Result<Report, CliError> {
    let pc = args
        .breakpoint
        .ok_or_else(|| usage("boundaries needs --breakpoint"))?;
    let records = of_cpu(&parse_side(ninge_text, "ninge")?, args.cpu);
    let hits = hit_records(&records, pc);
    if hits.is_empty() {
        // A breakpoint ninge never reaches compares nothing: say so instead of "0 agree".
        return Err(CliError::NoSuchHit(1));
    }
    let mut oracle = OracleCpu::connect(stream, args.cpu, RegisterLayout::default());
    let outcome = walk_boundaries(&mut oracle, pc, ARM_KIND, &hits, args.hits)?;
    let cpu = label(args.cpu);
    Ok(match outcome {
        BoundaryOutcome::Agree { hits } => Report {
            text: format!("{cpu}: {hits} boundary hit(s) at {pc:08x} agree"),
            diverged: false,
        },
        BoundaryOutcome::OracleEnded { hit } => Report {
            text: format!("{cpu}: the oracle never reached hit {hit} of {pc:08x}"),
            diverged: true,
        },
        BoundaryOutcome::Differs { hit, field, ninge } => Report {
            text: format!(
                "{cpu}: hit {hit} of {pc:08x} is the first boundary that differs ({field}; ninge step {})\n\
                 reset the oracle, then run `live` with --breakpoint {pc:x} --from-hit {} to find the instruction",
                ninge.step,
                hit - 1
            ),
            diverged: true,
        },
    })
}

#[cfg(test)]
mod tests;
