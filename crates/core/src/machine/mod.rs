// SPDX-License-Identifier: Apache-2.0
//! The whole emulated machine: [`Machine`], the one struct that owns all emulator state.

use crate::bus::{Arm7View, Arm9View, Memory, TcmConfig};
use crate::cart::{Header, SaveError, SaveType};
use crate::cpu::{ARM7_INSTRUCTION_COST, ARM9_INSTRUCTION_COST, Cpu};
use crate::hle::{BiosState, SwiRecord};
use crate::input::Input;
use crate::io::Io;
use crate::scheduler::{EventKind, FRAME_CYCLES, LineEvent, Round, Scheduler};
use crate::spu::OUTPUT_SAMPLE_CYCLES;
use serde::{Deserialize, Serialize};

/// Width of each screen in pixels (`docs/hw/display.md`).
pub const SCREEN_WIDTH: usize = 256;
/// Height of each screen in pixels (`docs/hw/display.md`).
pub const SCREEN_HEIGHT: usize = 192;

/// One of the two physical screens.
///
/// `Top` and `Bottom` are the physical screens, not graphics engines A/B: which engine shows
/// on which screen is set by POWCNT1 (Phase 2-3). See `docs/hw/display.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Screen {
    /// The upper screen.
    Top,
    /// The lower screen, which carries the touch panel.
    Bottom,
}

/// The whole emulated machine; it owns all emulator state.
///
/// Frame buffers are `u32` per pixel in xRGB8888 (`0x00RRGGBB`). That is the emulator's
/// host-side output format, not a hardware fact (`docs/hw/display.md`).
///
/// Load a savestate only through [`Machine::load_state`]; deserializing a `Machine` directly
/// skips its checks and leaves the derived page table empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    // Invariant: each of the four buffers holds exactly SCREEN_WIDTH * SCREEN_HEIGHT pixels.
    // They are `Vec`s because serde does not derive for arrays this large. `top` and `bottom`
    // are the last finished frame; each visible line is drawn into `work_top`/`work_bottom` at
    // its line start and the pairs swap when line 192 starts, so a frame a caller reads is always
    // a whole one (`docs/hw/bg-text.md`, "Lines"). All four are saved: a line drawn from a
    // register that changed later in the same frame cannot be drawn again after a load, so
    // saving them is what makes save, load and continue equal an uninterrupted run (#28).
    pub(crate) top: Vec<u32>,
    pub(crate) bottom: Vec<u32>,
    pub(crate) work_top: Vec<u32>,
    pub(crate) work_bottom: Vec<u32>,
    // The ROM image is an input, not machine state: it is skipped on save and a savestate
    // loads onto a machine that was handed the ROM again, so a savestate never embeds the
    // (possibly commercial) image (`savestate` module docs). Defaults to empty.
    // Invariant: `header` is `Some` exactly when `rom` is non-empty; `load_state` keeps it.
    // Note: `Clone` copies the whole ROM; revisit before rewind and run-ahead (no phase or issue
    // plans them yet).
    #[serde(skip)]
    pub(crate) rom: Vec<u8>,
    // Frames completed by `run_frame`; plain state, so it is serialized. A savestate
    // without it is refused, never half-loaded (AGENTS.md §2 rule 5).
    pub(crate) frame_count: u64,
    // The ARM9, the RAM both CPUs see and the register devices. Plain data; a savestate lacking
    // them is refused.
    pub(crate) arm9: Cpu<true>,
    pub(crate) memory: Memory,
    pub(crate) io: Io,
    // The ARM7 and its clock in master cycles; the scheduler (`docs/hw/scheduler.md`).
    pub(crate) arm7: Cpu<false>,
    pub(crate) arm7_clock: u64,
    // The master time the ARM9 is held until by a full geometry FIFO; 0 or the past: not held.
    pub(crate) arm9_hold_until: u64,
    pub(crate) scheduler: Scheduler,
    // The round a stop of `run_until` interrupted, if one did (`docs/hw/scheduler.md`, "Run
    // control"). Plain data: saving it is what makes a save, a load and a continue equal an
    // uninterrupted run.
    pub(crate) round: Option<Round>,
    // What the HLE BIOS records of each CPU's SWIs (`docs/hw/hle-swi.md`).
    pub(crate) bios: BiosState,
    // The parsed header of the booted program; `None` until `load_rom` accepts a ROM.
    pub(crate) header: Option<Header>,
    // The audio of the frame being run: output, not state, so it is not saved. `run_frame` clears it
    // (the capacity stays) and the sound unit's sample events fill it; reserved once, for a frame's
    // samples (`docs/hw/spu.md`, "Output samples").
    #[serde(skip)]
    pub(crate) audio: Vec<i16>,
}

