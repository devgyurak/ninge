// SPDX-License-Identifier: Apache-2.0
//! The debugger's view models: what the emu thread fills from `Inspect`, and the panel state.

use super::*;

fn smoke_machine() -> Machine {
    let rom = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testroms/build/smoke.nds"),
    )
    .expect("smoke ROM is built");
    crate::source::boot(rom).expect("valid header")
}

fn disassembly_focus() -> PanelState {
    PanelState {
        panel: Panel::Disassembly,
        scroll: 0,
    }
}

#[test]
fn given_the_registers_panel_when_the_next_panel_is_asked_then_the_disassembly_has_the_focus() {
    let mut state = PanelState::default();
    state.next_panel();
    assert_eq!(state.panel, Panel::Disassembly);
}

#[test]
fn given_the_disassembly_panel_when_the_next_panel_is_asked_then_the_registers_have_the_focus() {
    let mut state = disassembly_focus();
    state.next_panel();
    assert_eq!(state.panel, Panel::Registers);
}

#[test]
fn given_the_disassembly_focus_when_scrolled_down_and_up_then_the_scroll_follows() {
    let mut state = disassembly_focus();
    state.scroll_by(10);
    state.scroll_by(-3);
    assert_eq!(state.scroll, 7);
}

#[test]
fn given_the_registers_focus_when_scrolled_then_nothing_moves() {
    let mut state = PanelState::default();
    state.scroll_by(10);
    assert_eq!(state.scroll, 0);
}

#[test]
fn given_a_scroll_at_the_limit_when_scrolled_further_then_it_saturates_without_wrapping() {
    let mut state = PanelState {
        panel: Panel::Disassembly,
        scroll: i32::MAX,
    };
    state.scroll_by(1);
    assert_eq!(state.scroll, i32::MAX);
}

#[test]
fn given_a_scrolled_disassembly_when_recentered_then_the_scroll_is_zero() {
    let mut state = PanelState {
        panel: Panel::Disassembly,
        scroll: 40,
    };
    state.recenter();
    assert_eq!(state.scroll, 0);
}

#[test]
fn given_no_machine_when_filled_then_the_view_is_empty_and_not_present() {
    let mut view = DebugView::default();
    view.fill(None, PanelState::default(), false, Shown::ALL);
    assert_eq!(view, DebugView::default());
}

#[test]
fn given_a_machine_when_filled_then_the_registers_are_what_inspect_says() {
    let machine = smoke_machine();
    let mut view = DebugView::default();
    view.fill(Some(&machine), PanelState::default(), false, Shown::ALL);
    let inspect = machine.inspect();
    assert_eq!(
        (
            view.registers.arm9.next_pc,
            view.registers.arm7.next_pc,
            view.present
        ),
        (
            inspect.arm9().next_instruction(),
            inspect.arm7().next_instruction(),
            true
        )
    );
}

#[test]
fn given_a_machine_when_filled_then_the_disassembly_starts_at_the_next_instruction() {
    let machine = smoke_machine();
    let mut view = DebugView::default();
    view.fill(Some(&machine), PanelState::default(), false, Shown::ALL);
    assert_eq!(
        (view.disasm.lines.len(), view.disasm.lines[0].addr),
        (DISASM_LINES, machine.inspect().arm9().next_instruction())
    );
}

#[test]
fn given_a_scroll_of_three_when_filled_in_arm_state_then_the_first_line_is_twelve_bytes_on() {
    let machine = smoke_machine();
    let mut state = disassembly_focus();
    state.scroll_by(3);
    let mut view = DebugView::default();
    view.fill(Some(&machine), state, false, Shown::ALL);
    assert_eq!(
        view.disasm.lines[0].addr,
        machine.inspect().arm9().next_instruction().wrapping_add(12)
    );
}

#[test]
fn given_a_machine_when_filled_then_each_line_holds_the_word_memory_has_there() {
    let machine = smoke_machine();
    let mut view = DebugView::default();
    view.fill(Some(&machine), PanelState::default(), false, Shown::ALL);
    let line = &view.disasm.lines[1];
    assert_eq!(line.word, machine.inspect().arm9_peek32(line.addr));
}

#[test]
fn given_a_filled_view_when_filled_again_then_the_lines_buffer_is_reused() {
    let machine = smoke_machine();
    let mut view = DebugView::default();
    view.fill(Some(&machine), PanelState::default(), false, Shown::ALL);
    let before = view.disasm.lines.as_ptr();
    view.fill(Some(&machine), PanelState::default(), false, Shown::ALL);
    assert_eq!(view.disasm.lines.as_ptr(), before);
}

#[test]
fn given_a_broken_run_when_filled_then_the_view_says_so() {
    let mut view = DebugView::default();
    view.fill(None, PanelState::default(), true, Shown::ALL);
    assert!(view.broken);
}

#[test]
fn given_no_panel_shown_when_filled_then_the_machine_is_present_but_no_view_is_filled() {
    let machine = smoke_machine();
    let mut view = DebugView::default();
    view.fill(Some(&machine), PanelState::default(), false, Shown::NONE);
    assert_eq!(
        (view.present, view.registers, view.disasm.lines.len()),
        (true, RegistersView::default(), 0)
    );
}

#[test]
fn given_only_the_registers_shown_when_filled_then_the_disassembly_stays_empty() {
    let machine = smoke_machine();
    let mut view = DebugView::default();
    let shown = Shown {
        registers: true,
        disasm: false,
    };
    view.fill(Some(&machine), PanelState::default(), false, shown);
    assert_eq!(
        (view.registers.arm9.next_pc, view.disasm.lines.len()),
        (machine.inspect().arm9().next_instruction(), 0)
    );
}

#[test]
fn given_only_the_disassembly_shown_when_filled_then_the_registers_stay_empty() {
    let machine = smoke_machine();
    let mut view = DebugView::default();
    let shown = Shown {
        registers: false,
        disasm: true,
    };
    view.fill(Some(&machine), PanelState::default(), false, shown);
    assert_eq!(
        (view.registers, view.disasm.lines.len()),
        (RegistersView::default(), DISASM_LINES)
    );
}

#[test]
fn given_a_flag_when_a_shown_set_is_made_from_it_then_all_or_none_are_shown() {
    assert_eq!(
        (Shown::when(true), Shown::when(false)),
        (Shown::ALL, Shown::NONE)
    );
}
