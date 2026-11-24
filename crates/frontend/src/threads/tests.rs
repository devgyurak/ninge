// SPDX-License-Identifier: Apache-2.0
//! Tests of `threads`: each loop is driven directly, with no thread spawned.

use super::*;
use crate::audio::sink::NoAudio;
use crate::pacing::{AudioPacer, FramePacer, Level, Pacing};
use crate::savefile::SaveFile;
use crate::source::Source;
use crate::transfer::{NoShm, Transfer};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ninge_core::Buttons;
use ratatui::backend::TestBackend;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::channel;

fn size() -> TermSize {
    TermSize {
        cols: 120,
        rows: 50,
        cell_w: 10,
        cell_h: 20,
        px_known: true,
    }
}

fn rom(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testroms/build")
            .join(name),
    )
    .expect("test ROM is built")
}

/// An app running `rom_name` with a save file whose memory counts as changed (a savestate load
/// raises the generation), so a `finish` has something to write.
fn app_with_changed_save(rom_name: &str, file: &str) -> (App, PathBuf) {
    let mut machine = crate::source::boot(rom(rom_name)).expect("valid header");
    machine
        .insert_save(ninge_core::cart::SaveType::Eeprom512, vec![0x21; 512])
        .expect("512 bytes fit");
    let path = std::env::temp_dir().join(format!("ninge-threads-{}-{file}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let save = SaveFile::new(path.clone(), &machine);
    let state = machine.save_state();
    machine.load_state(&state).expect("own state loads");
    let app = App::new(size())
        .with_key_release(true)
        .with_source(Source::Machine(Box::new(machine)))
        .with_save(Some(save));
    (app, path)
}

fn key(code: KeyCode, kind: KeyEventKind) -> HostEvent {
    HostEvent::Action(Action::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind,
        state: KeyEventState::NONE,
    }))
}

/// A sleep that counts the frames and sends a quit once `frames` have run.
fn quit_after(tx: Sender<HostEvent>, frames: u32) -> impl FnMut(Duration) {
    let mut count = 0;
    move |_| {
        count += 1;
        if count == frames {
            tx.send(HostEvent::Action(Action::Quit)).expect("receiver");
        }
    }
}

/// What a run of the emu thread's body left behind.
struct EmuRun {
    result: anyhow::Result<()>,
    app: App,
    save_file: Option<Vec<u8>>,
}

/// Runs `run_emu_and_save` over `app` with `events` already queued and `sleep` as the wait.
fn run_emu_thread(
    mut app: App,
    path: Option<PathBuf>,
    events: Vec<HostEvent>,
    stop: bool,
    make_sleep: impl FnOnce(Sender<HostEvent>) -> Box<dyn FnMut(Duration)>,
) -> EmuRun {
    let (tx, rx) = channel();
    events
        .into_iter()
        .for_each(|e| tx.send(e).expect("receiver"));
    let handoff = Handoff::new(Snapshot::default);
    let stop = AtomicBool::new(stop);
    let mut sleep = make_sleep(tx);
    let result = run_emu_and_save(
        &mut app,
        &rx,
        &handoff,
        &mut Pace {
            pacer: &mut FramePacer::new(Instant::now()),
            sink: &mut NoAudio,
        },
        &stop,
        Instant::now,
        &mut sleep,
    );
    let save_file = path.and_then(|p| {
        let bytes = std::fs::read(&p).ok();
        let _ = std::fs::remove_file(p);
        bytes
    });
    EmuRun {
        result,
        app,
        save_file,
    }
}

fn frames(app: &App) -> Option<u64> {
    app.machine().map(ninge_core::Machine::frame_count)
}

fn input_buttons(app: &App) -> Buttons {
    app.machine().expect("a machine").inspect().input().buttons
}

#[test]
fn given_queued_events_and_a_quit_when_the_emu_runs_then_the_machine_got_the_buttons_and_the_save_is_written()
 {
    let (app, path) = app_with_changed_save("smoke.nds", "events.sav");
    let run = run_emu_thread(
        app,
        Some(path),
        vec![key(KeyCode::Char('x'), KeyEventKind::Press)],
        false,
        |tx| Box::new(quit_after(tx, 3)),
    );
    assert_eq!(
        (
            run.result.is_ok(),
            frames(&run.app),
            input_buttons(&run.app),
            run.save_file
        ),
        (true, Some(3), Buttons::A, Some(vec![0x21; 512]))
    );
}

