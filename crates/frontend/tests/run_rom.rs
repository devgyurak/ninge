// SPDX-License-Identifier: Apache-2.0
//! The frontend running our own `vramdisp` test ROM (`testroms/vramdisp/`), built by
//! `scripts/build_testroms.sh`. Engine A shows the ROM's picture on the BOTTOM screen
//! (POWCNT1 bit 15 clear); its first bar is white, so the bottom screen's (0, 0) pixel is
//! `FF FF FF` once the picture is drawn, which takes three frames (see
//! `crates/runner/tests/vramdisp.rs`).

use ninge::app::App;
use ninge::kitty::rgb24;
use ninge::layout::TermSize;
use ninge::source::{Source, boot, load};
use ninge::transfer::{ShmSink, Transfer};
use ninge_core::{Machine, Screen};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const FRAMES: u64 = 3;

fn rom_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/vramdisp.nds")
}

fn size() -> TermSize {
    TermSize {
        cols: 120,
        rows: 50,
        cell_w: 10,
        cell_h: 20,
        px_known: true,
    }
}

/// Records every shared-memory frame.
struct Recorder(Rc<RefCell<Vec<Vec<u8>>>>);

impl ShmSink for Recorder {
    fn put(&mut self, data: &[u8]) -> std::io::Result<String> {
        let mut puts = self.0.borrow_mut();
        puts.push(data.to_vec());
        Ok(format!("/rec-{}", puts.len()))
    }
}

type Puts = Rc<RefCell<Vec<Vec<u8>>>>;

fn app_with(size: TermSize) -> (App<Recorder>, Puts) {
    let puts = Puts::default();
    let app = App::with_transfer(size, Recorder(Rc::clone(&puts)), Transfer::Shm)
        .with_source(load(rom_path()).expect("vramdisp ROM is built"));
    (app, puts)
}

fn reference(frames: u64) -> Machine {
    let rom = std::fs::read(rom_path()).expect("vramdisp ROM is built");
    let mut machine = boot(rom).expect("valid header");
    (0..frames).for_each(|_| machine.run_frame());
    machine
}

fn run_ticks(app: &mut App<Recorder>, ticks: u64) {
    (0..ticks).for_each(|_| {
        let _ = app.graphics();
    });
}

fn frames_run(app: &App<Recorder>) -> Option<u64> {
    app.machine().map(Machine::frame_count)
}

#[test]
fn given_a_machine_source_when_taking_graphics_once_then_one_frame_ran() {
    let (mut app, _) = app_with(size());
    let _ = app.graphics();
    assert_eq!(frames_run(&app), Some(1));
}

#[test]
fn given_a_too_small_terminal_when_taking_graphics_then_nothing_is_sent_but_the_machine_ran() {
    let (mut app, _) = app_with(TermSize {
        cols: 2,
        rows: 1,
        ..size()
    });
    let out = app.graphics();
    assert_eq!((out, frames_run(&app)), (Vec::<u8>::new(), Some(1)));
}

#[test]
fn given_a_machine_source_over_shm_when_taking_graphics_then_the_sink_gets_the_machines_screens() {
    let (mut app, puts) = app_with(size());
    let _ = app.graphics();
    let machine = reference(1);
    let expected = vec![
        rgb24(machine.frame(Screen::Top)),
        rgb24(machine.frame(Screen::Bottom)),
    ];
    assert_eq!(*puts.borrow(), expected);
}

#[test]
fn given_vramdisp_when_three_frames_run_then_the_bottom_screens_first_pixel_is_white() {
    let (mut app, puts) = app_with(size());
    run_ticks(&mut app, FRAMES);
    let first_pixel: Vec<u8> = puts.borrow().last().expect("frames were sent")[..3].to_vec();
    assert_eq!(first_pixel, vec![0xFF, 0xFF, 0xFF]);
}

#[test]
fn given_no_rom_source_when_asking_for_the_machine_then_there_is_none() {
    let app = App::new(size()).with_source(Source::Pattern);
    assert!(app.machine().is_none());
}
