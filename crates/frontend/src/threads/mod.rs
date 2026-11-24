// SPDX-License-Identifier: Apache-2.0
//! The bodies of the three threads (#218): emu, render and input.
//!
//! Each loop is a plain function over what it drives: a channel of [`HostEvent`]s, a
//! [`Handoff`] of [`Snapshot`]s, a [`Pacer`], a clock and a sleep, a terminal backend and a
//! writer. A test runs one without spawning a thread, with a pre-filled channel, a no-op sleep, a
//! `TestBackend` and a `Vec<u8>`. [`crate::term`] keeps only the OS wiring: the real terminal, the
//! real pads, and spawning.
//!
//! The emu thread owns the model ([`App`]); the render thread owns the [`Renderer`] and the
//! terminal; the input thread owns the event reader and the pads. Input hands the emu thread
//! [`Action`]s, not a built `Input`: the keys' frame counts, the cursor and the pen mapping all
//! count emu frames or need the layout, so they stay on the emu thread and the mapping is
//! unchanged.

use crate::app::{Action, App, map_event};
use crate::audio::sink::AudioSink;
use crate::gamepad::PadEvent;
use crate::handoff::Handoff;
use crate::layout::TermSize;
use crate::pacing::{FRAME_TIME, Pacer};
use crate::render::{Renderer, Snapshot};
use crate::transfer::ShmSink;
use anyhow::Context as _;
use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::Backend;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// How long the render thread waits for a frame before it looks at the stop flag again. A stop
/// wakes it at once ([`Handoff::wake`]); this only bounds a wake that was missed.
pub const RENDER_WAIT: Duration = Duration::from_millis(50);

/// How long the input thread waits for a terminal event before it polls the pads and looks at the
/// stop flag again: it bounds how fast the thread sees a stop, and how late a pad event is seen.
pub const INPUT_POLL: Duration = Duration::from_millis(4);

/// How long the emu thread waits between two looks at its events while the run is paused: a
/// frame's time, so the focus coming back is acted on as fast as a key is.
pub const PAUSED_POLL: Duration = FRAME_TIME;

/// What the input thread tells the emu thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostEvent {
    /// A mapped terminal event. Never [`Action::Resized`] or [`Action::None`]: a resize carries
    /// its new size, and nothing is not worth a message.
    Action(Action),
    /// The terminal changed size.
    Resize(TermSize),
    /// A gamepad event.
    Pad(PadEvent),
    /// How many pads are plugged in now.
    PadsConnected(u32),
}

/// Sets the stop flag and wakes the render thread when dropped, so a thread that returns, fails or
/// panics stops the other two.
pub struct StopGuard<'a> {
    stop: &'a AtomicBool,
    handoff: &'a Handoff<Snapshot>,
}

impl<'a> StopGuard<'a> {
    /// A guard that stops the run through `stop` and wakes the reader of `handoff`.
    pub fn new(stop: &'a AtomicBool, handoff: &'a Handoff<Snapshot>) -> Self {
        Self { stop, handoff }
    }
}

impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.handoff.wake();
    }
}

/// Applies a host event to the model.
pub fn apply_host_event(app: &mut App, event: HostEvent) {
    match event {
        HostEvent::Action(action) => app.apply(action),
        HostEvent::Resize(size) => app.resize(size),
        HostEvent::Pad(pad) => app.apply_pad(pad),
        HostEvent::PadsConnected(connected) => app.set_pads_connected(connected),
    }
}

/// What paces the emu loop and takes its audio: the [`Pacer`] says when the next frame starts and
/// whether audio is still fed, the [`AudioSink`] takes each frame's audio.
pub struct Pace<'a, P, S> {
    /// When the next frame starts.
    pub pacer: &'a mut P,
    /// Where the frame's audio goes.
    pub sink: &'a mut S,
}

