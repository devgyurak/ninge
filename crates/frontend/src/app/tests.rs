// SPDX-License-Identifier: Apache-2.0
//! Tests of `app`.

use super::*;
use crossterm::event::{KeyEventState, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;

fn size() -> TermSize {
    TermSize {
        cols: 120,
        rows: 50,
        cell_w: 10,
        cell_h: 20,
        px_known: true,
    }
}

fn key(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind,
        state: KeyEventState::NONE,
    })
}

fn press(code: KeyCode) -> Action {
    map_event(&key(code, KeyModifiers::NONE, KeyEventKind::Press))
}

#[test]
fn given_ctrl_c_or_ctrl_q_when_mapping_then_quit() {
    let ctrl = |c| key(KeyCode::Char(c), KeyModifiers::CONTROL, KeyEventKind::Press);
    assert_eq!(
        (map_event(&ctrl('c')), map_event(&ctrl('q'))),
        (Action::Quit, Action::Quit)
    );
}

#[test]
fn given_a_shifted_ctrl_q_when_mapping_then_quit() {
    let e = key(
        KeyCode::Char('Q'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        KeyEventKind::Press,
    );
    assert_eq!(map_event(&e), Action::Quit);
}

#[test]
fn given_q_or_esc_when_mapping_then_they_are_keys_and_do_not_quit() {
    let (q, esc) = (
        key(KeyCode::Char('q'), KeyModifiers::NONE, KeyEventKind::Press),
        key(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Press),
    );
    assert_eq!(
        (map_event(&q), map_event(&esc)),
        (
            Action::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Action::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        )
    );
}

#[test]
fn given_a_plain_c_or_a_ctrl_release_when_mapping_then_it_does_not_quit() {
    let release = key(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    );
    assert_eq!(
        (
            matches!(press(KeyCode::Char('c')), Action::Key(_)),
            matches!(map_event(&release), Action::Key(_))
        ),
        (true, true)
    );
}

#[test]
fn given_a_key_release_when_mapping_then_it_goes_to_the_keys() {
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    assert_eq!(map_event(&Event::Key(release)), Action::Key(release));
}

#[test]
fn given_focus_lost_or_gained_when_mapping_then_only_losing_it_releases_the_keys() {
    assert_eq!(
        (map_event(&Event::FocusLost), map_event(&Event::FocusGained)),
        (Action::ReleaseAll, Action::None)
    );
}

#[test]
fn given_resize_event_when_mapping_then_resized() {
    assert_eq!(map_event(&Event::Resize(10, 10)), Action::Resized);
}

#[test]
fn given_mouse_event_when_mapping_then_none() {
    let e = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(map_event(&e), Action::None);
}

#[test]
fn given_quit_action_when_applied_then_app_should_quit() {
    let mut app = App::new(size());
    app.apply(Action::Quit);
    assert!(app.should_quit());
}

#[test]
fn given_resized_action_when_applied_then_app_keeps_running() {
    let mut app = App::new(size());
    app.apply(Action::Resized);
    assert!(!app.should_quit());
}

#[test]
fn given_frames_when_taking_graphics_then_every_frame_places_both_images_in_order() {
    let mut app = App::new(size());
    let areas = app.areas().expect("fits");
    let place = |id: u32, a: ratatui::layout::Rect| {
        format!(
            "a=T,U=1,c={},r={},f=24,s=256,v=192,i={id},t=d",
            a.width, a.height
        )
    };
    let frames: Vec<String> = (0..3)
        .map(|_| String::from_utf8_lossy(&app.graphics()).into_owned())
        .collect();
    let bad: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            let top = f.find(&place(TOP_IMAGE_ID, areas.top));
            let bottom = f.find(&place(BOTTOM_IMAGE_ID, areas.bottom));
            !top.zip(bottom).is_some_and(|(t, b)| t < b)
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(bad, Vec::<usize>::new());
}

#[test]
fn given_resize_when_taking_graphics_then_placement_uses_the_new_size() {
    let mut app = App::new(size());
    app.resize(TermSize {
        cols: 100,
        rows: 40,
        ..size()
    });
    let a = app.areas().expect("fits");
    let out = String::from_utf8_lossy(&app.graphics()).into_owned();
    assert!(out.contains(&format!("a=T,U=1,c={},r={},", a.top.width, a.top.height)));
}

#[test]
fn given_too_small_terminal_when_taking_graphics_then_nothing_is_sent() {
    let mut app = App::new(TermSize {
        cols: 2,
        rows: 1,
        ..size()
    });
    assert_eq!(app.graphics(), Vec::<u8>::new());
}

#[test]
fn given_frame_step_when_run_then_graphics_reach_the_writer_and_cells_are_drawn() {
    let mut app = App::new(size());
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    let mut gfx = Vec::new();
    frame_step(&mut app, &mut terminal, &mut gfx).expect("step");
    let a = app.areas().expect("fits");
    let cell = terminal.backend().buffer()[(a.top.x, a.top.y)]
        .symbol()
        .to_string();
    assert_eq!(
        (gfx.starts_with(b"\x1b_Ga=T,U=1"), cell),
        (true, "\u{10EEEE}\u{305}\u{305}".to_string())
    );
}

#[test]
fn given_time_left_when_choosing_poll_timeout_then_it_is_the_remainder() {
    let now = Instant::now();
    assert_eq!(
        poll_timeout(now + Duration::from_millis(10), now),
        Duration::from_millis(10)
    );
}

#[test]
fn given_deadline_passed_when_choosing_poll_timeout_then_it_is_zero() {
    let now = Instant::now();
    assert_eq!(
        poll_timeout(now, now + Duration::from_millis(20)),
        Duration::ZERO
    );
}

#[test]
fn given_too_small_terminal_when_drawing_then_it_says_so() {
    let mut app = App::new(TermSize {
        cols: 20,
        rows: 1,
        ..size()
    });
    let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test backend");
    frame_step(&mut app, &mut terminal, &mut Vec::new()).expect("step");
    let row: String = (0..18)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol().to_string())
        .collect();
    assert_eq!(row, "terminal too small");
}

