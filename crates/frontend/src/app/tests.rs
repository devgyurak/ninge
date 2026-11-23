// SPDX-License-Identifier: Apache-2.0
//! Tests of `app`.

use super::*;
use crate::render::{Renderer, Snapshot};
use crate::transfer::{NoShm, Transfer};
use crossterm::event::{KeyEventState, MouseButton, MouseEvent, MouseEventKind};

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
fn given_mouse_event_when_mapping_then_it_goes_to_the_pen() {
    let e = MouseEvent {
        kind: MouseEventKind::Moved,
        column: 3,
        row: 4,
        modifiers: KeyModifiers::NONE,
    };
    assert_eq!(map_event(&Event::Mouse(e)), Action::Mouse(e));
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
    (0..n).for_each(|_| app.step());
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
    let mut shown = Vec::new();
    show_pixels(&mut shown, &[1, 2, 0x00FF_FFFF], true);
    assert_eq!(shown, vec![1, 2, 0x00FF_FFFF]);
}

#[test]
fn given_an_unlit_screen_when_choosing_the_pixels_then_every_pixel_is_black_and_the_size_is_kept() {
    let mut shown = vec![9; 7];
    show_pixels(&mut shown, &[1, 2, 0x00FF_FFFF], false);
    assert_eq!(shown, vec![0, 0, 0]);
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
    let mut snapshot = Snapshot::default();
    app.fill(&mut snapshot);
    let mut renderer = Renderer::new(NoShm, Transfer::Direct { reason: None });
    let out = String::from_utf8_lossy(&renderer.graphics(&snapshot)).into_owned();
    assert_eq!((out.contains("i=1"), out.contains("i=2")), (true, true));
}

/// How many pixels of each screen the app would send that are not black.
fn lit_pixel_counts(app: &App) -> (usize, usize) {
    let mut shown = Snapshot::default();
    app.fill(&mut shown);
    (
        shown.top.iter().filter(|&&p| p != 0).count(),
        shown.bottom.iter().filter(|&&p| p != 0).count(),
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
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::A);
}

#[test]
fn given_a_released_key_when_a_frame_runs_then_the_machine_gets_no_button() {
    let mut app = smoke_app(true);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Release));
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::empty());
}

#[test]
fn given_focus_lost_when_a_frame_runs_then_the_machine_gets_no_button() {
    let mut app = smoke_app(true);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    app.apply(Action::ReleaseAll);
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::empty());
}

#[test]
fn given_the_microphone_and_lid_keys_when_a_frame_runs_then_the_machine_gets_them() {
    let mut app = smoke_app(true);
    app.apply(key_action(KeyCode::Char('m'), KeyEventKind::Press));
    app.apply(key_action(KeyCode::Char('`'), KeyEventKind::Press));
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!((input.mic, input.hinge_closed), (true, true));
}

#[test]
fn given_a_press_without_release_reports_when_frames_run_then_the_button_lasts_the_fixed_count() {
    let mut app = smoke_app(false);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    let held: Vec<bool> = (0..=crate::input::FALLBACK_HOLD_FRAMES)
        .map(|_| {
            app.step();
            app.machine().expect("a machine").inspect().input().buttons == ninge_core::Buttons::A
        })
        .collect();
    let mut expected = vec![true; usize::from(crate::input::FALLBACK_HOLD_FRAMES)];
    expected.push(false);
    assert_eq!(held, expected);
}

// ---------------------------------------------------------------------------------- mouse