/// The most `i16` values (two per output sample) one frame's audio holds: a frame's output samples
/// and one more for a frame that does not start on the grid.
pub(crate) const AUDIO_CAPACITY: usize = 2 * (FRAME_CYCLES / OUTPUT_SAMPLE_CYCLES + 2) as usize;

/// One ARM9 instruction through `view`, and its cost added to the master clock. If the
/// controller's line was asserted before it and CPSR I is clear, the IRQ entry takes the
/// instruction's place and costs the same (`docs/hw/interrupts.md`, "Behavior"). The DS has no FIQ
/// source.
///
/// A halted core runs nothing until `IE & IF` is not zero (`docs/hw/hle-swi.md`, "Halt, and what
/// wakes a CPU"); a halted core that does not wake idles for one instruction's time. `view` may
/// live across many instructions: it is told each one's times here, and the caller refreshes its
/// TCM when CP15 changes (`run_arm9_until`).
// TODO(hw-unverified): the line is sampled before each instruction and an IME or IE write reaches
// it at once, here and in `arm7_instruction` (`docs/hw/interrupts.md`, "Unverified").
fn arm9_instruction(arm9: &mut Cpu<true>, bios: &mut SwiRecord, view: &mut Arm9View<'_>) {
    if arm9.halted() {
        if !view.io().irq9().wakes() {
            view.advance(ARM9_INSTRUCTION_COST);
            return;
        }
        arm9.set_halted(false);
    }
    let irq = view.io().irq9().line();
    let now = view.clock();
    // The cost is known only once the instruction ends, so the floor is handed in: an event the
    // instruction schedules fires no earlier than its end (`timers.md`, "Model").
    view.begin_step(now, now.saturating_add(ARM9_INSTRUCTION_COST));
    // An interrupt comes first, even at a BIOS address: IntrWait waits with I clear, and the IRQ
    // must be taken before its loop looks again. At the vectors I is set, so the BIOS's work,
    // which takes the instruction's place there, is next (`docs/hw/hle-swi.md`).
    let cost = match arm9.take_interrupt(view, irq, false) {
        Some(cost) => cost,
        None => match crate::hle::enter(arm9, view, bios) {
            Some(cost) => cost,
            None => arm9.step(view),
        },
    };
    view.advance(cost);
    // A write to a full geometry FIFO held the ARM9 for the engine's cycles: it runs no instruction
    // until then, while the events in between still fire (`geometry-engine.md`, "The FIFO IRQ, the
    // DMA start and the stall").
    let stall = view.take_stall();
    view.hold(view.clock().saturating_add(stall));
}

/// One ARM7 instruction through `view` at the view's ARM7 clock, which it advances by the cost;
/// the master clock stays where it is, and that is the floor of what the instruction
/// schedules. The IRQ entry takes the instruction's place as in [`arm9_instruction`]. A write of
/// HALTCNT's halt mode halts the ARM7 from the next instruction (`docs/hw/interrupts.md`,
/// "HALTCNT").
// TODO(hw-unverified): the IRQ line is sampled before each instruction, as for the ARM9
// (`docs/hw/interrupts.md`, "Unverified").
fn arm7_instruction(arm7: &mut Cpu<false>, bios: &mut SwiRecord, view: &mut Arm7View<'_>) {
    if arm7.halted() {
        if !view.io().irq7().wakes() {
            view.advance(ARM7_INSTRUCTION_COST);
            return;
        }
        arm7.set_halted(false);
    }
    let irq = view.io().irq7().line();
    view.begin_step(view.clock(), view.master_clock());
    let cost = match arm7.take_interrupt(view, irq, false) {
        Some(cost) => cost,
        None => match crate::hle::enter(arm7, view, bios) {
            Some(cost) => cost,
            None => arm7.step(view),
        },
    };
    // The next instruction must not run: the halt takes effect between the two.
    halt_if_asked(arm7, view);
    view.advance(cost);
}

/// Halts the ARM7 when a write through `view` asked it to: a HALTCNT write of the halt mode, from
/// an instruction, a DMA unit or a sound capture (`docs/hw/interrupts.md`, "HALTCNT"). The one
/// place that takes the request; [`Machine::with_arm7_bus`] calls it after any work on the ARM7's
/// bus, and the instruction loop after each instruction as well.
fn halt_if_asked(arm7: &mut Cpu<false>, view: &mut Arm7View<'_>) {
    if view.take_halt() {
        arm7.set_halted(true);
    }
}