/// Runs frames until the user quits, the program shuts the console down, or `stop` is set.
///
/// At the start of each frame it applies every event queued since the last one, so input is never
/// dropped and never starves a frame. It then runs the frame, hands the frame's audio to `sink`
/// while its pacer still feeds audio and there is any (none without a ROM), publishes a snapshot,
/// and waits as `pacer` says. `sleep` is called once a turn; `now` is the pacer's clock.
///
/// While the app is paused ([`App::paused`]: the terminal is not focused) a turn runs no frame
/// and pushes no audio, so the device plays out what it has and falls silent. It publishes a
/// snapshot only when an event came in (the pause itself, a resize), and waits [`PAUSED_POLL`]
/// whatever the pacer says: an audio pacer sees an empty ring and would ask for the next frame at
/// once. A paused turn is not a frame, so the pacer and the sink are not told one started: to a
/// pacer the whole pause is one long gap between two frames, which never alone makes it give a
/// device up. The turn the pause begins writes the save file if anything is pending
/// ([`App::finish`]), since no frame will come to do it and the terminal may end the program
/// before the focus is back. The first turn after the pause tells the sink
/// ([`AudioSink::resumed`]).
pub fn run_emu<P: Pacer, S: AudioSink>(
    app: &mut App,
    events: &Receiver<HostEvent>,
    handoff: &Handoff<Snapshot>,
    pace: &mut Pace<'_, P, S>,
    stop: &AtomicBool,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
) {
    let mut was_paused = false;
    loop {
        if !app.paused() {
            pace.pacer.frame_started(now());
            pace.sink.frame_started();
        }
        // A closed channel (the input thread is gone) ends nothing by itself: its guard has set
        // `stop`.
        let mut changed = false;
        while let Ok(event) = events.try_recv() {
            apply_host_event(app, event);
            changed = true;
        }
        if app.should_quit() || stop.load(Ordering::SeqCst) {
            break;
        }
        if app.paused() {
            if !was_paused {
                // A failed write is not lost: the memory stays pending, and the flush at the
                // end of the run tries again and reports it.
                let _ = app.finish();
            }
            if changed {
                handoff.publish(|snapshot| app.fill(snapshot));
            }
            was_paused = true;
            sleep(PAUSED_POLL);
            continue;
        }
        if std::mem::take(&mut was_paused) {
            // This turn began paused, so its start was not announced above.
            pace.pacer.frame_started(now());
            pace.sink.frame_started();
            pace.sink.resumed();
        }
        app.step();
        if pace.pacer.feeds_audio() && !app.audio().is_empty() {
            pace.sink.push(app.audio());
        }
        handoff.publish(|snapshot| app.fill(snapshot));
        sleep(pace.pacer.remaining(now()));
    }
}

/// The whole emu thread: the frame loop, then the save. The save is written however the loop
/// ended, even when render or input failed, so a game's save is not lost.
pub fn run_emu_and_save<P: Pacer, S: AudioSink>(
    app: &mut App,
    events: &Receiver<HostEvent>,
    handoff: &Handoff<Snapshot>,
    pace: &mut Pace<'_, P, S>,
    stop: &AtomicBool,
    now: impl Fn() -> Instant,
    sleep: impl FnMut(Duration),
) -> anyhow::Result<()> {
    run_emu(app, events, handoff, pace, stop, now, sleep);
    app.finish().context("writing the save file")
}

/// Shows frames from `handoff` until `stop` is set, then shows one more if a frame is still
/// waiting, so the last frame the emu thread published is on screen. A frame the render side was
/// too slow for is skipped, not waited for. Flushes the graphics at the end.
pub fn run_render<S: ShmSink, B: Backend, W: Write>(
    renderer: &mut Renderer<S>,
    terminal: &mut Terminal<B>,
    gfx: &mut W,
    handoff: &Handoff<Snapshot>,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    let mut show = |wait: Duration| {
        handoff
            .take(wait, |snapshot| renderer.present(snapshot, terminal, gfx))
            .transpose()
    };
    while !stop.load(Ordering::SeqCst) {
        show(RENDER_WAIT)?;
    }
    // The emu thread publishes before it stops, so anything it published is already waiting.
    show(Duration::ZERO)?;
    gfx.flush()?;
    Ok(())
}

/// Sends the emu thread what a terminal `event` means: a resize with the size `query_size` reads
/// now, everything else as the [`Action`] it maps to. `false` once the emu thread has gone.
pub fn forward_event(
    event: &Event,
    query_size: impl FnOnce() -> io::Result<TermSize>,
    tx: &Sender<HostEvent>,
) -> anyhow::Result<bool> {
    let host = match map_event(event) {
        Action::None => return Ok(true),
        Action::Resized => HostEvent::Resize(query_size()?),
        action => HostEvent::Action(action),
    };
    Ok(tx.send(host).is_ok())
}

/// Sends the emu thread the pad `events` and, when `connected` differs from `last`, the new count.
/// `false` once the emu thread has gone.
pub fn forward_pads(
    events: impl IntoIterator<Item = PadEvent>,
    connected: u32,
    last: &mut u32,
    tx: &Sender<HostEvent>,
) -> bool {
    let counted = (connected != *last).then_some(HostEvent::PadsConnected(connected));
    *last = connected;
    events
        .into_iter()
        .map(HostEvent::Pad)
        .chain(counted)
        .all(|event| tx.send(event).is_ok())
}

#[cfg(test)]
mod tests;