#[test]
fn given_unknown_cell_size_when_drawing_then_registers_panel_warns() {
    let mut app = App::new(TermSize {
        px_known: false,
        ..size()
    });
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    frame_step(&mut app, &mut terminal, &mut Vec::new()).expect("step");
    let text = format!("{}", terminal.backend());
    assert!(text.contains("cell size unknown"));
}

/// Hands out `/fake-<n>` names, or fails every put.
struct FakeSink {
    puts: Vec<Vec<u8>>,
    fail: bool,
}

impl ShmSink for FakeSink {
    fn put(&mut self, data: &[u8]) -> std::io::Result<String> {
        self.puts.push(data.to_vec());
        if self.fail {
            Err(std::io::Error::other("no space left"))
        } else {
            Ok(format!("/fake-{}", self.puts.len()))
        }
    }
}

fn shm_app(fail: bool) -> App<FakeSink> {
    let sink = FakeSink {
        puts: Vec::new(),
        fail,
    };
    App::with_transfer(size(), sink, Transfer::Shm)
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn given_shm_transfer_when_taking_graphics_then_both_screens_go_as_object_names() {
    let mut app = shm_app(false);
    let out = text(app.graphics());
    assert_eq!(
        (
            out.matches("t=s,S=147456,q=2").count(),
            out.contains("t=d"),
            app.sink.puts.iter().map(Vec::len).collect::<Vec<_>>()
        ),
        (2, false, vec![256 * 192 * 3, 256 * 192 * 3])
    );
}

#[test]
fn given_shm_transfer_when_taking_graphics_then_payloads_encode_the_names_in_screen_order() {
    let mut app = shm_app(false);
    let out = text(app.graphics());
    let b64 = |name: &str| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, name);
    let top = out.find(&format!(";{}\x1b\\", b64("/fake-1")));
    let bottom = out.find(&format!(";{}\x1b\\", b64("/fake-2")));
    assert!(top.zip(bottom).is_some_and(|(t, b)| t < b));
}

#[test]
fn given_failing_shm_write_when_taking_graphics_then_the_frame_falls_back_to_zlib() {
    let mut app = shm_app(true);
    let out = text(app.graphics());
    assert_eq!(
        (out.contains("t=s"), out.matches("t=d,o=z").count()),
        (false, 2)
    );
}

#[test]
fn given_failing_shm_write_when_taking_more_frames_then_shm_is_not_tried_again() {
    let mut app = shm_app(true);
    let _ = app.graphics();
    let _ = app.graphics();
    assert_eq!(app.sink.puts.len(), 1);
}

#[test]
fn given_failing_shm_write_when_drawing_then_the_panel_states_the_fallback_reason() {
    let mut app = shm_app(true);
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    frame_step(&mut app, &mut terminal, &mut Vec::new()).expect("step");
    let text = format!("{}", terminal.backend());
    assert!(text.contains("transfer: zlib (t=d)") && text.contains("shm write failed:"));
}

#[test]
fn given_working_shm_when_drawing_then_the_panel_says_shm() {
    let mut app = shm_app(false);
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    frame_step(&mut app, &mut terminal, &mut Vec::new()).expect("step");
    let text = format!("{}", terminal.backend());
    assert!(text.contains("transfer: shm (t=s)"));
}

