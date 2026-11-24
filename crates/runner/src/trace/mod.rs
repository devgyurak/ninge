// SPDX-License-Identifier: Apache-2.0
//! `ninge-runner trace`: run a ROM and write the instruction trace of
//! `docs/contributing/trace-oracle.md` ("The trace format").
//!
//! ```text
//! ninge-runner trace <rom> --frames N [--from-frame F] [--cpu arm9|arm7|both] --out <file>
//! ```
//!
//! Frames before `--from-frame` run untraced; the frames from F up to N are traced. Only argument
//! parsing, the sink and the run are logic; the file is opened by [`write_trace_file`], a thin
//! adapter. A trace of a ROM that is not ours holds that ROM's bytes (the opcode column): never
//! commit it, paste it, or hand it to an agent.

use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use ninge_core::trace::{HEADER, TraceCpu, TraceCpus, TraceRecord, TraceSink, Tracer};
use ninge_core::{Input, Machine};

use crate::{RunnerError, load_rom_file};

/// The usage line of the subcommand, with the rule about traces of a user's ROM.
pub const TRACE_USAGE: &str = "usage: ninge-runner trace <rom> --frames N [--from-frame F] [--cpu arm9|arm7|both] --out <file>\n       a trace of a ROM that is not yours to share holds its bytes: never commit or paste it";

/// Parsed `trace` command line.
#[derive(Debug, PartialEq, Eq)]
pub struct TraceArgs {
    rom: PathBuf,
    frames: u64,
    from_frame: u64,
    cpus: TraceCpus,
    out: PathBuf,
}

impl TraceArgs {
    /// The ROM file to load.
    pub fn rom(&self) -> &Path {
        &self.rom
    }

    /// How many frames to run in all.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// The first traced frame, counting from 0.
    pub fn from_frame(&self) -> u64 {
        self.from_frame
    }

    /// Which CPUs are recorded.
    pub fn cpus(&self) -> TraceCpus {
        self.cpus
    }

    /// The trace file.
    pub fn out(&self) -> &Path {
        &self.out
    }
}

fn usage(what: &str) -> RunnerError {
    RunnerError::Usage(format!("{what}\n{TRACE_USAGE}"))
}

fn number(value: Option<OsString>, flag: &str) -> Result<u64, RunnerError> {
    value
        .as_deref()
        .and_then(OsStr::to_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| usage(&format!("{flag} needs a number")))
}

fn cpus(value: Option<OsString>) -> Result<TraceCpus, RunnerError> {
    match value.as_deref().and_then(OsStr::to_str) {
        Some("arm9") => Ok(TraceCpus::Arm9),
        Some("arm7") => Ok(TraceCpus::Arm7),
        Some("both") => Ok(TraceCpus::Both),
        _ => Err(usage("--cpu needs arm9, arm7 or both")),
    }
}

/// Parses the arguments after `trace`.
///
/// # Errors
///
/// [`RunnerError::Usage`] for a missing or repeated flag, a flag that needs a value and has none,
/// a ROM path that starts with `-`, or a `--from-frame` that is not below `--frames`.
pub fn parse_trace_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<TraceArgs, RunnerError> {
    let mut args = args.into_iter();
    let rom = args.next().ok_or_else(|| usage("missing ROM path"))?;
    if rom.as_encoded_bytes().first() == Some(&b'-') {
        return Err(usage(&format!(
            "expected a ROM path, found {}",
            rom.to_string_lossy()
        )));
    }
    let (mut frames, mut from_frame, mut cpu, mut out) = (None, None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next();
        match flag.to_str() {
            Some("--frames") if frames.is_none() => frames = Some(number(value, "--frames")?),
            Some("--from-frame") if from_frame.is_none() => {
                from_frame = Some(number(value, "--from-frame")?);
            }
            Some("--cpu") if cpu.is_none() => cpu = Some(cpus(value)?),
            Some("--out") if out.is_none() => {
                out = Some(PathBuf::from(
                    value.ok_or_else(|| usage("--out needs a path"))?,
                ));
            }
            _ => {
                return Err(usage(&format!(
                    "unexpected argument {}",
                    flag.to_string_lossy()
                )));
            }
        }
    }
    let frames = frames.ok_or_else(|| usage("--frames is required"))?;
    let from_frame = from_frame.unwrap_or(0);
    if from_frame >= frames {
        return Err(usage("--from-frame must be below --frames"));
    }
    Ok(TraceArgs {
        rom: PathBuf::from(rom),
        frames,
        from_frame,
        cpus: cpu.unwrap_or(TraceCpus::Both),
        out: out.ok_or_else(|| usage("--out is required"))?,
    })
}

