// SPDX-License-Identifier: Apache-2.0
//! The terminal adapter: raw mode, alternate screen, size queries, and the main loop.
//!
//! This is the only module that touches the OS terminal. Everything it drives is
//! tested through [`crate::app`] with a test backend. OS-specific pieces stay here so a
//! Windows backend is an addition (AGENTS.md §2).

use crate::app::{Action, App, frame_step, map_event, poll_timeout};
use crate::kitty;
use crate::layout::TermSize;
use crate::transfer::Transfer;
use anyhow::Context as _;
use crossterm::cursor::{Hide, Show};
use crossterm::event;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, window_size,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Write};
use std::time::{Duration, Instant};
#[cfg(unix)]
use {
    crate::shm::ShmWriter,
    crate::transfer::ShmSink,
    rustix::event::{PollFd, PollFlags, Timespec, poll},
    rustix::io::Errno,
    std::os::fd::AsFd,
};

/// How long to wait for the answer to the `t=s` query. A terminal that understands
/// nothing still answers the DA1 request that follows it, so this is only the bound for
/// one that answers neither. A reply that arrives later would reach the input reader.
const PROBE_TIMEOUT: Duration = Duration::from_millis(1000);

/// Fixed output rate; adaptive rate is #63.
const FRAME_TIME: Duration = Duration::from_micros(16_667);

/// Most input events handled between two frames.
const MAX_EVENTS_PER_FRAME: usize = 64;

/// Restores the terminal on drop.
struct Restore;

fn restore() {
    let mut out = io::stdout();
    let _ = crossterm::execute!(out, Show, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

impl Drop for Restore {
    fn drop(&mut self) {
        restore();
    }
}

/// Reads `fd` until the DA1 answer arrives, `timeout` passes, or the other end closes,
/// and returns the bytes.
///
/// The caller passes stdin and reads the descriptor directly, so the answer never reaches
/// the event reader, which would take its leading ESC for the Esc key.
#[cfg(unix)]
fn read_reply(fd: impl AsFd, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    while !kitty::reply_complete(&buf) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let wait = Timespec {
            tv_sec: left.as_secs().cast_signed(),
            tv_nsec: i64::from(left.subsec_nanos()),
        };
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        match poll(&mut fds, Some(&wait)) {
            Ok(0) => break,
            Err(Errno::INTR) => continue,
            Err(_) => break,
            Ok(_) => {}
        }
        match rustix::io::read(&fd, &mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    buf
}

/// Asks the terminal whether it accepts `t=s` and picks the medium from its answer.
///
/// Raw mode must be on and the event reader not started. The probe object is unlinked
/// right after the answer: a terminal that refuses never reads it.
#[cfg(unix)]
fn negotiate() -> (ShmWriter, Transfer) {
    let mut writer = ShmWriter::new();
    let probe = writer
        .put(&[0; kitty::PROBE_BYTES])
        .map_err(|e| format!("shared memory unavailable: {e}"))
        .and_then(|name| {
            let mut out = io::stdout();
            out.write_all(&kitty::probe_query(&name))
                .and_then(|()| out.flush())
                .map_err(|e| format!("probe not sent: {e}"))
        })
        .map(|()| kitty::parse_probe_reply(&read_reply(io::stdin(), PROBE_TIMEOUT)));
    writer.release();
    let transfer = probe.map_or_else(Transfer::direct, Transfer::from_probe);
    (writer, transfer)
}

/// Shared memory is POSIX-only so far (Windows is planned, AGENTS.md §2).
#[cfg(not(unix))]
fn negotiate() -> (crate::transfer::NoShm, Transfer) {
    (
        crate::transfer::NoShm,
        Transfer::direct("shared memory is not available on this platform"),
    )
}

/// The current terminal size, with the cell pixel size when the terminal reports it.
pub fn size() -> io::Result<TermSize> {
    let w = window_size()?;
    Ok(TermSize::from_window(w.columns, w.rows, w.width, w.height))
}

/// Runs the TUI until the user quits.
pub fn run() -> anyhow::Result<()> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    enable_raw_mode().context("enabling raw mode")?;
    let _restore = Restore;
    let (sink, transfer) = negotiate();
    let mut out = io::stdout();
    crossterm::execute!(out, EnterAlternateScreen, Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::with_transfer(size()?, sink, transfer);
    let mut gfx = io::stdout();
    while !app.should_quit() {
        let deadline = Instant::now() + FRAME_TIME;
        frame_step(&mut app, &mut terminal, &mut gfx)?;
        // Poll at least once per frame, even after an overrun, so quit and resize are
        // always read. The cap keeps a flood of input (key repeat) from starving drawing.
        for _ in 0..MAX_EVENTS_PER_FRAME {
            if !event::poll(poll_timeout(deadline, Instant::now()))? {
                break;
            }
            match map_event(&event::read()?) {
                Action::Resized => app.resize(size()?),
                other => app.apply(other),
            }
        }
    }
    gfx.flush()?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::pipe;

    const OK_AND_DA1: &[u8] = b"\x1b_Gi=31;OK\x1b\\\x1b[?62;22;52c";

    #[test]
    fn given_answer_already_written_when_reading_then_it_returns_before_the_deadline() {
        let (reader, mut writer) = pipe().expect("pipe");
        writer.write_all(OK_AND_DA1).expect("write");
        let started = Instant::now();
        let got = read_reply(reader, Duration::from_secs(30));
        assert_eq!(
            (got, started.elapsed() < Duration::from_secs(10)),
            (OK_AND_DA1.to_vec(), true)
        );
    }

    #[test]
    fn given_no_answer_when_reading_then_it_gives_up_after_the_timeout_with_nothing() {
        let (reader, _writer) = pipe().expect("pipe");
        let got = read_reply(reader, Duration::from_millis(50));
        assert_eq!(got, Vec::<u8>::new());
    }

    #[test]
    fn given_partial_answer_and_closed_writer_when_reading_then_it_returns_what_arrived() {
        let (reader, mut writer) = pipe().expect("pipe");
        writer.write_all(b"\x1b_Gi=31;OK").expect("write");
        drop(writer);
        let got = read_reply(reader, Duration::from_secs(30));
        assert_eq!(got, b"\x1b_Gi=31;OK".to_vec());
    }

    #[test]
    fn given_answer_in_two_writes_when_reading_then_it_waits_for_the_da1() {
        let (reader, mut writer) = pipe().expect("pipe");
        writer.write_all(b"\x1b_Gi=31;OK\x1b\\").expect("write");
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            writer.write_all(b"\x1b[?62c").expect("write");
        });
        let got = read_reply(reader, Duration::from_secs(30));
        late.join().expect("writer thread");
        assert_eq!(got, b"\x1b_Gi=31;OK\x1b\\\x1b[?62c".to_vec());
    }
}
