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
use crate::gamepad::PadEvent;
use crate::handoff::Handoff;
use crate::layout::TermSize;
use crate::pacing::Pacer;
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

/// Runs frames until the user quits, the program shuts the console down, or `stop` is set.
///
/// At the start of each frame it applies every event queued since the last one, so input is never
/// dropped and never starves a frame. It then runs the frame, publishes a snapshot, and waits as
/// `pacer` says. A frame is run for each `sleep`; `now` is the pacer's clock.
pub fn run_emu<P: Pacer>(
    app: &mut App,
    events: &Receiver<HostEvent>,
    handoff: &Handoff<Snapshot>,
    pacer: &mut P,
    stop: &AtomicBool,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
) {
    loop {
        pacer.frame_started(now());
        // A closed channel (the input thread is gone) ends nothing by itself: its guard has set
        // `stop`.
        while let Ok(event) = events.try_recv() {
            apply_host_event(app, event);
        }
        if app.should_quit() || stop.load(Ordering::SeqCst) {
            break;
        }
        app.step();
        handoff.publish(|snapshot| app.fill(snapshot));
        sleep(pacer.remaining(now()));
    }
}

/// The whole emu thread: the frame loop, then the save. The save is written however the loop
/// ended, even when render or input failed, so a game's save is not lost.
pub fn run_emu_and_save<P: Pacer>(
    app: &mut App,
    events: &Receiver<HostEvent>,
    handoff: &Handoff<Snapshot>,
    pacer: &mut P,
    stop: &AtomicBool,
    now: impl Fn() -> Instant,
    sleep: impl FnMut(Duration),
) -> anyhow::Result<()> {
    run_emu(app, events, handoff, pacer, stop, now, sleep);
    app.finish().context("writing the save file")
}

/// Shows frames from `handoff` until `stop` is set. A frame published before the stop is still
/// shown, so the last frame the emu thread ran is on screen; a frame the render side was too slow
/// for is skipped, not waited for. Flushes the graphics at the end.
pub fn run_render<S: ShmSink, B: Backend, W: Write>(
    renderer: &mut Renderer<S>,
    terminal: &mut Terminal<B>,
    gfx: &mut W,
    handoff: &Handoff<Snapshot>,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    loop {
        if let Some(shown) = handoff.take(RENDER_WAIT, |snapshot| {
            renderer.present(snapshot, terminal, gfx)
        }) {
            shown?;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
    }
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