/// A sink that writes each record as a line. It cannot return an error from
/// [`TraceSink::record`], so it keeps the first one and stops writing; [`WriterSink::finish`]
/// reports it.
#[derive(Debug)]
pub struct WriterSink<W: Write> {
    writer: W,
    error: Option<io::Error>,
    lines: u64,
}

impl<W: Write> WriterSink<W> {
    /// Writes the header to `writer` and returns the sink.
    ///
    /// # Errors
    ///
    /// The IO error of writing the header.
    pub fn new(mut writer: W) -> io::Result<Self> {
        writeln!(writer, "{HEADER}")?;
        Ok(Self {
            writer,
            error: None,
            lines: 0,
        })
    }

    /// Flushes and returns the writer and the number of lines written, or the first error.
    ///
    /// # Errors
    ///
    /// The first IO error a record write or the flush met.
    pub fn finish(mut self) -> io::Result<(W, u64)> {
        match self.error.take() {
            Some(error) => Err(error),
            None => self.writer.flush().map(|()| (self.writer, self.lines)),
        }
    }
}

impl<W: Write> TraceSink for WriterSink<W> {
    fn record(&mut self, record: &TraceRecord) {
        if self.error.is_some() {
            return;
        }
        match writeln!(self.writer, "{record}") {
            Ok(()) => self.lines += 1,
            Err(error) => self.error = Some(error),
        }
    }
}

/// What a traced run wrote.
#[derive(Debug, PartialEq, Eq)]
pub struct TraceSummary {
    /// Frames run, traced or not.
    pub frames: u64,
    /// Lines written, the header and comments excluded.
    pub lines: u64,
    /// Records of the ARM9 and of the ARM7.
    pub steps: [u32; 2],
}

impl std::fmt::Display for TraceSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "frames={} lines={} arm9={} arm7={}",
            self.frames, self.lines, self.steps[0], self.steps[1]
        )
    }
}

/// Runs `rom` for `args.frames()` frames, tracing from `args.from_frame()`, into `writer`.
///
/// # Errors
///
/// [`RunnerError::Rom`] if the machine refuses the ROM, [`RunnerError::Write`] (naming
/// `args.out()`) if writing fails.
pub fn run_trace<W: Write>(
    rom: Vec<u8>,
    args: &TraceArgs,
    writer: W,
) -> Result<(W, TraceSummary), RunnerError> {
    let write_error = |source| RunnerError::Write {
        path: args.out.clone(),
        source,
    };
    let mut machine = Machine::new();
    machine.load_rom(rom)?;
    let sink = WriterSink::new(writer).map_err(write_error)?;
    let mut tracer = Tracer::new(sink, args.cpus);
    (0..args.from_frame).for_each(|_| machine.run_frame(Input::new()));
    (args.from_frame..args.frames)
        .for_each(|_| machine.run_frame_traced(Input::new(), &mut tracer));
    let steps = [tracer.steps(TraceCpu::Arm9), tracer.steps(TraceCpu::Arm7)];
    let (writer, lines) = tracer.into_sink().finish().map_err(write_error)?;
    Ok((
        writer,
        TraceSummary {
            frames: machine.frame_count(),
            lines,
            steps,
        },
    ))
}

/// Loads the ROM, runs the trace and writes it to `args.out()` through a buffer: the thin file
/// adapter around [`run_trace`].
///
/// # Errors
///
/// Any error of [`load_rom_file`] or [`run_trace`], or [`RunnerError::Write`] if the file cannot
/// be created.
pub fn write_trace_file(args: &TraceArgs) -> Result<TraceSummary, RunnerError> {
    let rom = load_rom_file(args.rom())?;
    let file = std::fs::File::create(args.out()).map_err(|source| RunnerError::Write {
        path: args.out.clone(),
        source,
    })?;
    let result = run_trace(rom, args, BufWriter::new(file)).map(|(_, summary)| summary);
    // A run that failed (a refused ROM, a full disk) leaves no half-written trace behind. The
    // removal is best effort: the run's own error is the one worth reporting.
    if result.is_err() {
        let _ = std::fs::remove_file(args.out());
    }
    result
}

#[cfg(test)]
mod tests;
