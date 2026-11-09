// SPDX-License-Identifier: Apache-2.0
//! ninge-core: the emulator itself. Pure logic with no IO; inputs in, frames out.

pub mod bus;
pub mod cart;
pub mod cpu;
pub mod gpu2d;
mod hle;
pub mod io;
pub mod ipc;
pub mod irq;
pub mod savestate;
pub mod scheduler;
pub mod timers;

use bus::{Arm7View, Arm9View, Memory, TcmConfig, VramBankState, WramState};
use cart::Header;
use cpu::Cpu;
use io::Io;
use irq::InterruptController;
use scheduler::{
    ARM7_INSTRUCTION_COST, ARM9_INSTRUCTION_COST, Event, EventKind, HBLANK_START_CYCLES,
    LINE_CYCLES, Participants, Scheduler,
};
use serde::{Deserialize, Serialize};
use timers::Timers;

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
    // Invariant: each buffer holds exactly SCREEN_WIDTH * SCREEN_HEIGHT pixels. They are
    // `Vec`s because serde does not derive for arrays this large. The buffers are derived
    // output, not savestate data: they are skipped on save, and a load recreates them blank
    // (restoring the invariant) until the next frame is drawn. The savestate format is in
    // the `savestate` module.
    #[serde(skip, default = "blank_screen")]
    top: Vec<u32>,
    #[serde(skip, default = "blank_screen")]
    bottom: Vec<u32>,
    // The ROM image is an input, not machine state: it is skipped on save and a savestate
    // loads onto a machine that was handed the ROM again, so a savestate never embeds the
    // (possibly commercial) image (`savestate` module docs). Defaults to empty.
    // Invariant: `header` is `Some` exactly when `rom` is non-empty; `load_state` keeps it.
    // Note: `Clone` copies the whole ROM; revisit before rewind and run-ahead (no phase or issue
    // plans them yet).
    #[serde(skip)]
    rom: Vec<u8>,
    // Frames completed by `run_frame`; plain state, so it is serialized. A savestate
    // without it is refused, never half-loaded (AGENTS.md §2 rule 5).
    frame_count: u64,
    // The ARM9, the RAM both CPUs see and the register devices. Plain data; a savestate lacking
    // them is refused.
    arm9: Cpu<true>,
    memory: Memory,
    io: Io,
    // The ARM7 and its clock in master cycles; the scheduler (`docs/hw/scheduler.md`).
    arm7: Cpu<false>,
    arm7_clock: u64,
    scheduler: Scheduler,
    // The parsed header of the booted program; `None` until `load_rom` accepts a ROM.
    header: Option<Header>,
}

/// Why [`Machine::load_rom`] refused a ROM: the header is invalid (`docs/hw/cart-header.md`).
pub type RomError = cart::HeaderError;

/// A black screen of exactly `SCREEN_WIDTH * SCREEN_HEIGHT` pixels.
fn blank_screen() -> Vec<u32> {
    vec![0x0000_0000; SCREEN_WIDTH * SCREEN_HEIGHT]
}