#[test]
fn given_a_program_that_shuts_the_console_down_when_the_emu_runs_then_it_stops_and_the_save_is_written()
 {
    let (app, path) = app_with_changed_save("poweroff.nds", "poweroff.sav");
    let run = run_emu_thread(app, Some(path), Vec::new(), false, |_| Box::new(|_| {}));
    assert_eq!(
        (run.result.is_ok(), run.app.should_quit(), run.save_file),
        (true, true, Some(vec![0x21; 512]))
    );
}

#[test]
fn given_stop_set_by_another_thread_when_the_emu_runs_then_it_runs_no_frame_and_still_saves() {
    let (app, path) = app_with_changed_save("smoke.nds", "stopped.sav");
    let run = run_emu_thread(app, Some(path), Vec::new(), true, |_| Box::new(|_| {}));
    assert_eq!(
        (run.result.is_ok(), frames(&run.app), run.save_file),
        (true, Some(0), Some(vec![0x21; 512]))
    );
}

#[test]
fn given_a_quit_queued_before_any_frame_when_the_emu_runs_then_no_frame_runs() {
    let machine = crate::source::boot(rom("smoke.nds")).expect("valid header");
    let run = run_emu_thread(
        App::new(size()).with_source(Source::Machine(Box::new(machine))),
        None,
        vec![HostEvent::Action(Action::Quit)],
        false,
        |_| Box::new(|_| {}),
    );
    assert_eq!((run.app.should_quit(), frames(&run.app)), (true, Some(0)));
}

#[test]
fn given_frames_when_the_emu_runs_then_each_frame_publishes_a_snapshot_and_waits_as_paced() {
    let (tx, rx) = channel();
    let handoff = Handoff::new(Snapshot::default);
    let waits = std::cell::RefCell::new(Vec::new());
    let mut app = App::new(size());
    let started = Instant::now();
    run_emu(
        &mut app,
        &rx,
        &handoff,
        &mut Pace {
            pacer: &mut FramePacer::new(started),
            sink: &mut NoAudio,
        },
        &AtomicBool::new(false),
        || started,
        |wait| {
            waits.borrow_mut().push(wait);
            tx.send(HostEvent::Action(Action::Quit)).expect("receiver");
        },
    );
    let shown = handoff.take(Duration::ZERO, |s| s.areas.is_some());
    assert_eq!(
        (shown, waits.into_inner()),
        (Some(true), vec![crate::pacing::FRAME_TIME])
    );
}

// ----------------------------------------------------------------------------------- audio

/// A sink that keeps what each frame handed it.
#[derive(Default)]
struct Recorder {
    frames: Vec<Vec<i16>>,
    starts: u32,
}

impl AudioSink for Recorder {
    fn frame_started(&mut self) {
        self.starts += 1;
    }

    fn push(&mut self, samples: &[i16]) {
        self.frames.push(samples.to_vec());
    }
}

/// A sleep that sets `failed` after `fail_after` frames and quits after `quit_after`.
fn fail_then_quit(
    failed: Arc<AtomicBool>,
    tx: Sender<HostEvent>,
    fail_after: u32,
    quit_after: u32,
) -> impl FnMut(Duration) {
    let mut count = 0;
    move |_| {
        count += 1;
        if count == fail_after {
            failed.store(true, Ordering::Release);
        }
        if count == quit_after {
            tx.send(HostEvent::Action(Action::Quit)).expect("receiver");
        }
    }
}

/// What the sink got when `app` ran under `pacer` until `quit_after` frames had run; `fail_after`
/// frames in, `failed` is set.
fn sunk_frames(
    mut app: App,
    pacer: &mut impl Pacer,
    failed: &Arc<AtomicBool>,
    fail_after: u32,
    quit_after: u32,
) -> (Vec<Vec<i16>>, u32) {
    let (tx, rx) = channel();
    let mut sink = Recorder::default();
    run_emu(
        &mut app,
        &rx,
        &Handoff::new(Snapshot::default),
        &mut Pace {
            pacer,
            sink: &mut sink,
        },
        &AtomicBool::new(false),
        Instant::now,
        fail_then_quit(Arc::clone(failed), tx, fail_after, quit_after),
    );
    (sink.frames, sink.starts)
}