fn mouse_at(kind: MouseEventKind, column: u16, row: u16) -> Action {
    Action::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// The cell at the middle of the bottom screen of `app`.
fn bottom_centre(app: &App) -> (u16, u16) {
    let b = app.areas().expect("fits").bottom;
    (b.x + b.width / 2, b.y + b.height / 2)
}

/// The panel point the machine was given on the last frame.
fn machine_touch(app: &App) -> Option<ninge_core::Touch> {
    app.machine().expect("a machine").inspect().input().touch
}

#[test]
fn given_a_left_press_on_the_bottom_screen_when_a_frame_runs_then_the_machine_is_touched() {
    let mut app = smoke_app(true);
    let (c, r) = bottom_centre(&app);
    app.apply(mouse_at(MouseEventKind::Down(MouseButton::Left), c, r));
    app.step();
    assert!(machine_touch(&app).is_some());
}

#[test]
fn given_a_left_press_on_the_top_screen_when_a_frame_runs_then_the_machine_is_not_touched() {
    let mut app = smoke_app(true);
    let top = app.areas().expect("fits").top;
    app.apply(mouse_at(
        MouseEventKind::Down(MouseButton::Left),
        top.x,
        top.y,
    ));
    app.step();
    assert_eq!(machine_touch(&app), None);
}

#[test]
fn given_a_release_when_a_frame_runs_then_the_machine_is_not_touched() {
    let mut app = smoke_app(true);
    let (c, r) = bottom_centre(&app);
    app.apply(mouse_at(MouseEventKind::Down(MouseButton::Left), c, r));
    app.apply(mouse_at(MouseEventKind::Up(MouseButton::Left), c, r));
    app.step();
    assert_eq!(machine_touch(&app), None);
}

#[test]
fn given_focus_lost_or_a_resize_when_a_frame_runs_then_the_pen_is_up() {
    let touched_after = |action: fn(&mut App)| {
        let mut app = smoke_app(true);
        let (c, r) = bottom_centre(&app);
        app.apply(mouse_at(MouseEventKind::Down(MouseButton::Left), c, r));
        action(&mut app);
        app.step();
        machine_touch(&app)
    };
    assert_eq!(
        (
            touched_after(|app| app.apply(Action::ReleaseAll)),
            touched_after(|app| app.resize(size()))
        ),
        (None, None)
    );
}

#[test]
fn given_the_cell_unit_when_the_first_cell_is_pressed_then_the_point_is_the_cell_centre() {
    let mut app = smoke_app(true).with_mouse_unit(Unit::Cells);
    let b = app.areas().expect("fits").bottom;
    app.apply(mouse_at(MouseEventKind::Down(MouseButton::Left), b.x, b.y));
    app.step();
    let want = crate::pen::touch_at(
        b.x,
        b.y,
        &Target {
            unit: Unit::Cells,
            bottom: b,
            cell: (size().cell_w, size().cell_h),
        },
    );
    assert_eq!(machine_touch(&app), want);
}

#[test]
fn given_a_terminal_too_small_when_a_mouse_report_arrives_then_the_pen_stays_up() {
    let mut app = App::new(TermSize {
        cols: 2,
        rows: 1,
        ..size()
    });
    app.apply(mouse_at(MouseEventKind::Down(MouseButton::Left), 0, 0));
    assert_eq!(app.pen.touch(), None);
}

// -------------------------------------------------------------------------------- cursor

fn char_key(c: char, kind: KeyEventKind) -> Action {
    key_action(KeyCode::Char(c), kind)
}

fn tab_key() -> Action {
    key_action(KeyCode::Tab, KeyEventKind::Press)
}

#[test]
fn given_cursor_mode_and_space_when_a_frame_runs_then_the_machine_is_touched_at_the_centre() {
    let mut app = smoke_app(true);
    app.apply(tab_key());
    app.apply(char_key(' ', KeyEventKind::Press));
    app.step();
    assert_eq!(
        machine_touch(&app),
        Some(ninge_core::Touch { x: 128, y: 96 })
    );
}

#[test]
fn given_cursor_mode_off_when_space_is_pressed_then_the_machine_is_not_touched() {
    let mut app = smoke_app(true);
    app.apply(char_key(' ', KeyEventKind::Press));
    app.step();
    assert_eq!(machine_touch(&app), None);
}

#[test]
fn given_the_mouse_pen_down_and_the_cursor_touching_when_a_frame_runs_then_the_mouse_wins() {
    let mut app = smoke_app(true);
    app.apply(tab_key());
    app.apply(char_key(' ', KeyEventKind::Press));
    let b = app.areas().expect("fits").bottom;
    app.apply(mouse_at(MouseEventKind::Down(MouseButton::Left), b.x, b.y));
    app.step();
    assert_ne!(
        machine_touch(&app),
        Some(ninge_core::Touch { x: 128, y: 96 })
    );
}

#[test]
fn given_the_mouse_pen_up_and_the_cursor_touching_when_a_frame_runs_then_the_cursor_touches() {
    let mut app = smoke_app(true);
    app.apply(tab_key());
    app.apply(char_key(' ', KeyEventKind::Press));
    let b = app.areas().expect("fits").bottom;
    app.apply(mouse_at(MouseEventKind::Down(MouseButton::Left), b.x, b.y));
    app.apply(mouse_at(MouseEventKind::Up(MouseButton::Left), b.x, b.y));
    app.step();
    assert_eq!(
        machine_touch(&app),
        Some(ninge_core::Touch { x: 128, y: 96 })
    );
}

#[test]
fn given_a_game_key_in_cursor_mode_when_a_frame_runs_then_the_button_is_still_held() {
    let mut app = smoke_app(true);
    app.apply(tab_key());
    app.apply(char_key('x', KeyEventKind::Press));
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::A);
}