/// Why [`Machine::load_rom`] refused a ROM: the header is invalid (`docs/hw/cart-header.md`).
pub type RomError = crate::cart::HeaderError;

/// A black screen of exactly `SCREEN_WIDTH * SCREEN_HEIGHT` pixels.
fn blank_screen() -> Vec<u32> {
    vec![0x0000_0000; SCREEN_WIDTH * SCREEN_HEIGHT]
}

impl Machine {
    /// Creates a machine with both screens black.
    ///
    /// Black is a Phase 0 placeholder value, not the hardware power-on state.
    pub fn new() -> Self {
        let mut machine = Self {
            top: blank_screen(),
            bottom: blank_screen(),
            work_top: blank_screen(),
            work_bottom: blank_screen(),
            rom: Vec::new(),
            frame_count: 0,
            arm9: Cpu::new(),
            memory: Memory::new(),
            io: Io::new(),
            arm7: Cpu::new(),
            arm7_clock: 0,
            arm9_hold_until: 0,
            scheduler: Scheduler::new(),
            round: None,
            bios: BiosState::default(),
            header: None,
            audio: Vec::with_capacity(AUDIO_CAPACITY),
        };
        machine.schedule_events();
        machine
    }

    /// The time the queue is rebuilt from: the line and sample grids count events strictly after
    /// it. It is `now`, except in a round a stop interrupted after the ARM9 ran (`stop` is set),
    /// where the events due at `now` have not fired yet (the round fires them last) and are queued
    /// at `now` too. While the ARM9 still runs, an event due at `now` has fired already. This is
    /// the one place the rebuild looks at the round (`scheduler.md`, "Run control").
    pub(crate) fn events_from(&self) -> u64 {
        let now = self.scheduler.now();
        match self.round {
            Some(Round { stop: Some(_), .. }) => now.saturating_sub(1),
            _ => now,
        }
    }

    /// Queues every event from the device state and `now`, on an empty queue: the next line
    /// and HBlank starts on their grid, each timer's overflow and each waiting DMA start. The
    /// queue is not saved, so `new` and `load_state` both build it here (`scheduler.md`,
    /// "Savestate checks").
    pub(crate) fn schedule_events(&mut self) {
        let now = self.scheduler.now();
        let from = self.events_from();
        self.scheduler.schedule(
            Scheduler::next_hblank_start(from),
            EventKind::Line(LineEvent::HBlank),
        );
        self.scheduler.schedule(
            Scheduler::next_line_start(from),
            EventKind::Line(LineEvent::Start),
        );
        self.scheduler
            .schedule(Scheduler::next_sample_time(from), EventKind::Sample);
        self.io.schedule_all(&mut self.scheduler, now);
    }

    /// Hands the machine a ROM image, kept verbatim, and direct-boots it.
    ///
    /// Loading a ROM starts from power-on: every other piece of state is reset first. This is
    /// the host API's contract, not hardware behavior. The header is then parsed and the ARM9
    /// started at its entry point (`docs/hw/direct-boot.md`).
    ///
    /// A ROM whose header is invalid is refused with a [`RomError`] and the machine stays in
    /// the power-on state with no program loaded. A header whose CRC does not match is
    /// accepted and boots (`docs/hw/cart-header.md`). The image is guest-controlled input and
    /// never panics the core.
    pub fn load_rom(&mut self, rom: Vec<u8>) -> Result<(), RomError> {
        *self = Self::new();
        let header = crate::cart::parse(&rom)?;
        crate::hle::boot(
            &mut self.arm9,
            &mut self.arm7,
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            &rom,
            &header,
        );
        self.header = Some(header);
        self.rom = rom;
        // The first line start is line 1, so line 0 of the first frame is drawn here.
        self.render_line(0);
        Ok(())
    }

    /// The parsed header of the booted program, or `None` when no program is booted.
    // The debugger's CPU panel (#56) will show it.
    pub fn header(&self) -> Option<&Header> {
        self.header.as_ref()
    }

    /// Fits the cartridge with a save chip of `kind` holding `data`, as a save file holds it.
    ///
    /// Call it after [`Machine::load_rom`], which resets the machine and with it the chip. An image
    /// that is not as long as the chip's memory is refused and the chip stays as it was
    /// (`docs/hw/auxspi-saves.md`, "Save files").
    pub fn insert_save(&mut self, kind: SaveType, data: Vec<u8>) -> Result<(), SaveError> {
        self.io.cart_mut().insert_save(kind, data)
    }

    /// The save chip's memory, as a save file holds it; empty with no chip.
    pub fn save_data(&self) -> &[u8] {
        self.io.cart().save().data()
    }

    /// The type of the fitted save chip.
    pub fn save_type(&self) -> SaveType {
        self.io.cart().save().kind()
    }

