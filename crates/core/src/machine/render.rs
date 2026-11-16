// SPDX-License-Identifier: Apache-2.0
//! The per-line picture pipeline: both engines make their line in 15-bit colour, a capture stage
//! taps engine A's line, and only then does the master brightness and the host conversion run
//! (`docs/hw/blending.md`, "Master brightness": the capture unit reads the line before it).

use super::{Machine, SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::bus::Memory;
use crate::gpu2d::{EngineId, expand_line};

/// The display capture stage: it sees engine A's finished line, in the colour the hardware has
/// and before the master brightness, and will write it to VRAM.
// TODO(phase4): display capture (#36) fills this in; it takes the memory so it can store the line
// without another restructuring of `render_line` (`blending.md`, "Master brightness").
fn capture_tap(_memory: &mut Memory, _y: usize, _line: &[u16; SCREEN_WIDTH]) {}

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
        let gpu = self.io.gpu2d();
        let mut line_a = [0; SCREEN_WIDTH];
        let mut line_b = [0; SCREEN_WIDTH];
        let inputs_a = self.memory.engine_inputs(EngineId::A);
        let brightness_a = gpu.render_line_a(y, self.memory.lcdc_banks(), &inputs_a, &mut line_a);
        let inputs_b = self.memory.engine_inputs(EngineId::B);
        let brightness_b = gpu.render_line_b(y, &inputs_b, &mut line_b);
        capture_tap(&mut self.memory, y, &line_a);
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