#[test]
fn given_default_app_when_asking_for_transfer_then_it_is_direct_with_no_reason() {
    let app = App::new(size());
    assert_eq!(*app.transfer(), Transfer::Direct { reason: None });
}

// ------------------------------------------------------------------------------- save file

fn app_with_saved_machine(name: &str) -> (App, std::path::PathBuf) {
    let rom = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds"),
    )
    .expect("smoke ROM is built");
    let mut machine = crate::source::boot(rom).expect("valid header");
    machine
        .insert_save(ninge_core::cart::SaveType::Eeprom512, vec![0x21; 512])
        .expect("512 bytes fit");
    let path = std::env::temp_dir().join(format!("ninge-app-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let save = SaveFile::new(path.clone(), &machine);
    let app = App::new(size())
        .with_source(Source::Machine(Box::new(machine)))
        .with_save(Some(save));
    (app, path)
}

/// `n` frames of the app.
fn run_frames(app: &mut App, n: u32) {
    (0..n).for_each(|_| {
        app.graphics();
    });
}

#[test]
fn given_the_dummy_pattern_when_the_app_finishes_then_it_has_nothing_to_write() {
    assert!(App::new(size()).finish().is_ok());
}

#[test]
fn given_a_machine_whose_save_did_not_change_when_frames_run_and_the_app_finishes_then_no_file_is_written()
 {
    let (mut app, path) = app_with_saved_machine("unchanged.bin");
    run_frames(&mut app, crate::savefile::QUIET_FRAMES + 5);
    assert_eq!((app.finish().is_ok(), path.exists()), (true, false));
}

#[test]
fn given_a_machine_with_a_save_when_the_note_is_asked_then_it_names_the_chip() {
    let (app, path) = app_with_saved_machine("note.bin");
    assert_eq!(
        app.save_note(),
        Some(format!("save chip eeprom-512, file {}", path.display()))
    );
}

#[test]
fn given_the_dummy_pattern_when_the_note_is_asked_then_there_is_none() {
    assert_eq!(App::new(size()).save_note(), None);
}

#[test]
fn given_a_machine_and_a_clock_when_a_frame_runs_then_the_machine_got_the_clock_s_time() {
    let (app, path) = app_with_saved_machine("clock.bin");
    let mut app = app.with_clock(|| 123_456);
    run_frames(&mut app, 1);
    let latched = app.machine().map(|m| m.inspect().input().wall_time);
    let _ = std::fs::remove_file(path);
    assert_eq!(latched, Some(123_456));
}

// ------------------------------------------------------------- shutdown and unlit screens

/// An app running the poweroff ROM: its ARM7 shuts the console down (backlights off, bit 6).
fn poweroff_app() -> App {
    let rom = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testroms/build/poweroff.nds"),
    )
    .expect("poweroff ROM is built");
    let machine = crate::source::boot(rom).expect("valid header");
    App::new(size()).with_source(Source::Machine(Box::new(machine)))
}

#[test]
fn given_a_lit_screen_when_choosing_the_pixels_then_they_are_unchanged() {
    assert_eq!(
        lit_or_black(vec![1, 2, 0x00FF_FFFF], true),
        vec![1, 2, 0x00FF_FFFF]
    );
}

#[test]
fn given_an_unlit_screen_when_choosing_the_pixels_then_every_pixel_is_black_and_the_size_is_kept() {
    assert_eq!(lit_or_black(vec![1, 2, 0x00FF_FFFF], false), vec![0, 0, 0]);
}

#[test]
fn given_the_dummy_pattern_when_frames_run_then_the_app_does_not_quit_by_itself() {
    let mut app = App::new(size());
    run_frames(&mut app, 3);
    assert!(!app.should_quit());
}

#[test]
fn given_a_program_that_has_not_shut_down_when_a_frame_runs_then_the_app_does_not_quit() {
    let (mut app, path) = app_with_saved_machine("running.bin");
    run_frames(&mut app, 2);
    let _ = std::fs::remove_file(path);
    assert!(!app.should_quit());
}

#[test]
fn given_a_program_that_shuts_the_console_down_when_a_frame_runs_then_the_app_quits() {
    let mut app = poweroff_app();
    run_frames(&mut app, 1);
    assert!(app.should_quit());
}