    /// Counts the changes to the save memory. A host writes the save file when it differs from the
    /// value it saw at its last write. A savestate load raises it, since the memory may have changed
    /// (`docs/hw/auxspi-saves.md`, "Save files"). [`Machine::load_rom`] starts it again at 0, so a host
    /// that resets the machine takes a new baseline after it fits the save.
    pub fn save_generation(&self) -> u64 {
        self.io.cart().save().generation()
    }

    /// Returns the loaded ROM image; empty until [`Machine::load_rom`] is called.
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    /// Runs the machine for one frame: rounds of ARM9, ARM7, events until VCOUNT wraps to line
    /// 0, which counts the frame; the lines were drawn as their starts went by and the frame
    /// was presented at line 192 (`docs/hw/scheduler.md`, `docs/hw/display-timing.md`).
    ///
    /// A machine with no booted program (no ROM, or a refused one) runs no CPU and draws
    /// nothing: the frame only advances the counter and leaves both screens black.
    ///
    /// `input` is this frame's input: the buttons held, the touch point and the hinge. It is
    /// latched into the keypad registers as the frame begins and read from there until the next
    /// one, so every frame's input is explicit and one `Input` per frame is a complete input
    /// log, which replays to the same frames and savestate bytes (`docs/hw/keypad.md`, "Where
    /// the bits come from"). A host with no input passes `Input::default()`.
    // TODO(hw-unverified): the per-frame instruction counts are a model (`docs/hw/scheduler.md`).
    // TODO(hw-unverified): a button change reaches the registers at the frame boundary only
    // (`docs/hw/keypad.md`, "Unverified").
    pub fn run_frame(&mut self, input: Input) {
        self.run_until(input, &StopSet::frame_end());
    }

    /// Whether all four frame buffers have the size `new` allocates; a savestate that decodes
    /// to anything else is corrupt.
    fn screens_well_formed(&self) -> bool {
        [&self.top, &self.bottom, &self.work_top, &self.work_bottom]
            .iter()
            .all(|buffer| buffer.len() == SCREEN_WIDTH * SCREEN_HEIGHT)
    }

    /// The frame being drawn is complete: it becomes the one [`Machine::frame`] returns, and the
    /// buffers it replaces are drawn over by the next frame's lines.
    pub(crate) fn present_frame(&mut self) {
        std::mem::swap(&mut self.top, &mut self.work_top);
        std::mem::swap(&mut self.bottom, &mut self.work_bottom);
    }

    /// Draws every visible line from the registers and memory as they are now and presents the
    /// result: a whole frame without the line timing, for tests that set the state directly.
    #[cfg(test)]
    pub(crate) fn render_frame(&mut self) {
        // A frame starts from the registers, as VBlank leaves it.
        self.io.gpu2d_mut().vblank_started();
        (0..SCREEN_HEIGHT).for_each(|y| self.render_line(y));
        self.present_frame();
    }

    /// Returns how many frames [`Machine::run_frame`] has completed.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Steps the ARM9 by one instruction and advances the master clock by its cost
    /// (`docs/hw/scheduler.md`, "Debugger single-step").
    ///
    /// On a machine with a booted program the ARM7 is then brought up to the new time and any
    /// event that became due fires, so a debugger stepping across a frame end still draws and
    /// counts the frame, and the clocks stay consistent for a savestate. A machine with no
    /// program steps only the ARM9.
    ///
    /// The TCM registers are copied into the ARM9's bus view first, so a CP15 write takes effect
    /// from the next instruction (`docs/hw/arm9-memory-map.md`).
    ///
    /// A step that meets an asserted IRQ line with CPSR I clear only enters the IRQ vector; the
    /// instruction that would have run is not executed (`docs/hw/interrupts.md`, "Behavior").
    // TODO(hw-unverified): opcodes already in the pipeline keep the mapping they were fetched
    // under (`docs/hw/arm9-memory-map.md`, "Unverified").
    pub fn step_arm9(&mut self) {
        self.finish_round();
        self.step_arm9_instruction();
        if self.header.is_some() {
            crate::scheduler::settle(self);
        }
    }

