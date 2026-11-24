// SPDX-License-Identifier: Apache-2.0
//! The terminal adapter: raw mode, alternate screen, size queries, and the main loop.
//!
//! This is the only module that touches the OS terminal. It starts the three threads (emu, render
//! and input, #218) and joins them; what each does is in [`crate::threads`], tested with a test
//! backend and no thread. OS-specific pieces stay here so a Windows backend is an addition
//! (AGENTS.md §2).
//!
//! # A terminal that goes away
//!
//! A terminal that closes normally sends a hangup signal and the process dies with it. When the
//! signal is ignored (`nohup`) or never sent, the terminal is simply gone: writes fail and reads
//! return nothing. The input loop asks for that before it reads an event (`hung_up`,
//! [`crate::threads::run_input`]) and ends the run, and the run does not depend on the input
//! thread ending (`run_threads`).

use crate::app::App;
use crate::audio::device::{Device, use_audio};
use crate::audio::paced;
use crate::gamepad::Gamepads;
use crate::handoff::Handoff;
use crate::kitty;
use crate::layout::TermSize;
use crate::pen::Unit;
use crate::render::{Renderer, Snapshot};
use crate::savefile::SaveFile;
use crate::source::Source;
use crate::threads::{
    INPUT_GRACE, Pace, StopGuard, TerminalInput, finishes_within, run_emu_and_save, run_input,
    run_render,
};
use crate::transfer::{ShmSink, Transfer};
use anyhow::Context as _;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableFocusChange, EnableFocusChange, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode, window_size,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
#[cfg(unix)]
use {
    crate::shm::ShmWriter,
    rustix::event::{PollFd, PollFlags, Timespec, poll},
    rustix::io::Errno,
    std::io::IsTerminal as _,
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

/// Runs the TUI until the user quits, showing frames from `source`. With `audio` and a ROM it opens
/// the default sound device and keeps time by it; with no device, or without either, it keeps
/// time by the machine's frame period.
pub fn run(source: Source, save: Option<SaveFile>, audio: bool) -> anyhow::Result<Option<String>> {
    // Before the terminal changes mode: a sound library may print to stderr while it looks for a
    // device, and that should land on the normal screen.
    let mut refused = None;
    let device = use_audio(audio, matches!(source, Source::Machine(_)), || {
        Device::open().inspect_err(|error| refused = Some(error.to_string()))
    });
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
    let result = run_threads(
        &mut app,
        &mut renderer,
        &mut terminal,
        &mut gfx,
        pads,
        device.as_ref(),
    );
    let failed = device
        .as_ref()
        .is_some_and(|device| device.failed.load(Ordering::Acquire));
    outcome(
        result,
        join_notes(sound_note(refused.as_deref(), failed), app.save_note()),
    )
}

/// What to tell the user about the sound once the terminal is restored: why no device was used, or
/// that the one in use reported an error and the run went on without sound.
fn sound_note(refused: Option<&str>, failed: bool) -> Option<String> {
    match (refused, failed) {
        (Some(why), _) => Some(format!("no sound: {why}")),
        (None, true) => Some("the sound device reported an error; ran without sound".to_string()),
        (None, false) => None,
    }
}

/// Two notes as one, one per line.
fn join_notes(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}\n{second}")),
        (first, second) => first.or(second),
    }
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

/// Waits up to `wait` (a short one: the input loop's poll) for `fd` to become readable and says
/// whether what it leads to has hung up: the terminal's window closed or the connection dropped
/// (the other end of a pipe, in a test). An error on the descriptor counts as gone; an
/// interrupted wait does not, and the caller asks again.
///
/// A descriptor `poll` cannot watch (some terminal devices cannot be) tells nothing: the wait is
/// slept out and the answer is no, so a healthy run is never ended by a guess.
#[cfg(unix)]
fn hung_up(fd: impl AsFd, wait: Duration) -> bool {
    let timeout = Timespec {
        tv_sec: wait.as_secs().cast_signed(),
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let ready = poll(&mut fds, Some(&timeout));
    let state = fds[0].revents();
    if state.contains(PollFlags::NVAL) {
        thread::sleep(wait);
        return false;
    }
    matches!(ready, Ok(count) if count > 0) && state.intersects(PollFlags::HUP | PollFlags::ERR)
}

/// Whether the terminal is gone, after waiting up to `wait` for input from it. The event reader
/// reads the standard input when that is a terminal, so that is what is asked; when it is not,
/// the reader opens the terminal itself and there is nothing here to ask, so this only waits. A
/// terminal that goes away is then noticed by the render thread's failed write alone.
#[cfg(unix)]
fn terminal_gone(wait: Duration) -> bool {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        hung_up(&stdin, wait)
    } else {
        thread::sleep(wait);
        false
    }
}

