// SPDX-License-Identifier: Apache-2.0
//! Tests of `render`.

use super::*;
use crate::app::{Action, App};
use crate::layout::TermSize;
use crate::source::Source;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

/// The renderer of a default run: zlib, no reason.
fn direct() -> Renderer<NoShm> {
    Renderer::new(NoShm, Transfer::Direct { reason: None })
}

/// One frame of `app`, as the renderer is given it.
fn snapshot_of(app: &mut App) -> Snapshot {
    app.step();
    let mut snapshot = Snapshot::default();
    app.fill(&mut snapshot);
    snapshot
}

/// The graphics bytes of one more frame of `app`.
fn graphics_of<S: ShmSink>(app: &mut App, renderer: &mut Renderer<S>) -> Vec<u8> {
    renderer.graphics(&snapshot_of(app))
}

/// The text of the screen after one frame of `app`.
fn panel_text<S: ShmSink>(app: &mut App, renderer: &mut Renderer<S>) -> String {
    let snapshot = snapshot_of(app);
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    renderer
        .present(&snapshot, &mut terminal, &mut Vec::new())
        .expect("step");
    format!("{}", terminal.backend())
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn given_frames_when_taking_graphics_then_every_frame_places_both_images_in_order() {
    let mut app = App::new(size());
    let mut renderer = direct();
    let areas = app.areas().expect("fits");
    let place = |id: u32, a: ratatui::layout::Rect| {
        format!(
            "a=T,U=1,c={},r={},f=24,s=256,v=192,i={id},t=d",
            a.width, a.height
        )
    };
    let frames: Vec<String> = (0..3)
        .map(|_| text(graphics_of(&mut app, &mut renderer)))
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
    let out = text(graphics_of(&mut app, &mut direct()));
    assert!(out.contains(&format!("a=T,U=1,c={},r={},", a.top.width, a.top.height)));
}

#[test]
fn given_too_small_terminal_when_taking_graphics_then_nothing_is_sent() {
    let mut app = App::new(TermSize {
        cols: 2,
        rows: 1,
        ..size()
    });
    assert_eq!(graphics_of(&mut app, &mut direct()), Vec::<u8>::new());
}

#[test]
fn given_a_too_small_terminal_when_a_snapshot_is_filled_then_it_holds_no_pixels() {
    let mut app = App::new(TermSize {
        cols: 2,
        rows: 1,
        ..size()
    });
    let snapshot = snapshot_of(&mut app);
    assert_eq!(
        (snapshot.areas, snapshot.top.len(), snapshot.bottom.len()),
        (None, 0, 0)
    );
}

#[test]
fn given_present_when_run_then_graphics_reach_the_writer_and_cells_are_drawn() {
    let mut app = App::new(size());
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).expect("test backend");
    let mut gfx = Vec::new();
    let snapshot = snapshot_of(&mut app);
    direct()
        .present(&snapshot, &mut terminal, &mut gfx)
        .expect("step");
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
fn given_too_small_terminal_when_drawing_then_it_says_so() {
    let mut app = App::new(TermSize {
        cols: 20,
        rows: 1,
        ..size()
    });
    let snapshot = snapshot_of(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test backend");
    direct()
        .present(&snapshot, &mut terminal, &mut Vec::new())
        .expect("step");
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
    assert!(panel_text(&mut app, &mut direct()).contains("cell size unknown"));
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

fn shm_renderer(fail: bool) -> Renderer<FakeSink> {
    let sink = FakeSink {
        puts: Vec::new(),
        fail,
    };
    Renderer::new(sink, Transfer::Shm)
}

#[test]
fn given_shm_transfer_when_taking_graphics_then_both_screens_go_as_object_names() {
    let mut renderer = shm_renderer(false);
    let out = text(graphics_of(&mut App::new(size()), &mut renderer));
    assert_eq!(
        (
            out.matches("t=s,S=147456,q=2").count(),
            out.contains("t=d"),
            renderer.sink.puts.iter().map(Vec::len).collect::<Vec<_>>()
        ),
        (2, false, vec![256 * 192 * 3, 256 * 192 * 3])
    );
}

#[test]
fn given_shm_transfer_when_taking_graphics_then_payloads_encode_the_names_in_screen_order() {
    let out = text(graphics_of(&mut App::new(size()), &mut shm_renderer(false)));
    let b64 = |name: &str| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, name);
    let top = out.find(&format!(";{}\x1b\\", b64("/fake-1")));
    let bottom = out.find(&format!(";{}\x1b\\", b64("/fake-2")));
    assert!(top.zip(bottom).is_some_and(|(t, b)| t < b));
}

#[test]
fn given_failing_shm_write_when_taking_graphics_then_the_frame_falls_back_to_zlib() {
    let out = text(graphics_of(&mut App::new(size()), &mut shm_renderer(true)));
    assert_eq!(
        (out.contains("t=s"), out.matches("t=d,o=z").count()),
        (false, 2)
    );
}

#[test]
fn given_failing_shm_write_when_taking_more_frames_then_shm_is_not_tried_again() {
    let mut app = App::new(size());
    let mut renderer = shm_renderer(true);
    let _ = graphics_of(&mut app, &mut renderer);
    let _ = graphics_of(&mut app, &mut renderer);
    assert_eq!(renderer.sink.puts.len(), 1);
}

#[test]
fn given_failing_shm_write_when_drawing_then_the_panel_states_the_fallback_reason() {
    let text = panel_text(&mut App::new(size()), &mut shm_renderer(true));
    assert!(text.contains("transfer: zlib (t=d)") && text.contains("shm write failed:"));
}

#[test]
fn given_working_shm_when_drawing_then_the_panel_says_shm() {
    let text = panel_text(&mut App::new(size()), &mut shm_renderer(false));
    assert!(text.contains("transfer: shm (t=s)"));
}

#[test]
fn given_default_renderer_when_asking_for_transfer_then_it_is_direct_with_no_reason() {
    assert_eq!(*direct().transfer(), Transfer::Direct { reason: None });
}

// ------------------------------------------------------------------------------------ panels

fn panel_of(mut app: App) -> String {
    panel_text(&mut app, &mut direct())
}

#[test]
fn given_a_terminal_without_key_release_when_drawing_then_the_panel_says_so() {
    let text = panel_of(App::new(size()).with_key_release(false));
    assert!(text.contains("key release not reported"));
}

#[test]
fn given_a_terminal_with_key_release_when_drawing_then_the_panel_does_not_warn() {
    let text = panel_of(App::new(size()).with_key_release(true));
    assert!(!text.contains("key release not reported"));
}

#[test]
fn given_cursor_mode_when_drawing_then_the_panel_shows_the_position() {
    let mut app = App::new(size());
    app.apply(Action::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
    assert!(panel_of(app).contains("cursor mode: 128,96"));
}

#[test]
fn given_the_gamepad_library_failed_when_drawing_then_the_panel_says_it_is_unavailable() {
    let app = App::new(size()).with_gamepads(false, 0);
    assert!(panel_of(app).contains("gamepad unavailable"));
}

#[test]
fn given_no_pad_plugged_in_when_drawing_then_the_panel_says_none_is_connected() {
    let app = App::new(size()).with_gamepads(true, 0);
    assert!(panel_of(app).contains("no gamepad connected"));
}

#[test]
fn given_a_pad_plugged_in_when_drawing_then_the_panel_says_it_is_connected() {
    let app = App::new(size()).with_gamepads(true, 1);
    assert!(panel_of(app).contains("gamepad connected"));
}

#[test]
fn given_a_pad_plugged_in_later_when_the_count_is_updated_then_the_panel_says_it_is_connected() {
    let mut app = App::new(size()).with_gamepads(true, 0);
    app.set_pads_connected(1);
    assert!(panel_of(app).contains("gamepad connected"));
}

#[test]
fn given_a_machine_when_a_snapshot_is_filled_again_then_its_buffers_are_reused() {
    let rom = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds"),
    )
    .expect("smoke ROM is built");
    let machine = crate::source::boot(rom).expect("valid header");
    let mut app = App::new(size()).with_source(Source::Machine(Box::new(machine)));
    let mut snapshot = snapshot_of(&mut app);
    let before = snapshot.top.as_ptr();
    app.step();
    app.fill(&mut snapshot);
    assert_eq!(snapshot.top.as_ptr(), before);
}