impl Machine {
    /// Creates a machine with both screens black.
    ///
    /// Black is a Phase 0 placeholder value, not the hardware power-on state.
    pub fn new() -> Self {
        let mut scheduler = Scheduler::new();
        scheduler.schedule(HBLANK_START_CYCLES, EventKind::HBlankStart);
        scheduler.schedule(LINE_CYCLES, EventKind::LineStart);
        Self {
            top: blank_screen(),
            bottom: blank_screen(),
            rom: Vec::new(),
            frame_count: 0,
            arm9: Cpu::new(),
            memory: Memory::new(),
            io: Io::new(),
            arm7: Cpu::new(),
            arm7_clock: 0,
            scheduler,
            header: None,
        }
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
        let header = cart::parse(&rom)?;
        hle::boot(
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
    /// 0, which draws the screens and counts the frame (`docs/hw/scheduler.md`,
    /// `docs/hw/display-timing.md`).
    ///
    /// A machine with no booted program (no ROM, or a refused one) runs no CPU and draws
    /// nothing: the frame only advances the counter and leaves both screens black.
    // TODO(hw-unverified): the per-frame instruction counts are a model (`docs/hw/scheduler.md`).
    pub fn run_frame(&mut self) {
        if self.header.is_none() {
            self.frame_count = self.frame_count.wrapping_add(1);
            return;
        }
        let started = self.frame_count;
        while self.frame_count == started {
            scheduler::run_round(self);
        }
    }

    /// Draws both screens from the registers and VRAM as they are now.
    ///
    /// Engine A goes to the screen POWCNT1 bit 15 picks; the other screen is a black
    /// placeholder for engine B (`docs/hw/display-mode-vram.md`).
    // TODO(phase4): engine B draws its screen.
    // TODO(phase3): per-scanline drawing replaces the whole-frame render; the line timing it
    // needs is in `docs/hw/display-timing.md` (#104).
    pub(crate) fn render_frame(&mut self) {
        let (engine_a, engine_b) = if self.io.gpu2d().engine_a_on_top() {
            (&mut self.top, &mut self.bottom)
        } else {
            (&mut self.bottom, &mut self.top)
        };
        self.io
            .gpu2d()
            .render_engine_a(self.memory.vram_a_lcdc(), engine_a);
        engine_b.fill(gpu2d::BLACK);
    }

    /// VCOUNT: the current line, 0-262 (`docs/hw/display-timing.md`).
    // The debugger's IO register panel and the scheduler timeline (#59) will show it.
    pub fn vcount(&self) -> u16 {
        self.io.timing().vcount()
    }

    /// DISPSTAT as the ARM9 reads it.
    // The debugger's IO register panel will show it.
    pub fn dispstat9(&self) -> u16 {
        self.io.timing().dispstat::<true>()
    }

    /// DISPSTAT as the ARM7 reads it.
    // The debugger's IO register panel will show it.
    pub fn dispstat7(&self) -> u16 {
        self.io.timing().dispstat::<false>()
    }

    /// Engine A's display registers.
    // The debugger's IO register panel will show them.
    pub fn gpu2d(&self) -> &gpu2d::Gpu2d {
        self.io.gpu2d()
    }

    /// Returns how many frames [`Machine::run_frame`] has completed.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// The ARM9 core.
    // The debugger's CPU panel (#56) will show it.
    pub fn arm9(&self) -> &Cpu<true> {
        &self.arm9
    }

    /// The ARM7 core.
    // The debugger's scheduler event timeline (#59) will show it with the scheduler.
    pub fn arm7(&self) -> &Cpu<false> {
        &self.arm7
    }

    /// The ARM7's clock in master cycles (ARM9 cycles).
    pub fn arm7_clock(&self) -> u64 {
        self.arm7_clock
    }

    /// The master clock and the pending events.
    // The debugger's scheduler event timeline (#59) will show it.
    pub fn scheduler(&self) -> &scheduler::Scheduler {
        &self.scheduler
    }

    /// The RAM both CPUs see.
    // The debugger's memory viewer (#57) will show it.
    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    /// The register devices: display, interrupt controllers, IPC and timers.
    // The debugger's IO register panel will show them.
    pub fn io(&self) -> &Io {
        &self.io
    }

    /// VRAMCNT_A as decoded.
    // The debugger's VRAM bank map (#58) will show it.
    pub fn vram_a(&self) -> VramBankState {
        self.memory.vram_a()
    }

    /// WRAMCNT as decoded: which 16 KiB halves of the shared WRAM each CPU owns.
    // The debugger's memory viewer (#57) shows each CPU's window from it.
    pub fn wramcnt(&self) -> WramState {
        self.memory.wramcnt()
    }

    /// The ARM9's interrupt controller (IME, IE, IF).
    // The debugger's CPU panel (#56) shows it beside the ARM9's registers.
    pub fn irq9(&self) -> &InterruptController<true> {
        self.io.irq9()
    }

    /// The ARM7's interrupt controller (IME, IE, IF).
    // The debugger's CPU panel (#56) shows it beside the ARM7's registers.
    pub fn irq7(&self) -> &InterruptController<false> {
        self.io.irq7()
    }

    /// IPCSYNC and the two IPC FIFOs.
    // The debugger's IPC panel (#56 CPU panel until one exists) shows the sync nibbles, both
    // FIFOs' contents and the flags of both CPUs.
    pub fn ipc(&self) -> &ipc::Ipc {
        self.io.ipc()
    }

    /// The ARM9's four timers.
    // The debugger's IO register panel (#56 CPU panel until one exists) shows each timer's
    // counter, reload and control; the scheduler timeline (#59) shows their overflow events.
    pub fn timers9(&self) -> &Timers<true> {
        self.io.timers9()
    }

    /// The ARM7's four timers.
    // The debugger's IO register panel shows them with the ARM9's; the scheduler timeline (#59)
    // shows their overflow events.
    pub fn timers7(&self) -> &Timers<false> {
        self.io.timers7()
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
            scheduler::settle(self);
        }
    }

    /// One ARM9 instruction and its cost, with nothing else. If the controller's line was
    /// asserted before it and CPSR I is clear, the IRQ entry takes the instruction's place and
    /// costs the same (`docs/hw/interrupts.md`, "Behavior"). The DS has no FIQ source.
    // TODO(hw-unverified): the line is sampled before each instruction and an IME or IE write
    // reaches it at once (`docs/hw/interrupts.md`, "Unverified").
    fn step_arm9_instruction(&mut self) {
        let irq = self.io.irq9().line();
        let now = self.scheduler.now();
        let tcm = TcmConfig::from_cp15(self.arm9.cp15());
        let mut view = Arm9View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            tcm,
            now,
        );
        if !self.arm9.take_interrupt(&mut view, irq, false) {
            self.arm9.step(&mut view);
        }
        self.scheduler.advance(ARM9_INSTRUCTION_COST);
    }