fn smoke_app() -> App {
    let machine = crate::source::boot(rom("smoke.nds")).expect("valid header");
    App::new(size()).with_source(Source::Machine(Box::new(machine)))
}

/// A pacing that follows a ring that is always empty (so it never waits) until `failed` is set.
fn audio_pacing(failed: &Arc<AtomicBool>) -> Pacing<fn() -> Level> {
    let pacer = AudioPacer::new(
        (|| Level { fill: 0, pulled: 0 }) as fn() -> Level,
        2_400,
        48_000,
    );
    Pacing::audio(pacer, Arc::clone(failed), Instant::now())
}

/// The audio of the first `frames` frames of the smoke ROM on a bare machine.
fn reference_audio(frames: usize) -> Vec<Vec<i16>> {
    let mut machine = crate::source::boot(rom("smoke.nds")).expect("valid header");
    (0..frames)
        .map(|_| {
            machine.run_frame(ninge_core::Input::default());
            machine.audio().to_vec()
        })
        .collect()
}

#[test]
fn given_audio_pacing_when_frames_run_then_each_frames_audio_reaches_the_sink_in_order() {
    let failed = Arc::new(AtomicBool::new(false));
    let (got, _) = sunk_frames(smoke_app(), &mut audio_pacing(&failed), &failed, 99, 3);
    assert_eq!(got, reference_audio(3));
}

#[test]
fn given_frames_when_they_run_then_the_sink_is_told_each_frame_started() {
    let failed = Arc::new(AtomicBool::new(false));
    let (_, starts) = sunk_frames(smoke_app(), &mut audio_pacing(&failed), &failed, 99, 3);
    // A fourth start runs before the quit queued during the third wait is seen.
    assert_eq!(starts, 4);
}

#[test]
fn given_no_rom_when_frames_run_under_audio_pacing_then_nothing_is_pushed() {
    let failed = Arc::new(AtomicBool::new(false));
    let (got, _) = sunk_frames(App::new(size()), &mut audio_pacing(&failed), &failed, 99, 3);
    assert_eq!(got, Vec::<Vec<i16>>::new());
}

#[test]
fn given_frame_pacing_when_frames_run_then_nothing_is_pushed() {
    let failed = Arc::new(AtomicBool::new(false));
    let (got, _) = sunk_frames(
        smoke_app(),
        &mut FramePacer::new(Instant::now()),
        &failed,
        99,
        3,
    );
    assert_eq!(got, Vec::<Vec<i16>>::new());
}

#[test]
fn given_the_pacing_falls_back_when_more_frames_run_then_nothing_more_is_pushed() {
    let failed = Arc::new(AtomicBool::new(false));
    // Frames 1 and 2 are pushed; `failed` is set while the second one waits, so frame 3 starts on
    // frame pacing and frame 4 too.
    let (got, _) = sunk_frames(smoke_app(), &mut audio_pacing(&failed), &failed, 2, 4);
    assert_eq!(got, reference_audio(2));
}

#[test]
fn given_each_host_event_when_applied_then_the_model_takes_it() {
    let mut app = App::new(size()).with_gamepads(true, 0);
    let small = TermSize {
        cols: 2,
        rows: 1,
        ..size()
    };
    apply_host_event(&mut app, HostEvent::Resize(small));
    apply_host_event(&mut app, HostEvent::PadsConnected(2));
    apply_host_event(&mut app, HostEvent::Action(Action::Quit));
    let mut snapshot = Snapshot::default();
    app.fill(&mut snapshot);
    assert_eq!(
        (app.areas(), snapshot.pads_connected, app.should_quit()),
        (None, 2, true)
    );
}

// ---------------------------------------------------------------------------------- render

/// A snapshot of one frame of the dummy pattern.
fn published() -> Handoff<Snapshot> {
    let mut app = App::new(size());
    app.step();
    let handoff = Handoff::new(Snapshot::default);
    handoff.publish(|snapshot| app.fill(snapshot));
    handoff
}

