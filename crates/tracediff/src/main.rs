// SPDX-License-Identifier: Apache-2.0
//! The `ninge-tracediff` binary: files and a socket in, one report out. All logic is in
//! [`ninge_tracediff::cli`]; this only opens files and `127.0.0.1:<port>`.
//!
//! Exit codes: 0 no divergence, 1 a divergence was found, 2 a usage error, 3 any other error.

use std::net::TcpStream;
use std::path::Path;
use std::process::ExitCode;

use ninge_tracediff::cli::{self, CliError, Command, Report};

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// How long a read or write on the stub may take before the tool gives up: a wedged oracle must
/// not hang it.
const STUB_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn connect(port: u16) -> Result<TcpStream, String> {
    // Only the local machine: the oracle is launched by hand and never reached over a network.
    let stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|error| format!("cannot connect to 127.0.0.1:{port}: {error}"))?;
    stream
        .set_read_timeout(Some(STUB_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(STUB_TIMEOUT)))
        .map_err(|error| format!("cannot set the socket timeouts: {error}"))?;
    Ok(stream)
}

fn execute() -> Result<Report, (u8, String)> {
    let command = cli::parse(std::env::args_os().skip(1)).map_err(failure)?;
    let other = |message: String| (3, message);
    match command {
        Command::Files { ninge, oracle, cpu } => {
            let (ninge, oracle) = (read(&ninge).map_err(other)?, read(&oracle).map_err(other)?);
            cli::run_files(&ninge, &oracle, cpu).map_err(failure)
        }
        Command::Live(args) => {
            let text = read(&args.ninge).map_err(other)?;
            cli::run_live(connect(args.port).map_err(other)?, &text, &args).map_err(failure)
        }
        Command::Boundaries(args) => {
            let text = read(&args.ninge).map_err(other)?;
            cli::run_boundaries(connect(args.port).map_err(other)?, &text, &args).map_err(failure)
        }
    }
}

fn failure(error: CliError) -> (u8, String) {
    match error {
        CliError::Usage(_) => (2, error.to_string()),
        other => (3, other.to_string()),
    }
}

fn main() -> ExitCode {
    match execute() {
        Ok(report) => {
            println!("{}", report.text);
            ExitCode::from(u8::from(report.diverged))
        }
        Err((code, message)) => {
            eprintln!("ninge-tracediff: {message}");
            ExitCode::from(code)
        }
    }
}