    /// One ARM7 instruction and its cost, with nothing else; the IRQ entry takes its place as in
    /// [`Machine::step_arm9_instruction`].
    // TODO(hw-unverified): the line is sampled before each instruction and an IME or IE write
    // reaches it at once (`docs/hw/interrupts.md`, "Unverified").
    fn step_arm7_instruction(&mut self) {
        let irq = self.io.irq7().line();
        let mut view = Arm7View::new(
            &mut self.memory,
            &mut self.io,
            &mut self.scheduler,
            self.arm7_clock,
        );
        if !self.arm7.take_interrupt(&mut view, irq, false) {
            self.arm7.step(&mut view);
        }
        self.arm7_clock = self.arm7_clock.saturating_add(ARM7_INSTRUCTION_COST);
    }

    /// Reads `N` bytes as the ARM9 data bus would, without side effects: IPCFIFORECV shows its
    /// next word without taking it, and nothing needs the scheduler.
    fn arm9_peek<const N: usize>(&self, addr: u32) -> [u8; N] {
        bus::peek::<true, N>(
            &self.memory,
            &self.io,
            &TcmConfig::from_cp15(self.arm9.cp15()),
            self.scheduler.now(),
            addr,
        )
    }

    /// Reads a byte as the ARM9 data bus would, without side effects.
    // The debugger's memory viewer (#57) and the runner's boot memory-dump test use it.
    pub fn arm9_peek8(&self, addr: u32) -> u8 {
        u8::from_le_bytes(self.arm9_peek(addr))
    }

    /// Reads a halfword as the ARM9 data bus would, without side effects.
    // The debugger's memory viewer (#57) and the runner's boot memory-dump test use it.
    pub fn arm9_peek16(&self, addr: u32) -> u16 {
        u16::from_le_bytes(self.arm9_peek(addr))
    }