/// No hangup check off POSIX yet (Windows is planned, AGENTS.md §2): this only waits.
#[cfg(not(unix))]
fn terminal_gone(wait: Duration) -> bool {
    thread::sleep(wait);
    false
}

/// The real terminal as the input loop reads it.
struct Tty;

impl TerminalInput for Tty {
    fn gone(&mut self, wait: Duration) -> bool {
        terminal_gone(wait)
    }

    fn next_event(&mut self) -> io::Result<Option<Event>> {
        // A zero wait: `gone` has done the waiting, and has seen the terminal alive.
        if event::poll(Duration::ZERO)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    }

    fn size(&self) -> io::Result<TermSize> {
        size()
    }
}

/// Runs the emu, render and input threads until one of them ends, then joins them. The emu
/// thread keeps time by `device`'s ring when there is one; the device's own thread is `cpal`'s.
///
/// A thread that returns, fails or panics sets `stop` through its [`StopGuard`], so the other two
/// end too; the emu thread still writes the save first. The first failure of render, input and
/// emu, in that order, is the result.
///
/// The input thread is not a scoped one, and is waited for only [`INPUT_GRACE`]: the event
/// reader can be handed a terminal that hung up between the loop's check and its read, and then
/// never returns. Such a thread is left behind; the process ends right after the run. Leaving it
/// is not a failure of the run: what ended the run is in the other two threads' results (a
/// failed write, when the terminal went away).
fn run_threads<S: ShmSink + Send>(
    app: &mut App,
    renderer: &mut Renderer<S>,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    gfx: &mut io::Stdout,
    mut pads: Gamepads,
    device: Option<&Device>,
) -> anyhow::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let handoff = Arc::new(Handoff::new(Snapshot::default));
    let (tx, rx) = mpsc::channel();
    let (mut pacer, mut sink) = paced(
        device.map(|device| {
            (
                Arc::clone(&device.ring),
                Arc::clone(&device.failed),
                device.rate,
            )
        }),
        Instant::now(),
    );
    let joined = |handle: thread::ScopedJoinHandle<'_, anyhow::Result<()>>, name: &str| {
        handle
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the {name} thread panicked")))
    };
    let input = {
        let (stop, handoff) = (Arc::clone(&stop), Arc::clone(&handoff));
        let connected = pads.connected();
        thread::Builder::new()
            .name("input".to_string())
            .spawn(move || {
                let _stop = StopGuard::new(&stop, &handoff);
                let poll = || (pads.poll(), pads.connected());
                run_input(&mut Tty, poll, connected, &tx, &stop)
            })
            .context("starting the input thread")?
    };
    let (stop, handoff) = (&*stop, &*handoff);
    let (rendered, saved) = thread::scope(|scope| {
        let emu = scope.spawn(|| {
            let _stop = StopGuard::new(stop, handoff);
            // A `Receiver` is `Send` but not `Sync`: the emu thread takes it by value.
            let rx = rx;
            run_emu_and_save(
                app,
                &rx,
                handoff,
                &mut Pace {
                    pacer: &mut pacer,
                    sink: &mut sink,
                },
                stop,
                Instant::now,
                thread::sleep,
            )
        });
        let render = scope.spawn(|| {
            let _stop = StopGuard::new(stop, handoff);
            run_render(renderer, terminal, gfx, handoff, stop)
        });
        (joined(render, "render"), joined(emu, "emu"))
    });
    // Both have ended, so `stop` is set and the input thread is on its way out, unless the
    // event reader holds it.
    let ended = finishes_within(
        || input.is_finished(),
        INPUT_GRACE,
        Instant::now,
        thread::sleep,
    );
    let read = if ended {
        input
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the input thread panicked")))
    } else {
        Ok(())
    };
    rendered.and(read).and(saved)
}

#[cfg(all(test, unix))]
mod tests;