/// An app running the poweroff ROM with a save file, whose save memory counts as changed (a
/// savestate load raises the generation), so a `finish` has something to write.
fn poweroff_app_with_changed_save(name: &str) -> (App, std::path::PathBuf) {
    let rom = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testroms/build/poweroff.nds"),
    )
    .expect("poweroff ROM is built");
    let mut machine = crate::source::boot(rom).expect("valid header");
    machine
        .insert_save(ninge_core::cart::SaveType::Eeprom512, vec![0x21; 512])
        .expect("512 bytes fit");
    let path = std::env::temp_dir().join(format!("ninge-app-{}-{name}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let save = SaveFile::new(path.clone(), &machine);
    let state = machine.save_state();
    machine.load_state(&state).expect("own state loads");
    let app = App::new(size())
        .with_source(Source::Machine(Box::new(machine)))
        .with_save(Some(save));
    (app, path)
}

#[test]
fn given_a_shut_down_machine_with_changed_save_memory_when_the_app_finishes_then_the_save_file_holds_it()
 {
    let (mut app, path) = poweroff_app_with_changed_save("poweroff.sav");
    run_frames(&mut app, 1);
    let quit = app.should_quit();
    let finished = app.finish();
    let written = std::fs::read(&path).ok();
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        (quit, finished.is_ok(), written),
        (true, true, Some(vec![0x21; 512]))
    );
}

#[test]
fn given_a_shut_down_machine_with_unlit_screens_when_graphics_are_taken_then_both_images_are_still_placed()
 {
    let mut app = poweroff_app();
    run_frames(&mut app, 1);
    let out = text(app.graphics());
    assert_eq!((out.contains("i=1"), out.contains("i=2")), (true, true));
}

/// How many pixels of each screen the app would send that are not black.
fn lit_pixel_counts(app: &App) -> (usize, usize) {
    let (top, bottom) = app.screens();
    (
        top.iter().filter(|&&p| p != 0).count(),
        bottom.iter().filter(|&&p| p != 0).count(),
    )
}

#[test]
fn given_a_machine_with_both_backlights_off_when_the_screens_are_taken_then_no_pixel_is_lit() {
    let mut app = poweroff_app();
    run_frames(&mut app, 1);
    assert_eq!(lit_pixel_counts(&app), (0, 0));
}

// ----------------------------------------------------------------------------------- keys

/// An app running the smoke ROM, on a terminal that reports key release or not.
fn smoke_app(release_reported: bool) -> App {
    let rom = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds"),
    )
    .expect("smoke ROM is built");
    let machine = crate::source::boot(rom).expect("valid header");
    App::new(size())
        .with_key_release(release_reported)
        .with_source(Source::Machine(Box::new(machine)))
}

fn key_action(code: KeyCode, kind: KeyEventKind) -> Action {
    map_event(&key(code, KeyModifiers::NONE, kind))
}

#[test]
fn given_a_held_key_when_a_frame_runs_then_the_machine_gets_its_button() {
    let mut app = smoke_app(true);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    app.graphics();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::A);
}

#[test]
fn given_a_released_key_when_a_frame_runs_then_the_machine_gets_no_button() {
    let mut app = smoke_app(true);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Release));
    app.graphics();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::empty());
}

#[test]
fn given_focus_lost_when_a_frame_runs_then_the_machine_gets_no_button() {
    let mut app = smoke_app(true);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    app.apply(Action::ReleaseAll);
    app.graphics();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::empty());
}

#[test]
fn given_the_microphone_and_lid_keys_when_a_frame_runs_then_the_machine_gets_them() {
    let mut app = smoke_app(true);
    app.apply(key_action(KeyCode::Char('m'), KeyEventKind::Press));
    app.apply(key_action(KeyCode::Char('`'), KeyEventKind::Press));
    app.graphics();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!((input.mic, input.hinge_closed), (true, true));
}

#[test]
fn given_a_press_without_release_reports_when_frames_run_then_the_button_lasts_the_fixed_count() {
    let mut app = smoke_app(false);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    let held: Vec<bool> = (0..=crate::input::FALLBACK_HOLD_FRAMES)
        .map(|_| {
            app.graphics();
            app.machine().expect("a machine").inspect().input().buttons == ninge_core::Buttons::A
        })
        .collect();
    let mut expected = vec![true; usize::from(crate::input::FALLBACK_HOLD_FRAMES)];
    expected.push(false);
    assert_eq!(held, expected);
}

/// The text of the screen after one frame of `app`.
fn panel_text(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    frame_step(app, &mut terminal, &mut Vec::new()).expect("step");
    format!("{}", terminal.backend())
}

#[test]
fn given_a_terminal_without_key_release_when_drawing_then_the_panel_says_so() {
    let text = panel_text(&mut App::new(size()).with_key_release(false));
    assert!(text.contains("key release not reported"));
}

#[test]
fn given_a_terminal_with_key_release_when_drawing_then_the_panel_does_not_warn() {
    let text = panel_text(&mut App::new(size()).with_key_release(true));
    assert!(!text.contains("key release not reported"));
}
