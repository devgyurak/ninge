// SPDX-License-Identifier: Apache-2.0
//! The whole emulated machine: [`Machine`], the one struct that owns all emulator state.

use crate::bus::{Arm7View, Arm9View, Memory, TcmConfig};
use crate::cart::Header;
use crate::cpu::{ARM7_INSTRUCTION_COST, ARM9_INSTRUCTION_COST, Cpu};
use crate::hle::{BiosState, SwiRecord};
use crate::input::Input;
use crate::io::Io;
use crate::scheduler::{EventKind, Scheduler};
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
    pub(crate) scheduler: Scheduler,
    // What the HLE BIOS records of each CPU's SWIs (`docs/hw/hle-swi.md`).
    pub(crate) bios: BiosState,
    // The parsed header of the booted program; `None` until `load_rom` accepts a ROM.
    pub(crate) header: Option<Header>,
    // The input the host set for the next frame; `run_frame` latches it into the keypads
    // (`docs/hw/keypad.md`, "Where the bits come from"). Plain state, so it is saved.
    pub(crate) input: Input,
}

/// One ARM9 instruction through `view`, and its cost added to the master clock. If the
/// controller's line was asserted before it and CPSR I is clear, the IRQ entry takes the
/// instruction's place and costs the same (`docs/hw/interrupts.md`, "Behavior"). The DS has no FIQ
/// source.
///
/// A halted core runs nothing until `IE & IF` is not zero (`docs/hw/hle-swi.md`, "Halt, and what
/// wakes a CPU"); a halted core that does not wake idles for one instruction's time. `view` may
/// live across many instructions: it is told each one's times here, and the caller refreshes its
/// TCM when CP15 changes (`run_arm9_until`).
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
}

/// One ARM7 instruction through `view` at the ARM7 clock `clock`, which it returns advanced by the
/// cost; the master clock stays where it is, and that is the floor of what the instruction
/// schedules. The IRQ entry takes the instruction's place as in [`arm9_instruction`]. A write of
/// HALTCNT's halt mode halts the ARM7 from the next instruction (`docs/hw/interrupts.md`,
/// "HALTCNT").
fn arm7_instruction(
    arm7: &mut Cpu<false>,
    bios: &mut SwiRecord,
    view: &mut Arm7View<'_>,
    clock: u64,
) -> u64 {
    if arm7.halted() {
        if !view.io().irq7().wakes() {
            return clock.saturating_add(ARM7_INSTRUCTION_COST);
        }
        arm7.set_halted(false);
    }
    let irq = view.io().irq7().line();
    view.begin_step(clock, view.master_clock());
    let cost = match arm7.take_interrupt(view, irq, false) {
        Some(cost) => cost,
        None => match crate::hle::enter(arm7, view, bios) {
            Some(cost) => cost,
            None => arm7.step(view),
        },
    };
    if view.io_mut().power_mut().take_halt() {
        arm7.set_halted(true);
    }
    clock.saturating_add(cost)
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
            scheduler: Scheduler::new(),
            bios: BiosState::default(),
            header: None,
            input: Input::default(),
        };
        machine.schedule_events();
        machine
    }

    /// Queues every event from the device state and `now`, on an empty queue: the next line
    /// and HBlank starts on their grid, each timer's overflow and each waiting DMA start. The
    /// queue is not saved, so `new` and `load_state` both build it here (`scheduler.md`,
    /// "Savestate checks").
    pub(crate) fn schedule_events(&mut self) {
        let now = self.scheduler.now();
        self.scheduler
            .schedule(Scheduler::next_hblank_start(now), EventKind::HBlankStart);
        self.scheduler
            .schedule(Scheduler::next_line_start(now), EventKind::LineStart);
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
    // TODO(hw-unverified): the per-frame instruction counts are a model (`docs/hw/scheduler.md`).
    pub fn run_frame(&mut self) {
        // The frame boundary is where the host's input reaches the registers.
        self.io.latch_input(self.input);
        if self.header.is_none() {
            self.frame_count = self.frame_count.wrapping_add(1);
            return;
        }
        let started = self.frame_count;
        while self.frame_count == started {
            crate::scheduler::run_round(self);
        }
    }

    /// Whether all four frame buffers have the size `new` allocates; a savestate that decodes
    /// to anything else is corrupt.
    pub(crate) fn screens_well_formed(&self) -> bool {
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
            TcmConfig::from_cp15(self.arm9.cp15()),
            0,
            0,
        );
        arm9_instruction(&mut self.arm9, &mut self.bios.arm9, &mut view);
    }

    /// One ARM7 instruction and its cost, with nothing else; see [`arm7_instruction`]. The run
    /// loop keeps one view for a whole batch instead; this is the lone step tests drive.
    #[cfg(test)]
    pub(crate) fn step_arm7_instruction(&mut self) {
        let floor = self.scheduler.now();
        let mut view = Arm7View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            self.arm7_clock,
            floor,
        );
        self.arm7_clock = arm7_instruction(
            &mut self.arm7,
            &mut self.bios.arm7,
            &mut view,
            self.arm7_clock,
        );
    }

    /// Sets the input for the next frame: the buttons held, the touch point and the hinge. It is
    /// latched into the keypad registers when [`Machine::run_frame`] begins and stays until the
    /// next one, so `set_input` then `run_frame`, once per frame, is a complete input log and
    /// replays to the same frames and savestate bytes (`docs/hw/keypad.md`, "Where the bits come
    /// from"). [`Machine::load_rom`] starts from power-on and so clears the input.
    pub fn set_input(&mut self, input: Input) {
        self.input = input;
    }

    /// The audio the last frame made, as interleaved stereo `i16` samples.
    ///
    /// The shape is fixed now so the frontend can be written against it: a slice the machine
    /// owns, borrowed like [`Machine::frame`], valid until the next `run_frame`.
    // TODO(phase6): the SPU (#47) fills this; until then no sound is made and it is empty.
    pub fn audio(&self) -> &[i16] {
        &[]
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

mod events;
mod render;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
