// SPDX-License-Identifier: Apache-2.0
//! The terminal adapter: raw mode, alternate screen, size queries, and the main loop.
//!
//! This is the only module that touches the OS terminal. Everything it drives is
//! tested through [`crate::app`] with a test backend. OS-specific pieces stay here so a
//! Windows backend is an addition (AGENTS.md §2).

use crate::app::{Action, App, map_event, poll_timeout};
use crate::gamepad::Gamepads;
use crate::kitty;
use crate::layout::TermSize;
use crate::pen::Unit;
use crate::render::{Renderer, Snapshot};
use crate::savefile::SaveFile;
use crate::source::Source;
use crate::transfer::{ShmSink, Transfer};
use anyhow::Context as _;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableFocusChange, EnableFocusChange, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, window_size,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
#[cfg(unix)]
use {
    crate::shm::ShmWriter,
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

/// What the keyboard protocol is asked for: escape codes for every key, with press, repeat and
/// release events. Letters send release only with "report all keys as escape codes"; with just
/// "disambiguate" and "event types" a held `x` would never be seen to let go.
const KEYBOARD_FLAGS: KeyboardEnhancementFlags =
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        .union(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        .union(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES);

/// Whether [`KEYBOARD_FLAGS`] are pushed, so `restore` pops them: the panic hook calls it too,
/// and a terminal without the protocol is never sent the pop.
static KEYBOARD_FLAGS_PUSHED: AtomicBool = AtomicBool::new(false);

/// Restores the terminal on drop.
struct Restore;

fn restore() {
    let mut out = io::stdout();
    if KEYBOARD_FLAGS_PUSHED.swap(false, Ordering::SeqCst) {
        let _ = crossterm::execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = out.write_all(kitty::mouse_disable());
    let _ = crossterm::execute!(out, DisableFocusChange, Show, LeaveAlternateScreen);
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

/// What the startup probe found out.
struct Negotiated<W> {
    /// Where shared-memory frames are written.
    sink: W,
    /// How frames go over.
    transfer: Transfer,
    /// Whether the terminal has the kitty keyboard protocol, so key release is reported.
    key_release: bool,
    /// Whether the terminal can report mouse positions in pixels (SGR-Pixels).
    pixels: bool,
}

/// Asks the terminal whether it accepts `t=s` and has the kitty keyboard protocol, and picks the
/// medium and the key handling from its answers.
///
/// Raw mode must be on and the event reader not started. The probe object is unlinked
/// right after the answer: a terminal that refuses never reads it.
#[cfg(unix)]
fn negotiate() -> Negotiated<ShmWriter> {
    let mut writer = ShmWriter::new();
    let mut key_release = false;
    let mut pixels = false;
    let probe = writer
        .put(&[0; kitty::PROBE_BYTES])
        .map_err(|e| format!("shared memory unavailable: {e}"))
        .and_then(|name| {
            let mut out = io::stdout();
            out.write_all(&kitty::probe_query(&name))
                .and_then(|()| out.flush())
                .map_err(|e| format!("probe not sent: {e}"))
        })
        .map(|()| {
            let reply = read_reply(io::stdin(), PROBE_TIMEOUT);
            key_release = kitty::parse_keyboard_reply(&reply);
            pixels = kitty::parse_pixels_reply(&reply);
            kitty::parse_probe_reply(&reply)
        });
    writer.release();
    let transfer = probe.map_or_else(Transfer::direct, Transfer::from_probe);
    Negotiated {
        sink: writer,
        transfer,
        key_release,
        pixels,
    }
}

/// Shared memory is POSIX-only so far (Windows is planned, AGENTS.md §2).
#[cfg(not(unix))]
fn negotiate() -> Negotiated<crate::transfer::NoShm> {
    Negotiated {
        sink: crate::transfer::NoShm,
        transfer: Transfer::direct("shared memory is not available on this platform"),
        key_release: false,
        pixels: false,
    }
}

/// The current terminal size, with the cell pixel size when the terminal reports it.
pub fn size() -> io::Result<TermSize> {
    let w = window_size()?;
    Ok(TermSize::from_window(w.columns, w.rows, w.width, w.height))
}

/// Runs the TUI until the user quits, showing frames from `source`.
pub fn run(source: Source, save: Option<SaveFile>) -> anyhow::Result<Option<String>> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    enable_raw_mode().context("enabling raw mode")?;
    let _restore = Restore;
    let found = negotiate();
    let mut out = io::stdout();
    crossterm::execute!(out, EnterAlternateScreen, Hide, EnableFocusChange)?;
    // After the alternate screen is entered: each screen has its own stack of keyboard flags.
    if found.key_release {
        // Marked first: a push that fails half way must still be popped by `restore`.
        KEYBOARD_FLAGS_PUSHED.store(true, Ordering::SeqCst);
        crossterm::execute!(out, PushKeyboardEnhancementFlags(KEYBOARD_FLAGS))?;
    }
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let size = size()?;
    // Pixel positions only help with a known cell size; without one the cell centers are used.
    let unit = if found.pixels && size.px_known {
        Unit::Pixels
    } else {
        Unit::Cells
    };
    out.write_all(&kitty::mouse_enable(unit == Unit::Pixels))?;
    out.flush()?;
    let mut renderer = Renderer::new(found.sink, found.transfer);
    let mut app = App::new(size)
        .with_mouse_unit(unit)
        .with_key_release(found.key_release)
        .with_source(source)
        .with_save(save);
    let mut gfx = io::stdout();
    let mut pads = Gamepads::open();
    app = app.with_gamepads(pads.available(), pads.connected());
    let looped = main_loop(&mut app, &mut renderer, &mut terminal, &mut gfx, &mut pads);
    // The save file is written last and even when the loop failed, so a game's save is not lost;
    // a write failure shows once the terminal is restored.
    let saved = app.finish().context("writing the save file");
    outcome(looped.and(saved), app.save_note())
}

/// The run's result with the save note: on success the note is handed back to print; on failure it
/// is put in the error, so the user still learns which chip the save file belongs to.
fn outcome(result: anyhow::Result<()>, note: Option<String>) -> anyhow::Result<Option<String>> {
    match (result, note) {
        (Ok(()), note) => Ok(note),
        (Err(error), Some(note)) => Err(error.context(note)),
        (Err(error), None) => Err(error),
    }
}

/// Draws and reads input until the user quits.
fn main_loop(
    app: &mut App,
    renderer: &mut Renderer<impl ShmSink>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    gfx: &mut io::Stdout,
    pads: &mut Gamepads,
) -> anyhow::Result<()> {
    let mut snapshot = Snapshot::default();
    while !app.should_quit() {
        let deadline = Instant::now() + FRAME_TIME;
        // The pads are read once a frame, before the frame runs.
        pads.poll().into_iter().for_each(|e| app.apply_pad(e));
        app.set_pads_connected(pads.connected());
        app.step();
        app.fill(&mut snapshot);
        renderer.present(&snapshot, terminal, gfx)?;
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
mod tests;
