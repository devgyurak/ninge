// SPDX-License-Identifier: Apache-2.0
//! The per-line picture pipeline: both engines make their line in 15-bit colour, a capture stage
//! taps engine A's line, and only then does the master brightness and the host conversion run
//! (`docs/hw/blending.md`, "Master brightness": the capture unit reads the line before it).

use super::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::bus::Memory;
use crate::gpu2d::{EngineId, Gpu2d, Line3d, TRANSPARENT_LINE, expand_line};

/// The display capture stage: it sees engine A's composed graphics screen for the line, in the
/// colour the hardware has and before the master brightness, and stores it in the VRAM block
/// DISPCAPCNT names (`docs/hw/display-capture.md`, "Where the capture unit sits"). A line the
/// capture does not take (no capture is running, or its size has no such row) is left alone.
fn capture_tap(
    memory: &mut Memory,
    gpu: &Gpu2d,
    y: usize,
    composed: &[u16; SCREEN_WIDTH],
    three_d: &Line3d,
    fifo: &[u16; SCREEN_WIDTH],
) {
    let Some(capture) = gpu.capture_line(y) else {
        return;
    };
    // Source B is read in full before the line is written, so a read and a write of one bank on one
    // line see the read first (`display-capture.md`, "Unverified").
    // TODO(hw-unverified): the read-before-write order for one bank on one line.
    let read_bank = memory
        .lcdc_banks()
        .get(capture.read_block())
        .copied()
        .flatten();
    let b = capture.source_b(y, read_bank, fifo);
    let pixels = capture.pixels(composed, three_d, &b);
    if let Some(bank) = memory.lcdc_bank_mut(capture.write_block()) {
        capture.store(y, bank, &pixels);
    }
}

impl Machine {
    /// Draws visible line `y` of both screens into the frame being drawn, from the registers and
    /// memory as they are now (`docs/hw/bg-text.md`, "Lines"). A line past the last visible one is
    /// not drawn.
    ///
    /// Engine A goes to the screen POWCNT1 bit 15 picks and engine B to the other one
    /// (`docs/hw/bg-modes.md`, "Engine B"). Each line is made in 15-bit colour on the stack (it
    /// carries nothing to the next line, so it is not state), tapped for capture, then brightened
    /// and expanded for the host.
    // TODO(hw-unverified): the registers, the transform and the reference point are sampled at the
    // start of the line, where the hardware reads them while it draws (`bg-modes.md`, "Unverified").
    pub(crate) fn render_line(&mut self, y: usize) {
        if y >= SCREEN_HEIGHT {
            return;
        }
        self.io.gpu2d_mut().begin_line(y);
        // The display takes this line's pixels out of the main-memory display FIFO, shown or not
        // (`display-capture.md`, "The main-memory display").
        let fifo = self.io.gpu2d_mut().take_fifo_line();
        let gpu = self.io.gpu2d();
        let mut line_a = [0; SCREEN_WIDTH];
        let mut line_b = [0; SCREEN_WIDTH];
        let mut composed = [0; SCREEN_WIDTH];
        // The 3D layer's line: transparent until the rasterizer draws (#49).
        // TODO(phase7): the rasterizer's line for `y`.
        let three_d = &TRANSPARENT_LINE;
        let inputs_a = self.memory.engine_inputs(EngineId::A).with_three_d(three_d);
        // The layers are composed when the screen shows them; a capture reads them in every
        // display mode, so it will ask for them too (`display-capture.md`).
        let capture_wants_a = gpu
            .capture_line(y)
            .is_some_and(|capture| capture.reads_source_a());
        if gpu.shows_layers_a(&inputs_a) || capture_wants_a {
            gpu.compose_line_a(y, &inputs_a, &mut composed);
        }
        let brightness_a = gpu.select_line_a(
            y,
            self.memory.lcdc_banks(),
            &inputs_a,
            &composed,
            &fifo,
            &mut line_a,
        );
        let inputs_b = self.memory.engine_inputs(EngineId::B);
        let brightness_b = gpu.render_line_b(y, &inputs_b, &mut line_b);
        capture_tap(&mut self.memory, gpu, y, &composed, three_d, &fifo);
        let (out_a, out_b) = if gpu.engine_a_on_top() {
            (&mut self.work_top, &mut self.work_bottom)
        } else {
            (&mut self.work_bottom, &mut self.work_top)
        };
        let row = y * SCREEN_WIDTH..(y + 1) * SCREEN_WIDTH;
        if let Some(out) = out_a.get_mut(row.clone()) {
            expand_line(&line_a, brightness_a, out);
        }
        if let Some(out) = out_b.get_mut(row) {
            expand_line(&line_b, brightness_b, out);
        }
        // The affine layers' reference points move down a line (`bg-modes.md`, "When the picture is
        // made"). Both render paths come through here.
        self.io.gpu2d_mut().end_line();
    }
}