#[test]
fn given_a_published_snapshot_and_stop_when_the_render_runs_then_both_images_are_written_and_cells_drawn()
 {
    let handoff = published();
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    let mut gfx = Vec::new();
    let mut renderer = Renderer::new(NoShm, Transfer::Direct { reason: None });
    let result = run_render(
        &mut renderer,
        &mut terminal,
        &mut gfx,
        &handoff,
        &AtomicBool::new(true),
    );
    let text = String::from_utf8_lossy(&gfx).into_owned();
    let panel = format!("{}", terminal.backend());
    assert_eq!(
        (
            result.is_ok(),
            text.contains("i=1"),
            text.contains("i=2"),
            panel.contains("Registers")
        ),
        (true, true, true, true)
    );
}

/// Fails every put.
struct FailingSink;

impl ShmSink for FailingSink {
    fn put(&mut self, _data: &[u8]) -> io::Result<String> {
        Err(io::Error::other("no space left"))
    }
}

#[test]
fn given_a_failing_shm_sink_when_the_render_runs_then_it_sends_zlib_and_the_panel_says_why() {
    let handoff = published();
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    let mut gfx = Vec::new();
    let mut renderer = Renderer::new(FailingSink, Transfer::Shm);
    let _ = run_render(
        &mut renderer,
        &mut terminal,
        &mut gfx,
        &handoff,
        &AtomicBool::new(true),
    );
    let text = String::from_utf8_lossy(&gfx).into_owned();
    let panel = format!("{}", terminal.backend());
    assert_eq!(
        (
            text.contains("t=s"),
            text.contains("t=d,o=z"),
            panel.contains("shm write failed:")
        ),
        (false, true, true)
    );
}

#[test]
fn given_nothing_published_and_stop_when_the_render_runs_then_it_writes_nothing_and_returns() {
    let handoff = Handoff::new(Snapshot::default);
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    let mut gfx = Vec::new();
    let mut renderer = Renderer::new(NoShm, Transfer::Direct { reason: None });
    handoff.wake();
    let result = run_render(
        &mut renderer,
        &mut terminal,
        &mut gfx,
        &handoff,
        &AtomicBool::new(true),
    );
    assert_eq!((result.is_ok(), gfx), (true, Vec::<u8>::new()));
}

// ------------------------------------------------------------------------------------ stop

#[test]
fn given_a_stop_guard_when_it_is_dropped_then_stop_is_set() {
    let stop = AtomicBool::new(false);
    let handoff = Handoff::new(Snapshot::default);
    drop(StopGuard::new(&stop, &handoff));
    assert!(stop.load(Ordering::SeqCst));
}

fn fail_under_guard(stop: &AtomicBool, handoff: &Handoff<Snapshot>) -> anyhow::Result<()> {
    let _guard = StopGuard::new(stop, handoff);
    anyhow::bail!("draw failed")
}

#[test]
fn given_a_thread_body_that_returns_an_error_when_it_ends_then_stop_is_set() {
    let stop = AtomicBool::new(false);
    let handoff = Handoff::new(Snapshot::default);
    let result = fail_under_guard(&stop, &handoff);
    assert_eq!((result.is_err(), stop.load(Ordering::SeqCst)), (true, true));
}

#[test]
fn given_a_thread_body_that_panics_when_it_unwinds_then_stop_is_set() {
    let stop = AtomicBool::new(false);
    let handoff = Handoff::new(Snapshot::default);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = StopGuard::new(&stop, &handoff);
        panic!("a thread failed");
    }));
    assert_eq!(
        (unwound.is_err(), stop.load(Ordering::SeqCst)),
        (true, true)
    );
}

#[test]
fn given_a_stop_guard_when_it_is_dropped_then_a_waiting_reader_is_woken_at_once() {
    let stop = AtomicBool::new(false);
    let handoff = Handoff::new(Snapshot::default);
    drop(StopGuard::new(&stop, &handoff));
    let started = Instant::now();
    let taken = handoff.take(Duration::from_secs(30), |_| ());
    assert_eq!(
        (taken, started.elapsed() < Duration::from_secs(10)),
        (None, true)
    );
}