/// The bottom image's pixels of the pattern as the app sends them, with or without the cursor:
/// the number of pixels that differ.
fn crosshair_pixels(app: &mut App) -> usize {
    let plain = dummy_frame(0, 1);
    let mut shown = Snapshot::default();
    app.fill(&mut shown);
    plain
        .iter()
        .zip(&shown.bottom)
        .filter(|(a, b)| a != b)
        .count()
}

#[test]
fn given_cursor_mode_off_when_the_bottom_screen_is_made_then_no_pixel_changes() {
    assert_eq!(crosshair_pixels(&mut App::new(size())), 0);
}

#[test]
fn given_cursor_mode_on_when_the_bottom_screen_is_made_then_the_crosshair_is_drawn() {
    let mut app = App::new(size());
    app.apply(tab_key());
    assert_eq!(crosshair_pixels(&mut app), 13);
}

#[test]
fn given_cursor_mode_on_when_the_top_screen_is_made_then_it_is_unchanged() {
    let mut app = App::new(size());
    app.apply(tab_key());
    let mut shown = Snapshot::default();
    app.fill(&mut shown);
    assert_eq!(shown.top, dummy_frame(0, 0));
}

// -------------------------------------------------------------------------------- gamepad

#[test]
fn given_a_pad_button_and_a_key_when_a_frame_runs_then_the_machine_gets_both_or_ed() {
    let mut app = smoke_app(true).with_gamepads(true, 1);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    app.apply_pad(PadEvent::Button(gilrs::Button::South, true));
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(
        input.buttons,
        ninge_core::Buttons::A | ninge_core::Buttons::B
    );
}

#[test]
fn given_the_same_button_on_pad_and_keyboard_when_the_key_is_released_then_the_pad_still_holds_it()
{
    let mut app = smoke_app(true).with_gamepads(true, 1);
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Press));
    app.apply_pad(PadEvent::Button(gilrs::Button::East, true));
    app.apply(key_action(KeyCode::Char('x'), KeyEventKind::Release));
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::A);
}

#[test]
fn given_a_held_pad_button_when_focus_is_lost_then_the_machine_gets_no_button() {
    let mut app = smoke_app(true).with_gamepads(true, 1);
    app.apply_pad(PadEvent::Button(gilrs::Button::East, true));
    app.apply(Action::ReleaseAll);
    app.step();
    let input = app.machine().expect("a machine").inspect().input();
    assert_eq!(input.buttons, ninge_core::Buttons::empty());
}
