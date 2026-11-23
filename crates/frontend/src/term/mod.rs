// SPDX-License-Identifier: Apache-2.0
//! The terminal adapter: raw mode, alternate screen, size queries, and the main loop.
//!
//! This is the only module that touches the OS terminal. It starts the three threads (emu, render
//! and input, #218) and joins them; what each does is in [`crate::threads`], tested with a test
//! backend and no thread. OS-specific pieces stay here so a Windows backend is an addition
//! (AGENTS.md §2).

use crate::app::App;
use crate::gamepad::Gamepads;
use crate::handoff::Handoff;
use crate::kitty;
use crate::layout::TermSize;
use crate::pacing::FramePacer;
use crate::pen::Unit;
use crate::render::{Renderer, Snapshot};
use crate::savefile::SaveFile;
use crate::source::Source;
use crate::threads::{
    HostEvent, INPUT_POLL, StopGuard, forward_event, forward_pads, run_emu_and_save, run_render,
};
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
use std::sync::mpsc::{self, Sender};
use std::thread;
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
    let mut app = App::new(size)
        .with_mouse_unit(unit)
        .with_key_release(found.key_release)
        .with_source(source)
        .with_save(save);
    let mut renderer = Renderer::new(found.sink, found.transfer);
    let mut gfx = io::stdout();
    // Opened here so the model starts knowing whether the library started and how many pads there
    // are; the input thread then owns it.
    let pads = Gamepads::open();
    app = app.with_gamepads(pads.available(), pads.connected());
    // The save file is written by the emu thread, last and even when render or input failed, so a
    // game's save is not lost; a write failure shows once the terminal is restored.
    let result = run_threads(&mut app, &mut renderer, &mut terminal, &mut gfx, pads);
    outcome(result, app.save_note())
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

/// Reads terminal events and the pads and sends them to the emu thread until `stop` is set.
///
/// The poll's timeout is [`INPUT_POLL`]: it bounds how fast the thread sees `stop` and how late a
/// pad event is seen.
fn input_loop(
    pads: &mut Gamepads,
    tx: &Sender<HostEvent>,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    let mut connected = pads.connected();
    while !stop.load(Ordering::SeqCst) {
        if event::poll(INPUT_POLL)? && !forward_event(&event::read()?, size, tx)? {
            break;
        }
        if !forward_pads(pads.poll(), pads.connected(), &mut connected, tx) {
            break;
        }
    }
    Ok(())
}

/// Runs the emu, render and input threads until one of them ends, then joins all three.
///
/// A thread that returns, fails or panics sets `stop` through its [`StopGuard`], so the other two
/// end too; the emu thread still writes the save first. The first failure of render, input and
/// emu, in that order, is the result.
fn run_threads<S: ShmSink + Send>(
    app: &mut App,
    renderer: &mut Renderer<S>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    gfx: &mut io::Stdout,
    mut pads: Gamepads,
) -> anyhow::Result<()> {
    let stop = AtomicBool::new(false);
    let handoff = Handoff::new(Snapshot::default);
    let (tx, rx) = mpsc::channel();
    let mut pacer = FramePacer::new(Instant::now());
    let joined = |handle: thread::ScopedJoinHandle<'_, anyhow::Result<()>>, name: &str| {
        handle
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the {name} thread panicked")))
    };
    thread::scope(|scope| {
        let emu = scope.spawn(|| {
            let _stop = StopGuard::new(&stop, &handoff);
            // A `Receiver` is `Send` but not `Sync`: the emu thread takes it by value.
            let rx = rx;
            run_emu_and_save(
                app,
                &rx,
                &handoff,
                &mut pacer,
                &stop,
                Instant::now,
                thread::sleep,
            )
        });
        let render = scope.spawn(|| {
            let _stop = StopGuard::new(&stop, &handoff);
            run_render(renderer, terminal, gfx, &handoff, &stop)
        });
        let input = scope.spawn(|| {
            let _stop = StopGuard::new(&stop, &handoff);
            input_loop(&mut pads, &tx, &stop)
        });
        let rendered = joined(render, "render");
        let read = joined(input, "input");
        let saved = joined(emu, "emu");
        rendered.and(read).and(saved)
    })
}

#[cfg(all(test, unix))]
mod tests;