// ------------------------------------------------------------------------------------ input

fn plain_key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Runs `forward_event` with a size query that reports `queried`, returning what was sent.
fn forwarded(
    event: &Event,
    queried: io::Result<TermSize>,
) -> (anyhow::Result<bool>, Vec<HostEvent>) {
    let (tx, rx) = channel();
    let result = forward_event(event, || queried, &tx);
    drop(tx);
    (result, rx.try_iter().collect())
}

#[test]
fn given_a_key_event_when_forwarded_then_the_emu_gets_the_action() {
    let (result, sent) = forwarded(&plain_key(KeyCode::Tab), Ok(size()));
    assert_eq!(
        (result.expect("sent"), sent),
        (
            true,
            vec![HostEvent::Action(Action::Key(KeyEvent::new(
                KeyCode::Tab,
                KeyModifiers::NONE
            )))]
        )
    );
}

#[test]
fn given_a_quit_chord_when_forwarded_then_the_emu_gets_quit() {
    let chord = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
    let (_, sent) = forwarded(&chord, Ok(size()));
    assert_eq!(sent, vec![HostEvent::Action(Action::Quit)]);
}

#[test]
fn given_a_resize_when_forwarded_then_the_emu_gets_the_size_the_query_reports() {
    let (_, sent) = forwarded(&Event::Resize(10, 10), Ok(size()));
    assert_eq!(sent, vec![HostEvent::Resize(size())]);
}

#[test]
fn given_a_failing_size_query_when_a_resize_is_forwarded_then_it_is_an_error_and_nothing_is_sent() {
    let (result, sent) = forwarded(&Event::Resize(10, 10), Err(io::Error::other("no size")));
    assert_eq!((result.is_err(), sent), (true, Vec::new()));
}

#[test]
fn given_an_event_the_console_ignores_when_forwarded_then_nothing_is_sent() {
    let (result, sent) = forwarded(&Event::FocusGained, Ok(size()));
    assert_eq!((result.expect("ok"), sent), (true, Vec::new()));
}

#[test]
fn given_focus_lost_when_forwarded_then_the_emu_gets_release_all() {
    let (_, sent) = forwarded(&Event::FocusLost, Ok(size()));
    assert_eq!(sent, vec![HostEvent::Action(Action::ReleaseAll)]);
}

#[test]
fn given_the_emu_gone_when_an_event_is_forwarded_then_it_reports_false() {
    let (tx, rx) = channel();
    drop(rx);
    let result = forward_event(&plain_key(KeyCode::Tab), || Ok(size()), &tx);
    assert!(!result.expect("not an error"));
}

#[test]
fn given_pad_events_and_a_changed_count_when_forwarded_then_the_events_come_first_then_the_count() {
    let (tx, rx) = channel();
    let mut last = 0;
    let open = forward_pads(
        [PadEvent::Button(gilrs::Button::East, true)],
        1,
        &mut last,
        &tx,
    );
    assert_eq!(
        (open, last, rx.try_iter().collect::<Vec<_>>()),
        (
            true,
            1,
            vec![
                HostEvent::Pad(PadEvent::Button(gilrs::Button::East, true)),
                HostEvent::PadsConnected(1)
            ]
        )
    );
}

#[test]
fn given_an_unchanged_count_and_no_events_when_pads_are_forwarded_then_nothing_is_sent() {
    let (tx, rx) = channel();
    let mut last = 2;
    let open = forward_pads(Vec::new(), 2, &mut last, &tx);
    assert_eq!((open, rx.try_iter().count()), (true, 0));
}

#[test]
fn given_the_emu_gone_when_pads_are_forwarded_then_it_reports_false() {
    let (tx, rx) = channel();
    drop(rx);
    let mut last = 0;
    assert!(!forward_pads([PadEvent::Disconnected], 0, &mut last, &tx));
}

/// One thing a scripted terminal does when `next_event` is called.
enum Step {
    /// Hands this event out.
    Event(Event),
    /// Sets `stop`, then hands this event out: a stop that comes in the middle of a burst.
    StopThen(Event),
    /// Fails the read.
    Fail,
    /// Reports that nothing more is waiting, which ends the turn's reads.
    Idle,
}