    /// Reads a word as the ARM9 data bus would, without side effects.
    // The debugger's memory viewer (#57) and the runner's boot memory-dump test use it.
    pub fn arm9_peek32(&self, addr: u32) -> u32 {
        u32::from_le_bytes(self.arm9_peek(addr))
    }

    /// Returns the pixels of one screen, row by row, as xRGB8888.
    pub fn frame(&self, screen: Screen) -> &[u32] {
        match screen {
            Screen::Top => &self.top,
            Screen::Bottom => &self.bottom,
        }
    }
}

impl Participants for Machine {
    fn scheduler(&mut self) -> &mut Scheduler {
        &mut self.scheduler
    }

    fn run_arm9_until(&mut self, target: u64) {
        while self.scheduler.now() < target {
            self.step_arm9_instruction();
        }
    }

    fn run_arm7_until(&mut self, target: u64) {
        while self.arm7_clock < target {
            self.step_arm7_instruction();
        }
    }

    fn dispatch(&mut self, event: Event) {
        match event.kind {
            EventKind::LineStart => self.line_started(event.time),
            EventKind::HBlankStart => self.hblank_started(event.time),
            EventKind::TimerOverflow { arm9, index } => {
                self.io.timer_overflowed(&mut self.scheduler, arm9, index);
            }
        }
    }
}

impl Machine {
    /// A line begins at `time` (`docs/hw/display-timing.md`). VBlank starts here too, when the
    /// line is the first blank one; this is the one place for it.
    ///
    /// The start of line 0 ends the frame: the screens are drawn and the frame is counted.
    // TODO(phase3): VBlank-start DMA (#26) and the IntrWait VBlank wait (#27) hook in here.
    fn line_started(&mut self, time: u64) {
        let requests = self.io.timing_mut().line_started();
        self.io.request_irq(requests.arm9, requests.arm7);
        if self.io.timing().vcount() == 0 {
            self.render_frame();
            self.frame_count = self.frame_count.wrapping_add(1);
        }
        // From the event's own time, not `now`, so a line's length never drifts.
        self.scheduler
            .schedule(time.saturating_add(LINE_CYCLES), EventKind::LineStart);
    }

