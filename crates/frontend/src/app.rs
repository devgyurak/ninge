// SPDX-License-Identifier: Apache-2.0
//! Application state, input mapping, and one frame of output.

use crate::kitty;
use crate::layout::{Areas, TermSize, compute};
use crate::pattern::dummy_frame;
use crate::placeholder::ScreenImage;
use anyhow::anyhow;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::widgets::{Block, Borders, Paragraph};
use std::io::Write;

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

/// Frontend state.
pub struct App {
    size: TermSize,
    areas: Option<Areas>,
    frame_no: u64,
    quit: bool,
    placement_pending: bool,
}

impl App {
    /// A new app for a terminal of `size`.
    pub fn new(size: TermSize) -> Self {
        Self {
            size,
            areas: compute(size),
            frame_no: 0,
            quit: false,
            placement_pending: true,
        }
    }

    /// Records that the terminal is now `size`; placements are re-sent next frame.
    pub fn resize(&mut self, size: TermSize) {
        self.size = size;
        self.areas = compute(size);
        self.placement_pending = true;
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

    /// The graphics bytes for this frame: both screens' pixels, then the virtual
    /// placements when the layout changed. Advances the frame counter.
    pub fn graphics(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(areas) = self.areas {
            out.extend(kitty::transmit(
                TOP_IMAGE_ID,
                &dummy_frame(self.frame_no, 0),
            ));
            out.extend(kitty::transmit(
                BOTTOM_IMAGE_ID,
                &dummy_frame(self.frame_no, 1),
            ));
            if self.placement_pending {
                out.extend(kitty::virtual_placement(
                    TOP_IMAGE_ID,
                    areas.top.width,
                    areas.top.height,
                ));
                out.extend(kitty::virtual_placement(
                    BOTTOM_IMAGE_ID,
                    areas.bottom.width,
                    areas.bottom.height,
                ));
                self.placement_pending = false;
            }
        }
        self.frame_no += 1;
        out
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
            Paragraph::new(format!("TODO(phase1) (#56){note}"))
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

/// One tick: write the graphics to `gfx`, then draw with ratatui.
pub fn frame_step<B: Backend, W: Write>(
    app: &mut App,
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
    fn given_new_app_when_taking_graphics_then_pixels_then_both_placements_are_sent() {
        let mut app = App::new(size());
        let out = String::from_utf8_lossy(&app.graphics()).into_owned();
        let at = |needle: &str| out.find(needle);
        assert!(
            at("a=t,f=24,s=256,v=192,i=1")
                .is_some_and(|t| at("a=p,U=1,i=1").is_some_and(|p| t < p))
        );
        assert!(at("a=p,U=1,i=2").is_some());
    }

    #[test]
    fn given_placements_already_sent_when_taking_graphics_again_then_only_pixels_follow() {
        let mut app = App::new(size());
        let _ = app.graphics();
        let out = String::from_utf8_lossy(&app.graphics()).into_owned();
        assert!(out.contains("i=2,t=d") && !out.contains("a=p"));
    }

    #[test]
    fn given_resize_when_taking_graphics_then_placements_are_re_sent() {
        let mut app = App::new(size());
        let _ = app.graphics();
        app.resize(TermSize {
            cols: 100,
            rows: 40,
            ..size()
        });
        let out = String::from_utf8_lossy(&app.graphics()).into_owned();
        assert!(out.contains("a=p,U=1,i=1"));
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
            (gfx.starts_with(b"\x1b_Ga=t"), cell),
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
}