/// A terminal that does what it is told: hands out its steps in order (nothing once they are
/// spent), answers `gone` true from the `gone_from`th call on, and sets `stop` at the
/// `stop_at`th call of `gone` (both counted from 1).
struct Scripted {
    steps: VecDeque<Step>,
    gone_from: Option<u32>,
    stop_at: Option<u32>,
    stop: Arc<AtomicBool>,
    reported_size: Option<TermSize>,
    gone_calls: u32,
    event_calls: u32,
}

impl Scripted {
    fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            gone_from: None,
            stop_at: None,
            stop: Arc::new(AtomicBool::new(false)),
            reported_size: Some(size()),
            gone_calls: 0,
            event_calls: 0,
        }
    }
}

impl TerminalInput for Scripted {
    fn gone(&mut self, _wait: Duration) -> bool {
        self.gone_calls += 1;
        if self.stop_at == Some(self.gone_calls) {
            self.stop.store(true, Ordering::SeqCst);
        }
        self.gone_from.is_some_and(|from| self.gone_calls >= from)
    }

    fn next_event(&mut self) -> io::Result<Option<Event>> {
        self.event_calls += 1;
        match self.steps.pop_front() {
            Some(Step::Event(event)) => Ok(Some(event)),
            Some(Step::StopThen(event)) => {
                self.stop.store(true, Ordering::SeqCst);
                Ok(Some(event))
            }
            Some(Step::Fail) => Err(io::Error::other("read failed")),
            Some(Step::Idle) | None => Ok(None),
        }
    }

    fn size(&self) -> io::Result<TermSize> {
        self.reported_size
            .ok_or_else(|| io::Error::other("no size"))
    }
}

/// What a `run_input` run left behind.
struct InputRun {
    /// The error's message, `None` for `Ok`.
    error: Option<String>,
    sent: Vec<HostEvent>,
    /// How many times the pads were asked.
    pad_asks: u32,
}

/// Runs `run_input` on `terminal` with `pad_turns` as what the pads report on each ask (nothing,
/// and the last count, once spent), starting from `connected` pads. `emu_gone` drops the
/// receiver first.
fn run_scripted(
    terminal: &mut Scripted,
    pad_turns: Vec<(Vec<PadEvent>, u32)>,
    connected: u32,
    emu_gone: bool,
) -> InputRun {
    let (tx, rx) = channel();
    if emu_gone {
        drop(rx);
        let (error, pad_asks) = drive(terminal, pad_turns, connected, &tx);
        return InputRun {
            error,
            sent: Vec::new(),
            pad_asks,
        };
    }
    let (error, pad_asks) = drive(terminal, pad_turns, connected, &tx);
    drop(tx);
    InputRun {
        error,
        sent: rx.try_iter().collect(),
        pad_asks,
    }
}

fn drive(
    terminal: &mut Scripted,
    pad_turns: Vec<(Vec<PadEvent>, u32)>,
    connected: u32,
    tx: &Sender<HostEvent>,
) -> (Option<String>, u32) {
    let stop = Arc::clone(&terminal.stop);
    let mut turns: VecDeque<_> = pad_turns.into();
    let mut asks = 0;
    let mut count = connected;
    let pads = || {
        asks += 1;
        let (events, now) = turns.pop_front().unwrap_or((Vec::new(), count));
        count = now;
        (events, now)
    };
    let result = run_input(terminal, pads, connected, tx, &stop);
    (result.err().map(|error| format!("{error:#}")), asks)
}

fn key_step(code: KeyCode) -> Step {
    Step::Event(plain_key(code))
}

