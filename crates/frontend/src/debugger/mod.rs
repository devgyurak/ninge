// SPDX-License-Identifier: Apache-2.0
//! The debugger's data path through the thread split (#322, part 5; panels #56-#59).
//!
//! [`ninge_core::debug::Inspect`] borrows the machine, which lives on the emu thread, so a panel
//! cannot hold it. The emu thread fills plain-data view models from it ([`DebugView::fill`]) and
//! publishes them inside the [`crate::render::Snapshot`] through the same hand-off as the screens;
//! the render thread draws from the snapshot alone. What a panel owns — which panel shows, how far
//! it is scrolled — is [`PanelState`], kept in [`crate::app::App`] and copied into the snapshot, so
//! the logic here needs no terminal.
//!
//! A view is filled only while its panel is on screen ([`Shown`]): the emu thread pays for what is
//! drawn and nothing else, which a memory or VRAM viewer (#57-#59), far costlier than 24
//! instructions, needs before it is added. [`PanelState::panel`] is the panel that has the focus,
//! and the scroll applies to the disassembly.

use ninge_core::Machine;
use ninge_core::debug::{Disassembled, Inspect};

/// How many instructions the disassembly view holds.
pub const DISASM_LINES: usize = 24;

/// The debugger panels this module has data for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Panel {
    /// The two CPUs' registers.
    #[default]
    Registers,
    /// The ARM9's instructions around the next one to run.
    Disassembly,
}

/// What the panels own: which one has the focus and how far the disassembly is scrolled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PanelState {
    /// The panel with the focus.
    pub panel: Panel,
    /// Instructions the disassembly starts past the next one to run (negative is before it).
    pub scroll: i32,
}

impl PanelState {
    /// Gives the focus to the other panel.
    pub fn next_panel(&mut self) {
        self.panel = match self.panel {
            Panel::Registers => Panel::Disassembly,
            Panel::Disassembly => Panel::Registers,
        };
    }

    /// Scrolls the disassembly by `rows` instructions (negative is up) when it has the focus; the
    /// registers panel has nothing to scroll.
    pub fn scroll_by(&mut self, rows: i32) {
        if self.panel == Panel::Disassembly {
            self.scroll = self.scroll.saturating_add(rows);
        }
    }

    /// Back to the next instruction to run.
    pub fn recenter(&mut self) {
        self.scroll = 0;
    }
}

/// One CPU as the registers panel shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuView {
    /// r0-r15 in the current mode.
    pub regs: [u32; 16],
    /// The CPSR.
    pub cpsr: u32,
    /// Whether the CPU is halted.
    pub halted: bool,
    /// The address of the instruction that runs next.
    pub next_pc: u32,
}

impl CpuView {
    /// The view of `cpu`'s registers, as `Inspect` gives them.
    fn of<const ARM9: bool>(cpu: &ninge_core::cpu::Cpu<ARM9>) -> Self {
        Self {
            regs: std::array::from_fn(|n| cpu.regs().get(n)),
            cpsr: cpu.regs().cpsr(),
            halted: cpu.halted(),
            next_pc: cpu.next_instruction(),
        }
    }
}

/// Both CPUs, for the registers panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistersView {
    /// The ARM9.
    pub arm9: CpuView,
    /// The ARM7.
    pub arm7: CpuView,
}

/// Which panels are on screen this frame, so which views a fill pays for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// The registers panel.
    pub registers: bool,
    /// The disassembly panel.
    pub disasm: bool,
}

impl Shown {
    /// Every panel on screen.
    pub const ALL: Self = Self {
        registers: true,
        disasm: true,
    };

    /// No panel on screen (the terminal has no room, or no area to draw them in).
    pub const NONE: Self = Self {
        registers: false,
        disasm: false,
    };

    /// Every panel when `on`, else none.
    pub const fn when(on: bool) -> Self {
        match on {
            true => Self::ALL,
            false => Self::NONE,
        }
    }
}

/// The ARM9's instructions, for the disassembly panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisasmView {
    /// The lines, the first being `scroll` instructions past the next one. Each is plain `Copy`
    /// data from `Inspect`; the text is formatted by the renderer when it draws.
    pub lines: Vec<Disassembled>,
}

/// What the debugger panels show of one frame, as plain data for the render thread.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugView {
    /// Which panel has the focus and how far the disassembly is scrolled.
    pub panel: PanelState,
    /// Whether a machine is behind the panels; with the dummy pattern they are empty.
    pub present: bool,
    /// Whether the run is stopped by the debugger ([`crate::app::App::broken`]).
    pub broken: bool,
    /// The registers of both CPUs.
    pub registers: RegistersView,
    /// The disassembly of the ARM9.
    pub disasm: DisasmView,
}

impl DebugView {
    /// Fills the view for `state` from `machine` (`None` while the dummy pattern is shown: the
    /// panels are then empty), only the panels `shown` says are on screen. Nothing allocates once
    /// the lines' `Vec` has grown to [`DISASM_LINES`].
    pub fn fill(
        &mut self,
        machine: Option<&Machine>,
        state: PanelState,
        broken: bool,
        shown: Shown,
    ) {
        self.panel = state;
        self.broken = broken;
        self.present = machine.is_some();
        self.registers = RegistersView::default();
        self.disasm.lines.clear();
        let Some(machine) = machine else {
            return;
        };
        let inspect = machine.inspect();
        if shown.registers {
            self.registers = RegistersView {
                arm9: CpuView::of(inspect.arm9()),
                arm7: CpuView::of(inspect.arm7()),
            };
        }
        if shown.disasm {
            self.fill_disasm(&inspect, state.scroll);
        }
    }

    /// The ARM9's instructions, `scroll` past the next one, in its current instruction set.
    fn fill_disasm(&mut self, inspect: &Inspect<'_>, scroll: i32) {
        self.disasm
            .lines
            .extend(inspect.disassemble_arm9(scroll, DISASM_LINES));
    }
}

#[cfg(test)]
mod tests;