    /// One ARM9 instruction and its cost, with nothing else; see [`arm9_instruction`].
    // TODO(hw-unverified): the line is sampled before each instruction and an IME or IE write
    // reaches it at once (`docs/hw/interrupts.md`, "Unverified").
    fn step_arm9_instruction(&mut self) {
        let mut view = Arm9View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            &self.rom,
            TcmConfig::from_cp15(self.arm9.cp15()),
        )
        .with_hold(self.arm9_hold_until);
        // A hold that is already on ends before the step: the ARM9 runs no instruction in it.
        let waiting = view.hold_until().saturating_sub(view.clock());
        view.advance(waiting);
        arm9_instruction(&mut self.arm9, &mut self.bios.arm9, &mut view);
        // A lone step runs the whole hold: a debugger step is one instruction and what it cost.
        let rest = view.hold_until().saturating_sub(view.clock());
        view.advance(rest);
        self.arm9_hold_until = 0;
    }

    /// Runs one ARM7 instruction and its cost, for a debugger, as [`Machine::step_arm9`] does for
    /// the ARM9: the machine is then settled (the ARM9 and the events that became due catch up to
    /// the ARM7's clock) and the round a stop interrupted ends. A machine with no program steps
    /// nothing.
    pub fn step_arm7(&mut self) {
        if self.header.is_none() {
            return;
        }
        self.finish_round();
        self.step_arm7_instruction();
        // The ARM9 and the events catch up to the ARM7's clock, which the step moved ahead.
        let target = self.arm7_clock;
        crate::scheduler::catch_up(self, target);
    }

    /// Runs what is left of the round a stop of [`Machine::run_until`] interrupted, with no stop
    /// conditions, so a step after a stop starts from the state an uninterrupted run reaches at the
    /// end of that round and not from a half-run one.
    fn finish_round(&mut self) {
        if let Some(mut round) = self.round.take() {
            crate::scheduler::continue_round(self, &mut round);
        }
    }

    /// One ARM7 instruction and its cost, with nothing else; see [`arm7_instruction`]. The run
    /// loop keeps one view for a whole batch instead.
    pub(crate) fn step_arm7_instruction(&mut self) {
        self.with_arm7_bus(arm7_instruction);
    }

    /// Runs `work` on the ARM7's bus view at the ARM7's clock, keeps the clock the view leaves and
    /// then halts the ARM7 if a write in `work` asked to ([`halt_if_asked`]). Every path that
    /// writes through the ARM7's bus while the machine runs (its instructions, a DMA unit, a sound
    /// capture) builds its view here, so a new one acts on the halt request without naming it.
    pub(crate) fn with_arm7_bus<R>(
        &mut self,
        work: impl FnOnce(&mut Cpu<false>, &mut SwiRecord, &mut Arm7View<'_>) -> R,
    ) -> R {
        let mut view = Arm7View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            &self.rom,
            self.arm7_clock,
        );
        let result = work(&mut self.arm7, &mut self.bios.arm7, &mut view);
        halt_if_asked(&mut self.arm7, &mut view);
        self.arm7_clock = view.clock();
        result
    }

    /// The audio the last frame made, as interleaved stereo `i16` samples.
    ///
    /// The samples are interleaved stereo `i16`, the left channel then the right. The number of
    /// samples may differ from frame to frame, so a host must not assume a fixed length. The
    /// slice is the machine's own, borrowed like [`Machine::frame`], and valid until the next
    /// `run_frame`.
    ///
    /// The output rate is one sample per [`OUTPUT_SAMPLE_CYCLES`] master cycles
    /// (`docs/hw/spu.md`, "Output samples"); resampling to a device is the host's (#47).
    pub fn audio(&self) -> &[i16] {
        &self.audio
    }

    /// Whether the program shut the console down through the power-management device. A host ends
    /// its run when this turns true: every later [`Machine::run_frame`] runs nothing
    /// (`docs/hw/touch-power.md`, "What the power register does").
    pub fn is_shut_down(&self) -> bool {
        self.io.spi().power().is_shut_down()
    }

    /// Whether `screen`'s backlight is on. The frame buffers are not dimmed: an unlit screen is
    /// the host's to draw black (`docs/hw/touch-power.md`, "What the power register does").
    /// `Screen::Top` is the upper backlight and `Screen::Bottom` the lower, the physical screens
    /// [`Machine::frame`] returns.
    // TODO(hw-unverified): what an unlit screen shows is the host's choice (`touch-power.md`,
    // "Unverified").
    pub fn backlight(&self, screen: Screen) -> bool {
        let power = self.io.spi().power();
        match screen {
            Screen::Top => power.upper_backlight(),
            Screen::Bottom => power.lower_backlight(),
        }
    }

    /// Returns the pixels of one screen, row by row, as xRGB8888.
    pub fn frame(&self, screen: Screen) -> &[u32] {
        match screen {
            Screen::Top => &self.top,
            Screen::Bottom => &self.bottom,
        }
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

mod check;
mod events;
mod render;
mod run;
pub use run::{MAX_BREAKPOINTS, StopReason, StopSet};
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