    /// HBlank begins at `time` in the current line (`docs/hw/display-timing.md`); the one place
    /// for it.
    // TODO(phase3): HBlank-start DMA (#26) hooks in here.
    fn hblank_started(&mut self, time: u64) {
        let requests = self.io.timing_mut().hblank_started();
        self.io.request_irq(requests.arm9, requests.arm7);
        self.scheduler
            .schedule(time.saturating_add(LINE_CYCLES), EventKind::HBlankStart);
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::{Error, MapDeserializer};

    fn run_frames(machine: &mut Machine, frames: u32) {
        (0..frames).for_each(|_| machine.run_frame());
    }

    #[test]
    fn given_new_machine_when_reading_each_screen_then_length_is_256_by_192() {
        let machine = Machine::new();
        let lengths = [
            machine.frame(Screen::Top).len(),
            machine.frame(Screen::Bottom).len(),
        ];
        assert_eq!(lengths, [49_152, 49_152]);
    }

    #[test]
    fn given_new_machine_when_reading_frames_then_every_pixel_is_black() {
        let machine = Machine::default();
        let all_black = [Screen::Top, Screen::Bottom]
            .iter()
            .all(|&s| machine.frame(s).iter().all(|&p| p == 0));
        assert!(all_black);
    }

    #[test]
    fn given_saved_machine_when_loaded_onto_a_fresh_one_then_both_screens_are_full_size() {
        let mut saved = Machine::new();
        saved.run_frame();
        let mut machine = Machine::new();
        let result = machine.load_state(&saved.save_state());
        let lengths = [
            machine.frame(Screen::Top).len(),
            machine.frame(Screen::Bottom).len(),
        ];
        assert_eq!((result, lengths), (Ok(()), [49_152, 49_152]));
    }

    #[test]
    fn given_rom_bytes_when_loading_then_rom_is_kept_verbatim() {
        let mut machine = Machine::new();
        let image = cart::test_image::Image::valid().build();
        machine.load_rom(image.clone()).expect("valid image");
        assert_eq!(machine.rom(), image.as_slice());
    }

    #[test]
    fn given_six_byte_rom_when_loading_then_it_is_refused_as_too_short_and_not_kept() {
        let mut machine = Machine::new();
        let result = machine.load_rom(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
        assert_eq!(
            (result, machine.rom().len()),
            (Err(cart::HeaderError::ImageTooShort { len: 6 }), 0)
        );
    }

    #[test]
    fn given_new_machine_when_running_one_frame_then_frame_count_is_one() {
        let mut machine = Machine::new();
        machine.run_frame();
        assert_eq!(machine.frame_count(), 1);
    }

    #[test]
    fn given_new_machine_when_reading_counters_then_frame_count_is_zero_and_rom_is_empty() {
        let machine = Machine::new();
        assert_eq!((machine.frame_count(), machine.rom().len()), (0, 0));
    }

    #[test]
    fn given_empty_rom_when_running_a_frame_then_it_is_refused_rom_is_empty_and_frame_count_is_one()
    {
        let mut machine = Machine::new();
        let result = machine.load_rom(Vec::new());
        machine.run_frame();
        assert_eq!(
            (result, machine.rom().len(), machine.frame_count()),
            (Err(cart::HeaderError::ImageTooShort { len: 0 }), 0, 1)
        );
    }

    #[test]
    fn given_odd_length_rom_when_running_a_frame_then_it_is_refused_rom_is_empty_and_frame_count_is_one()
     {
        let mut machine = Machine::new();
        let result = machine.load_rom(vec![1, 2, 3]);
        machine.run_frame();
        assert_eq!(
            (result, machine.rom().len(), machine.frame_count()),
            (Err(cart::HeaderError::ImageTooShort { len: 3 }), 0, 1)
        );
    }

    #[test]
    fn given_three_frames_run_when_reading_frame_count_then_it_is_three() {
        let mut machine = Machine::new();
        machine.run_frame();
        machine.run_frame();
        machine.run_frame();
        assert_eq!(machine.frame_count(), 3);
    }

    #[test]
    fn given_machine_that_ran_7_frames_when_saved_and_loaded_then_rom_is_empty_and_frame_count_is_seven()
     {
        let mut saved = Machine::new();
        run_frames(&mut saved, 7);
        let mut machine = Machine::new();
        let result = machine.load_state(&saved.save_state());
        assert_eq!(
            (result, machine.rom().len(), machine.frame_count()),
            (Ok(()), 0, 7)
        );
    }

    #[test]
    fn given_savestate_map_with_only_frame_count_when_deserializing_machine_then_it_is_refused() {
        let map = MapDeserializer::<_, Error>::new([("frame_count", 7u32)].into_iter());
        assert!(Machine::deserialize(map).is_err());
    }

    #[test]
    fn given_savestate_map_without_frame_count_when_deserializing_machine_then_it_is_refused() {
        let map = MapDeserializer::<_, Error>::new(std::iter::empty::<(&str, u32)>());
        assert!(Machine::deserialize(map).is_err());
    }

    #[test]
    fn given_machine_that_ran_two_frames_when_loading_rom_then_frame_count_is_zero() {
        let mut machine = Machine::new();
        machine.run_frame();
        machine.run_frame();
        machine
            .load_rom(cart::test_image::Image::valid().build())
            .expect("valid image");
        assert_eq!(machine.frame_count(), 0);
    }
}

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests_arm9;
#[cfg(test)]
mod tests_ipc;
#[cfg(test)]
mod tests_irq;
#[cfg(test)]
mod tests_scheduler;
#[cfg(test)]
mod tests_timers;
#[cfg(test)]
mod tests_timing;