fn key_sent(code: KeyCode) -> HostEvent {
    HostEvent::Action(Action::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

#[test]
fn given_a_terminal_gone_at_the_first_turn_when_the_input_runs_then_it_fails_before_any_read() {
    let mut terminal = Scripted::new([key_step(KeyCode::Tab)]);
    terminal.gone_from = Some(1);
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (
            run.error
                .is_some_and(|e| e.contains("the terminal is gone")),
            terminal.event_calls,
            run.sent,
            run.pad_asks
        ),
        (true, 0, Vec::new(), 0)
    );
}

#[test]
fn given_a_terminal_that_goes_away_after_two_turns_when_the_input_runs_then_it_fails_with_the_earlier_events_forwarded()
 {
    let mut terminal = Scripted::new([
        key_step(KeyCode::Char('a')),
        Step::Idle,
        key_step(KeyCode::Char('b')),
        Step::Idle,
        key_step(KeyCode::Char('c')),
    ]);
    terminal.gone_from = Some(3);
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (
            run.error
                .is_some_and(|e| e.contains("the terminal is gone")),
            run.sent,
            terminal.gone_calls
        ),
        (
            true,
            vec![key_sent(KeyCode::Char('a')), key_sent(KeyCode::Char('b'))],
            3
        )
    );
}

#[test]
fn given_several_events_waiting_when_one_turn_runs_then_all_are_forwarded_in_order_before_the_next_ask()
 {
    let mut terminal = Scripted::new([
        key_step(KeyCode::Char('a')),
        key_step(KeyCode::Char('b')),
        key_step(KeyCode::Char('c')),
    ]);
    // Stop comes at the second ask: whatever the first turn did not forward is never sent.
    terminal.stop_at = Some(2);
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (run.error, run.sent, terminal.gone_calls),
        (
            None,
            vec![
                key_sent(KeyCode::Char('a')),
                key_sent(KeyCode::Char('b')),
                key_sent(KeyCode::Char('c'))
            ],
            2
        )
    );
}

#[test]
fn given_stop_set_in_the_middle_of_a_burst_when_the_input_runs_then_it_returns_ok_and_reads_no_more()
 {
    let mut terminal = Scripted::new([
        key_step(KeyCode::Char('a')),
        Step::StopThen(plain_key(KeyCode::Char('b'))),
        key_step(KeyCode::Char('c')),
    ]);
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (run.error, run.sent, terminal.event_calls, run.pad_asks),
        (None, vec![key_sent(KeyCode::Char('a'))], 2, 0)
    );
}

#[test]
fn given_stop_already_set_when_the_input_runs_then_it_returns_ok_without_asking_the_terminal() {
    let mut terminal = Scripted::new([key_step(KeyCode::Tab)]);
    terminal.stop.store(true, Ordering::SeqCst);
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (
            run.error,
            terminal.gone_calls,
            terminal.event_calls,
            run.sent,
            run.pad_asks
        ),
        (None, 0, 0, Vec::new(), 0)
    );
}

#[test]
fn given_the_emu_gone_and_an_event_waiting_when_the_input_runs_then_it_returns_ok_without_asking_the_pads()
 {
    let mut terminal = Scripted::new([key_step(KeyCode::Tab)]);
    let run = run_scripted(&mut terminal, Vec::new(), 0, true);
    assert_eq!((run.error, run.pad_asks), (None, 0));
}

#[test]
fn given_the_emu_gone_and_only_pad_events_when_the_input_runs_then_it_returns_ok_after_one_ask() {
    let mut terminal = Scripted::new([]);
    let pads = vec![(vec![PadEvent::Disconnected], 0)];
    let run = run_scripted(&mut terminal, pads, 0, true);
    assert_eq!((run.error, run.pad_asks, terminal.gone_calls), (None, 1, 1));
}

#[test]
fn given_pad_events_and_a_changed_count_when_the_input_runs_then_the_events_come_first_then_the_count()
 {
    let mut terminal = Scripted::new([]);
    terminal.stop_at = Some(1);
    let pads = vec![(vec![PadEvent::Button(gilrs::Button::East, true)], 1)];
    let run = run_scripted(&mut terminal, pads, 0, false);
    assert_eq!(
        (run.error, run.sent),
        (
            None,
            vec![
                HostEvent::Pad(PadEvent::Button(gilrs::Button::East, true)),
                HostEvent::PadsConnected(1)
            ]
        )
    );
}

#[test]
fn given_a_count_equal_to_the_starting_one_when_the_input_runs_then_no_pads_connected_is_sent() {
    let mut terminal = Scripted::new([]);
    terminal.stop_at = Some(1);
    let pads = vec![(vec![PadEvent::Button(gilrs::Button::East, true)], 2)];
    let run = run_scripted(&mut terminal, pads, 2, false);
    assert_eq!(
        (run.error, run.sent),
        (
            None,
            vec![HostEvent::Pad(PadEvent::Button(gilrs::Button::East, true))]
        )
    );
}

