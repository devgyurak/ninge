// SPDX-License-Identifier: Apache-2.0
//! Application state, input mapping, and one frame of output.

use crate::kitty;
use crate::layout::{Areas, TermSize, compute};
use crate::pattern::dummy_frame;
use crate::placeholder::ScreenImage;
use crate::transfer::{NoShm, ShmSink, Transfer};
use anyhow::anyhow;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use std::io::Write;
use std::time::{Duration, Instant};

/// Image id of the top screen.
pub const TOP_IMAGE_ID: u32 = 1;
/// Image id of the bottom screen.
pub const BOTTOM_IMAGE_ID: u32 = 2;

/// What an input event asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing to do.
    None,
    /// Leave the program.
    Quit,
    /// The terminal changed size; the caller re-queries it and calls [`App::resize`].
    Resized,
}

/// Maps a terminal event to an [`Action`]. `q`, Esc, and Ctrl-C quit (raw mode delivers
/// Ctrl-C as a key).
pub fn map_event(event: &Event) -> Action {
    match event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) => {
            let ctrl_c = *code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL);
            if ctrl_c || matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
                Action::Quit
            } else {
                Action::None
            }
        }
        Event::Resize(..) => Action::Resized,
        _ => Action::None,
    }
}

/// Frontend state. `S` is where shared-memory frames are written.
pub struct App<S = NoShm> {
    size: TermSize,
    areas: Option<Areas>,
    frame_no: u64,
    quit: bool,
    transfer: Transfer,
    sink: S,
}

impl App<NoShm> {
    /// A new app for a terminal of `size` that sends frames as zlib.
    pub fn new(size: TermSize) -> Self {
        Self::with_transfer(size, NoShm, Transfer::Direct { reason: None })
    }
}

impl<S: ShmSink> App<S> {
    /// A new app that sends frames the `transfer` way, writing shared-memory frames
    /// through `sink`.
    pub fn with_transfer(size: TermSize, sink: S, transfer: Transfer) -> Self {
        Self {
            size,
            areas: compute(size),
            frame_no: 0,
            quit: false,
            transfer,
            sink,
        }
    }

    /// The medium frames go over now; it turns to zlib for good if a shared-memory write
    /// fails.
    pub fn transfer(&self) -> &Transfer {
        &self.transfer
    }

    /// Records that the terminal is now `size`; the next frame places the images anew.
    pub fn resize(&mut self, size: TermSize) {
        self.size = size;
        self.areas = compute(size);
    }

    /// Applies an [`Action`]; `Resized` is handled by the caller through [`App::resize`].
    pub fn apply(&mut self, action: Action) {
        self.quit |= action == Action::Quit;
    }

    /// Whether the user asked to quit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// The current layout, `None` when the terminal is too small.
    pub fn areas(&self) -> Option<Areas> {
        self.areas
    }

    /// The graphics bytes for this frame: each screen's pixels with its virtual
    /// placement. Re-transmitting an image deletes its placements, so every frame
    /// places both again. Advances the frame counter.
    pub fn graphics(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(areas) = self.areas {
            let top = dummy_frame(self.frame_no, 0);
            let bottom = dummy_frame(self.frame_no, 1);
            out.extend(self.send(TOP_IMAGE_ID, &top, areas.top));
            out.extend(self.send(BOTTOM_IMAGE_ID, &bottom, areas.bottom));
        }
        self.frame_no += 1;
        out
    }

    /// One screen over the current medium. A failed shared-memory write is not retried:
    /// this frame and every later one go as zlib, and the panel says why.
    fn send(&mut self, image_id: u32, pixels: &[u32], area: Rect) -> Vec<u8> {
        if self.transfer == Transfer::Shm {
            match self.sink.put(&kitty::rgb24(pixels)) {
                Ok(name) => {
                    return kitty::transmit_shm(image_id, &name, area.width, area.height);
                }
                Err(e) => self.transfer = Transfer::direct(format!("shm write failed: {e}")),
            }
        }
        kitty::transmit(image_id, pixels, area.width, area.height)
    }

    /// Draws the screens and panels.
    pub fn draw(&self, frame: &mut Frame) {
        let Some(areas) = self.areas else {
            frame.render_widget(Paragraph::new("terminal too small"), frame.area());
            return;
        };
        frame.render_widget(
            ScreenImage {
                image_id: TOP_IMAGE_ID,
            },
            areas.top,
        );
        frame.render_widget(
            ScreenImage {
                image_id: BOTTOM_IMAGE_ID,
            },
            areas.bottom,
        );
        let note = if self.size.px_known {
            ""
        } else {
            "\ncell size unknown, assuming 8x16"
        };
        // TODO(phase1): real register and disassembly panels (#56).
        frame.render_widget(
            Paragraph::new(format!(
                "TODO(phase1) (#56){note}\n{}",
                self.transfer.label()
            ))
            .wrap(Wrap { trim: false })
            .block(Block::new().borders(Borders::ALL).title("Registers")),
            areas.registers,
        );
        frame.render_widget(
            Paragraph::new("TODO(phase1) (#56)")
                .block(Block::new().borders(Borders::ALL).title("Disassembly")),
            areas.disasm,
        );
    }
}

/// How long to wait for input: the time left until `deadline`, or zero once it has
/// passed. Zero still polls, so pending keys and resizes are read every frame even when
/// drawing overran the frame time.
pub fn poll_timeout(deadline: Instant, now: Instant) -> Duration {
    deadline.saturating_duration_since(now)
}

/// One tick: write the graphics to `gfx`, then draw with ratatui.
pub fn frame_step<S: ShmSink, B: Backend, W: Write>(
    app: &mut App<S>,
    terminal: &mut Terminal<B>,
    gfx: &mut W,
) -> anyhow::Result<()> {
    gfx.write_all(&app.graphics())?;
    gfx.flush()?;
    terminal.draw(|f| app.draw(f)).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
    fn given_q_or_esc_when_mapping_then_quit() {
        assert_eq!(
            (press(KeyCode::Char('q')), press(KeyCode::Esc)),
            (Action::Quit, Action::Quit)
        );
    }

    #[test]
    fn given_ctrl_c_when_mapping_then_quit() {
        let e = key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        );
        assert_eq!(map_event(&e), Action::Quit);
    }

    #[test]
    fn given_other_key_or_plain_c_when_mapping_then_none() {
        assert_eq!(
            (press(KeyCode::Char('x')), press(KeyCode::Char('c'))),
            (Action::None, Action::None)
        );
    }

    #[test]
    fn given_key_release_of_q_when_mapping_then_none() {
        let e = key(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(map_event(&e), Action::None);
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
        puts: Vec<usize>,
        fail: bool,
    }

    impl ShmSink for FakeSink {
        fn put(&mut self, data: &[u8]) -> std::io::Result<String> {
            self.puts.push(data.len());
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
                app.sink.puts.clone()
            ),
            (2, false, vec![256 * 192 * 3, 256 * 192 * 3])
        );
    }

    #[test]
    fn given_shm_transfer_when_taking_graphics_then_payloads_encode_the_names_in_screen_order() {
        let mut app = shm_app(false);
        let out = text(app.graphics());
        let b64 =
            |name: &str| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, name);
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
}