#[test]
fn given_a_failing_read_when_the_input_runs_then_it_ends_with_that_error_after_the_earlier_events()
{
    let mut terminal = Scripted::new([key_step(KeyCode::Tab), Step::Fail]);
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (run.error, run.sent),
        (
            Some("read failed".to_string()),
            vec![key_sent(KeyCode::Tab)]
        )
    );
}

#[test]
fn given_a_resize_and_a_failing_size_query_when_the_input_runs_then_it_ends_with_an_error_and_sends_nothing()
 {
    let mut terminal = Scripted::new([Step::Event(Event::Resize(10, 10))]);
    terminal.reported_size = None;
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (run.error, run.sent),
        (Some("no size".to_string()), Vec::new())
    );
}

#[test]
fn given_a_resize_event_when_the_input_runs_then_the_emu_gets_the_size_the_terminal_reports() {
    let mut terminal = Scripted::new([Step::Event(Event::Resize(10, 10))]);
    terminal.stop_at = Some(2);
    let run = run_scripted(&mut terminal, Vec::new(), 0, false);
    assert_eq!(
        (run.error, run.sent),
        (None, vec![HostEvent::Resize(size())])
    );
}

/// Runs `finishes_within` on a clock that only `sleep` moves. `done` turns true after
/// `done_after` asks (never, for `None`). Returns the answer and every duration slept.
fn waited(done_after: Option<u32>, limit: Duration) -> (bool, Vec<Duration>) {
    let clock = Cell::new(Instant::now());
    let slept = RefCell::new(Vec::new());
    let asks = Cell::new(0_u32);
    let done = || {
        asks.set(asks.get() + 1);
        done_after.is_some_and(|after| asks.get() > after)
    };
    let sleep = |wait: Duration| {
        slept.borrow_mut().push(wait);
        clock.set(clock.get() + wait);
    };
    let finished = finishes_within(done, limit, || clock.get(), sleep);
    (finished, slept.into_inner())
}

#[test]
fn given_a_done_check_true_at_once_when_waiting_then_it_is_true_and_never_sleeps() {
    assert_eq!(waited(Some(0), INPUT_GRACE), (true, Vec::new()));
}

#[test]
fn given_a_done_check_true_after_three_asks_when_waiting_then_it_is_true_after_three_join_polls() {
    assert_eq!(waited(Some(3), INPUT_GRACE), (true, vec![JOIN_POLL; 3]));
}

#[test]
fn given_a_done_check_never_true_when_waiting_then_it_is_false_after_sleeping_the_limit() {
    let limit = Duration::from_millis(10);
    let (finished, slept) = waited(None, limit);
    assert_eq!(
        (
            finished,
            slept.iter().sum::<Duration>() >= limit,
            slept.iter().all(|wait| *wait == JOIN_POLL)
        ),
        (false, true, true)
    );
}

#[test]
fn given_a_zero_limit_and_a_done_check_false_when_waiting_then_it_is_false_without_sleeping() {
    assert_eq!(waited(None, Duration::ZERO), (false, Vec::new()));
}

#[test]
fn given_a_render_loop_waiting_on_another_thread_when_a_frame_is_published_then_it_is_shown() {
    let handoff = Handoff::new(Snapshot::default);
    let stop = AtomicBool::new(false);
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    let mut gfx = Vec::new();
    let mut renderer = Renderer::new(NoShm, Transfer::Direct { reason: None });
    let mut app = App::new(size());
    app.step();
    let result = std::thread::scope(|scope| {
        let render =
            scope.spawn(|| run_render(&mut renderer, &mut terminal, &mut gfx, &handoff, &stop));
        handoff.publish(|snapshot| app.fill(snapshot));
        drop(StopGuard::new(&stop, &handoff));
        render.join().expect("render thread")
    });
    let text = String::from_utf8_lossy(&gfx).into_owned();
    assert_eq!(
        (result.is_ok(), text.contains("i=1"), text.contains("i=2")),
        (true, true, true)
    );
}
