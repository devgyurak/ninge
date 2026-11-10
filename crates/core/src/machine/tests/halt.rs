// SPDX-License-Identifier: Apache-2.0
//! Halt and the wait SWIs through `Machine`: the ARM9's CP15 wait-for-interrupt, the ARM7's
//! HALTCNT, SWI Halt, WaitByLoop, IntrWait and VBlankIntrWait. Expected values come from
//! `docs/hw/hle-swi.md` ("Halt, and what wakes a CPU", "The wait SWIs"), `docs/hw/cp15.md` ("C7"),
//! `docs/hw/interrupts.md` ("HALTCNT", "Halt") and `docs/hw/scheduler.md` ("Interleaving"),
//! never from running the code.
//!
//! The programs are hand-assembled; the comments name each word. The ARM9 runs with the high
//! vectors (the BIOS), a DTCM at 0x027C0000 for the handler pointer and the IRQ check flags, and
//! its IRQ stack at `SP_IRQ9`; the ARM7 uses its fixed addresses.

use super::bios::{
    BX_LR, DTCM_BASE, ENTRY9, HANDLER, HANDLER7, IE, SP_IRQ9, SYSTEM_FLAGS, SYSTEM_I_CLEAR, TIMER0,
    arm_swi, arm7_running, arm9_running, machine_with, request_timer0_on_arm9, step7, step9,
};
use crate::cpu::{Bank, Bus};
use crate::io::irq::Requests;
use crate::machine::*;
use crate::scheduler::run_round;

/// `mcr p15, 0, r0, c7, c0, 4` and `mcr p15, 0, r0, c7, c8, 2`: the two wait-for-interrupt forms.
const WFI_C0_4: u32 = 0xEE07_0F90;
const WFI_C8_2: u32 = 0xEE07_0F58;
/// `mcr p15, 0, r0, c7, c5, 0`: invalidate the instruction cache, which halts nothing.
const INVALIDATE_ICACHE: u32 = 0xEE07_0F15;
/// `mov r1, #0x66` and `b .`.
const MOV_R1: u32 = 0xE3A0_1066;
const SPIN: u32 = 0xEAFF_FFFE;
/// `mov r2, #0x66`: the marker the ARM7 HALTCNT programs run after the halt.
const MOV_R2: u32 = 0xE3A0_2066;
/// `mov r6, #0x99`, run when a wait returns.
const MOV_R6: u32 = 0xE3A0_6099;

/// The IRQ check flags: the ARM9's at DTCM + 3FF8h, the ARM7's at 0380FFF8h.
const FLAGS9: u32 = DTCM_BASE + 0x3FF8;
const FLAGS7: u32 = 0x0380_FFF8;
const VBLANK: u32 = 1;

// ----------------------------------------------------------------------- CP15 wait-for-interrupt

#[test]
fn given_an_arm9_wfi_at_c7_c0_4_when_executed_then_the_arm9_is_halted() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    step9(&mut m, 1);
    assert!(m.arm9().halted());
}

#[test]
fn given_an_arm9_wfi_at_c7_c8_2_when_executed_then_the_arm9_is_halted() {
    let mut m = arm9_running(&[WFI_C8_2, MOV_R1, SPIN]);
    step9(&mut m, 1);
    assert!(m.arm9().halted());
}

#[test]
fn given_a_cache_command_in_c7_when_executed_then_the_arm9_is_not_halted() {
    let mut m = arm9_running(&[INVALIDATE_ICACHE, MOV_R1, SPIN]);
    step9(&mut m, 1);
    assert!(!m.arm9().halted());
}

#[test]
fn given_a_halted_arm9_with_nothing_pending_when_a_round_runs_then_no_instruction_runs() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    step9(&mut m, 1);
    run_round(&mut m);
    assert_eq!((m.arm9().regs().get(1), m.arm9().halted()), (0, true));
}

#[test]
fn given_a_halted_arm9_when_a_round_runs_then_its_clock_skips_to_the_round_target() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    step9(&mut m, 1);
    let started = m.scheduler().now();
    run_round(&mut m);
    // The round's target is one slice (64 master cycles) ahead: no event is nearer.
    assert_eq!(m.scheduler().now() - started, 64);
}

/// The master clock after each of `rounds` rounds.
fn clocks_after_rounds(m: &mut Machine, rounds: usize) -> Vec<u64> {
    (0..rounds)
        .map(|_| {
            run_round(m);
            m.scheduler().now()
        })
        .collect()
}

#[test]
fn given_a_halted_arm9_when_rounds_run_then_the_clock_lands_on_the_first_hblank_event() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    step9(&mut m, 1);
    // HBlank starts at master cycle 3212 of the first line (`display-timing.md`): no round steps
    // over it, so one of them ends exactly there.
    assert!(clocks_after_rounds(&mut m, 60).contains(&3_212));
}

#[test]
fn given_a_halted_arm9_when_ie_and_if_are_nonzero_with_ime_0_and_i_set_then_it_wakes_and_runs_on() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    m.arm9.regs_mut().set_cpsr(0x9F);
    step9(&mut m, 1);
    m.bus9().write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
    step9(&mut m, 1);
    assert_eq!((m.arm9().halted(), m.arm9().regs().get(1)), (false, 0x66));
}

#[test]
fn given_a_halted_arm9_when_ie_and_if_are_nonzero_with_ime_1_and_i_clear_then_the_irq_is_taken() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    step9(&mut m, 1);
    request_timer0_on_arm9(&mut m);
    step9(&mut m, 1);
    let regs = m.arm9().regs();
    assert_eq!(
        (m.arm9().halted(), regs.get(1), regs.cpsr() & 0x1F),
        (false, 0, 0x12)
    );
}

#[test]
fn given_a_halted_arm9_when_if_is_set_but_ie_is_not_then_it_stays_halted() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    step9(&mut m, 1);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
    step9(&mut m, 2);
    assert_eq!((m.arm9().halted(), m.arm9().regs().get(1)), (true, 0));
}

#[test]
fn given_an_irq_already_pending_and_enabled_when_the_arm9_executes_wfi_then_it_wakes_at_once() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    m.arm9.regs_mut().set_cpsr(0x9F);
    m.bus9().write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
    step9(&mut m, 3);
    assert_eq!((m.arm9().halted(), m.arm9().regs().get(1)), (false, 0x66));
}

#[test]
fn given_a_halted_arm9_when_single_stepped_then_the_clock_advances_by_one_cost_unit() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    step9(&mut m, 1);
    let before = m.scheduler().now();
    step9(&mut m, 1);
    assert_eq!(m.scheduler().now() - before, 1);
}

// ------------------------------------------------------------------------------ ARM7 HALTCNT

/// `mov r0, #0x04000000`, `mov r1, #value`, `strb r1, [r0, #0x301]`, then `mov r2, #0x66`.
fn haltcnt_program(value: u32) -> [u32; 5] {
    [0xE3A0_0301, 0xE3A0_1000 | value, 0xE5C0_1301, MOV_R2, SPIN]
}

fn arm7_after_haltcnt_write(value: u32) -> Machine {
    let mut m = arm7_running(&haltcnt_program(value));
    step7(&mut m, 3);
    m
}

#[test]
fn given_a_haltcnt_write_of_80h_when_the_arm7_stores_it_then_the_arm7_is_halted() {
    assert!(arm7_after_haltcnt_write(0x80).arm7().halted());
}

#[test]
fn given_a_haltcnt_write_with_bits_0_to_5_set_when_the_mode_is_2_then_the_arm7_is_halted() {
    assert!(arm7_after_haltcnt_write(0xBF).arm7().halted());
}

#[test]
fn given_haltcnt_modes_0_1_and_3_when_written_then_the_arm7_is_not_halted() {
    let halted: Vec<bool> = [0x00, 0x40, 0xC0]
        .iter()
        .map(|&value| arm7_after_haltcnt_write(value).arm7().halted())
        .collect();
    assert_eq!(halted, [false, false, false]);
}

#[test]
fn given_the_arm9_when_it_writes_0400_0301h_then_nothing_halts() {
    let mut m = arm9_running(&haltcnt_program(0x80));
    step9(&mut m, 3);
    assert_eq!((m.arm9().halted(), m.arm7().halted()), (false, false));
}

#[test]
fn given_a_halted_arm7_when_a_round_runs_then_its_clock_reaches_the_arm9_clock() {
    let mut m = arm7_after_haltcnt_write(0x80);
    run_round(&mut m);
    assert_eq!(m.arm7_clock(), m.scheduler().now());
}

#[test]
fn given_a_halted_arm7_when_ie_and_if_are_nonzero_with_ime_0_then_it_wakes_and_runs_on() {
    let mut m = arm7_after_haltcnt_write(0x80);
    m.bus7(0).write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: TIMER0,
    });
    step7(&mut m, 1);
    assert_eq!((m.arm7().halted(), m.arm7().regs().get(2)), (false, 0x66));
}

#[test]
fn given_a_halted_arm7_with_nothing_pending_when_stepped_then_it_stays_halted() {
    let mut m = arm7_after_haltcnt_write(0x80);
    step7(&mut m, 3);
    assert_eq!((m.arm7().halted(), m.arm7().regs().get(2)), (true, 0));
}

#[test]
fn given_a_halted_cpu_when_a_state_is_saved_and_loaded_then_it_is_halted_again() {
    let mut m = arm7_after_haltcnt_write(0x80);
    run_round(&mut m);
    let mut loaded = Machine::new();
    loaded.load_rom(m.rom().to_vec()).expect("same image");
    let result = loaded.load_state(&m.save_state());
    assert_eq!((result, loaded.arm7().halted()), (Ok(()), true));
}

// -------------------------------------------------------------------------------- SWI Halt, 03h

#[test]
fn given_swi_halt_when_executed_then_it_returns_and_the_cpu_is_halted() {
    let mut m = arm9_running(&[arm_swi(0x06_0000), MOV_R1, SPIN]);
    step9(&mut m, 2);
    let regs = m.arm9().regs();
    assert_eq!(
        (m.arm9().halted(), regs.cpsr(), regs.get(15), regs.get(1)),
        (true, SYSTEM_FLAGS, ENTRY9 + 4 + 8, 0)
    );
}

#[test]
fn given_swi_halt_on_the_arm9_when_an_enabled_irq_is_pending_then_the_next_instruction_runs() {
    let mut m = arm9_running(&[arm_swi(0x06_0000), MOV_R1, SPIN]);
    step9(&mut m, 2);
    m.bus9().write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
    step9(&mut m, 1);
    assert_eq!(m.arm9().regs().get(1), 0x66);
}

#[test]
fn given_swi_halt_on_the_arm7_when_executed_then_the_arm7_is_halted_and_nothing_else() {
    let mut m = arm7_running(&[arm_swi(0x06_0000), MOV_R1, SPIN]);
    step7(&mut m, 2);
    assert_eq!(
        (
            m.arm7().halted(),
            m.arm9().halted(),
            m.swi_record7().last_unimplemented()
        ),
        (true, false, None)
    );
}

#[test]
fn given_swi_waitbyloop_when_executed_then_it_returns_without_halting_and_is_not_unimplemented() {
    let mut m = arm9_running(&[arm_swi(0x03_0000), MOV_R1, SPIN]);
    step9(&mut m, 3);
    assert_eq!(
        (
            m.arm9().halted(),
            m.arm9().regs().get(1),
            m.swi_record9().last(),
            m.swi_record9().last_unimplemented(),
        ),
        (false, 0x66, Some(3), None)
    );
}

// ------------------------------------------------------------------- IntrWait, VBlankIntrWait

/// The user handler: `<prefix>`, then the flag word gets `bit`, IF bit `bit` is acknowledged (the
/// flag word is in the IE/IF format), `bx lr`. The prefix loads `r0` with the address 0xFF8 below
/// the flag word: `mov r0, #0x027C0000` and `add r0, r0, #0x3000` on the ARM9 (DTCM + 3000h),
/// `mov r0, #0x03800000` and `orr r0, r0, #0xF000` on the ARM7.
fn handler(prefix: &[u32], bit: u32) -> Vec<u32> {
    let [load, store] = [0xE590_1FF8, 0xE580_1FF8];
    prefix
        .iter()
        .copied()
        .chain([
            load,
            0xE381_1000 | bit, // orr r1, r1, #bit
            store,
            0xE3A0_2301,       // mov r2, #0x04000000
            0xE3A0_3000 | bit, // mov r3, #bit
            0xE582_3214,       // str r3, [r2, #0x214]
            BX_LR,
        ])
        .collect()
}

const ARM9_HANDLER_PREFIX: [u32; 2] = [0xE3A0_079F, 0xE280_0A03];
const ARM7_HANDLER_PREFIX: [u32; 2] = [0xE3A0_0638, 0xE380_0A0F];

/// `mov r0, #discard`, `mov r1, #flags`, `swi n`, `mov r6, #0x99`, `b .`.
fn wait_program(discard: u32, flags: u32, number: u32) -> [u32; 5] {
    [
        0xE3A0_0000 | discard,
        0xE3A0_1000 | flags,
        arm_swi(number << 16),
        MOV_R6,
        SPIN,
    ]
}

/// An ARM9 about to run `program` with the BIOS vectors, a DTCM holding the handler pointer and
/// the user handler that raises `bit` in the flag word.
fn arm9_waiting(program: &[u32], bit: u32) -> Machine {
    let mut m = arm9_running(program);
    m.arm9.write_cp15(9, 1, 0, 0x027C_000A);
    m.arm9.write_cp15(1, 0, 0, (1 << 13) | (1 << 16));
    m.bus9().write32(DTCM_BASE + 0x3FFC, HANDLER);
    handler(&ARM9_HANDLER_PREFIX, bit)
        .iter()
        .enumerate()
        .for_each(|(i, &word)| m.bus9().write32(HANDLER + 4 * i as u32, word));
    m.arm9.regs_mut().set_cpsr(SYSTEM_FLAGS | 0x80);
    m.arm9.regs_mut().set_banked(Bank::Irq, 13, SP_IRQ9);
    m
}

fn arm7_waiting(program: &[u32], bit: u32) -> Machine {
    let mut m = arm7_running(program);
    m.bus7(0).write32(0x0380_FFFC, HANDLER7);
    handler(&ARM7_HANDLER_PREFIX, bit)
        .iter()
        .enumerate()
        .for_each(|(i, &word)| m.bus7(0).write32(HANDLER7 + 4 * i as u32, word));
    m.arm7.regs_mut().set_cpsr(SYSTEM_FLAGS | 0x80);
    m
}

/// Raises the timer 0 request for the ARM9 and enables it, as the source the handler serves.
fn raise_arm9(m: &mut Machine) {
    m.bus9().write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
}

fn raise_arm7(m: &mut Machine) {
    m.bus7(0).write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: TIMER0,
    });
}

#[test]
fn given_intrwait_with_nothing_set_when_executed_then_the_arm9_halts_in_the_wait_loop() {
    let mut m = arm9_waiting(&wait_program(0, 8, 4), 8);
    step9(&mut m, 5);
    let regs = m.arm9().regs();
    assert_eq!(
        (
            m.arm9().halted(),
            regs.cpsr() & 0x1F,
            regs.cpsr() & 0x80,
            regs.get(6)
        ),
        (true, 0x13, 0, 0)
    );
}

#[test]
fn given_intrwait_when_executed_then_ime_is_forced_to_1() {
    let mut m = arm9_waiting(&wait_program(0, 8, 4), 8);
    step9(&mut m, 4);
    assert!(m.irq9().ime());
}

#[test]
fn given_intrwait_halted_when_the_enabled_irq_arrives_then_the_handler_runs_and_the_wait_returns() {
    let mut m = arm9_waiting(&wait_program(0, 8, 4), 8);
    step9(&mut m, 5);
    raise_arm9(&mut m);
    step9(&mut m, 40);
    let regs = m.arm9().regs();
    assert_eq!(
        (
            regs.get(6),
            regs.cpsr(),
            m.arm9().halted(),
            m.arm9_peek32(FLAGS9)
        ),
        (0x99, SYSTEM_FLAGS | 0x80, false, 0)
    );
}

#[test]
fn given_intrwait_returning_when_the_flag_word_held_other_bits_then_only_the_waited_bits_clear() {
    let mut m = arm9_waiting(&wait_program(0, 8, 4), 8);
    m.bus9().write32(FLAGS9, 0x108);
    step9(&mut m, 5);
    assert_eq!((m.arm9().regs().get(6), m.arm9_peek32(FLAGS9)), (0, 0x100));
}

#[test]
fn given_intrwait_with_the_flag_already_set_and_r0_0_when_executed_then_it_returns_at_once() {
    let mut m = arm9_waiting(&wait_program(0, 8, 4), 8);
    m.bus9().write32(FLAGS9, 8);
    step9(&mut m, 6);
    assert_eq!(
        (
            m.arm9().regs().get(6),
            m.arm9().halted(),
            m.arm9_peek32(FLAGS9)
        ),
        (0x99, false, 0)
    );
}

#[test]
fn given_intrwait_with_r0_1_when_old_flags_are_set_then_they_are_discarded_and_it_waits() {
    let mut m = arm9_waiting(&wait_program(1, 8, 4), 8);
    m.bus9().write32(FLAGS9, 0x108);
    step9(&mut m, 5);
    assert_eq!((m.arm9().halted(), m.arm9_peek32(FLAGS9)), (true, 0x100));
}

#[test]
fn given_intrwait_when_it_waits_then_ie_and_if_are_not_changed_by_the_bios() {
    let mut m = arm9_waiting(&wait_program(0, 8, 4), 8);
    step9(&mut m, 5);
    let irq = m.irq9();
    assert_eq!((irq.ie(), irq.if_()), (0, 0));
}

#[test]
fn given_vblankintrwait_when_executed_with_other_register_values_then_it_waits_for_vblank_with_r0_1()
 {
    let mut m = arm9_waiting(
        &[0xE3A0_0000, 0xE3A0_1008, arm_swi(0x05_0000), MOV_R6, SPIN],
        VBLANK,
    );
    m.bus9().write32(FLAGS9, 0x8);
    step9(&mut m, 5);
    // R0 = 1 and R1 = 1 are forced: the V-blank bit is waited for and the old flags are
    // discarded, so the unrelated bit 3 stays and the wait halts.
    assert_eq!((m.arm9().halted(), m.arm9_peek32(FLAGS9)), (true, 0x8));
}

#[test]
fn given_vblankintrwait_halted_when_a_vblank_irq_arrives_then_it_returns() {
    let mut m = arm9_waiting(
        &[0xE3A0_0000, 0xE3A0_1008, arm_swi(0x05_0000), MOV_R6, SPIN],
        VBLANK,
    );
    step9(&mut m, 5);
    m.bus9().write32(IE, 1);
    m.io.request_irq(Requests {
        arm9: VBLANK,
        arm7: 0,
    });
    step9(&mut m, 40);
    assert_eq!(m.arm9().regs().get(6), 0x99);
}

#[test]
fn given_vblankintrwait_on_the_arm7_halted_when_a_vblank_irq_arrives_then_it_returns() {
    let mut m = arm7_waiting(
        &[0xE3A0_0000, 0xE3A0_1008, arm_swi(0x05_0000), MOV_R6, SPIN],
        VBLANK,
    );
    step7(&mut m, 5);
    m.bus7(0).write32(IE, 1);
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: VBLANK,
    });
    step7(&mut m, 40);
    assert_eq!(
        (m.arm7().regs().get(6), m.arm7_peek32(0, FLAGS7)),
        (0x99, 0)
    );
}

#[test]
fn given_intrwait_on_the_arm7_halted_when_the_irq_arrives_then_the_flag_is_consumed_and_it_returns()
{
    let mut m = arm7_waiting(&wait_program(0, 8, 4), 8);
    step7(&mut m, 5);
    raise_arm7(&mut m);
    step7(&mut m, 40);
    assert_eq!(
        (
            m.arm7().regs().get(6),
            m.arm7().halted(),
            m.arm7_peek32(0, FLAGS7)
        ),
        (0x99, false, 0)
    );
}

#[test]
fn given_intrwait_on_the_arm7_when_executed_then_it_halts_in_svc_mode_with_irqs_open() {
    let mut m = arm7_waiting(&wait_program(0, 8, 4), 8);
    step7(&mut m, 5);
    let regs = m.arm7().regs();
    assert_eq!((m.arm7().halted(), regs.cpsr() & 0x9F), (true, 0x13));
}

// --------------------------------------------------------------- savestate while waiting (§5.5)

fn waiting_machine() -> Machine {
    let mut m = arm9_waiting(&wait_program(0, 8, 4), 8);
    step9(&mut m, 5);
    m
}

/// Runs `m` through the wake-up: the request arrives and the program runs to its end.
fn wake_and_finish(m: &mut Machine) -> Vec<u8> {
    raise_arm9(m);
    step9(m, 40);
    m.save_state()
}

#[test]
fn given_a_machine_saved_while_waiting_when_loaded_and_continued_then_it_equals_an_uninterrupted_run()
 {
    let mut uninterrupted = waiting_machine();
    let saved = uninterrupted.save_state();
    let mut resumed = Machine::new();
    resumed
        .load_rom(uninterrupted.rom().to_vec())
        .expect("same image");
    let loaded = resumed.load_state(&saved);
    let a = wake_and_finish(&mut uninterrupted);
    let b = wake_and_finish(&mut resumed);
    assert_eq!((loaded, a == b), (Ok(()), true));
}

#[test]
fn given_a_machine_saved_while_waiting_when_loaded_then_it_is_still_halted() {
    let waiting = waiting_machine();
    let mut resumed = Machine::new();
    resumed
        .load_rom(waiting.rom().to_vec())
        .expect("same image");
    let loaded = resumed.load_state(&waiting.save_state());
    assert_eq!((loaded, resumed.arm9().halted()), (Ok(()), true));
}

#[test]
fn given_the_same_wait_run_twice_when_saved_then_the_bytes_are_identical() {
    let a = wake_and_finish(&mut waiting_machine());
    let b = wake_and_finish(&mut waiting_machine());
    assert_eq!(a, b);
}

// ------------------------------------------------------------- whole frames with a halted CPU

#[test]
fn given_a_halted_arm9_when_a_frame_runs_then_the_frame_completes_and_the_cpu_stays_halted() {
    let mut m = machine_with(&[WFI_C0_4, MOV_R1, SPIN], &[SPIN]);
    m.run_frame();
    assert_eq!((m.frame_count(), m.arm9().halted()), (1, true));
}

#[test]
fn given_a_halted_arm7_when_a_frame_runs_then_the_frame_completes_and_the_arm7_clock_is_not_behind()
{
    let mut m = machine_with(&[SPIN], &haltcnt_program(0x80));
    m.run_frame();
    assert_eq!(
        (
            m.frame_count(),
            m.arm7().halted(),
            m.arm7_clock() >= m.scheduler().now()
        ),
        (1, true, true)
    );
}

// ------------------------------------------------- review follow-ups: event before halt, DMA halt

const TM0CNT: u32 = 0x0400_0100;

#[test]
fn given_a_timer_started_just_before_the_arm9_halts_when_a_round_runs_then_the_clock_stops_at_the_overflow()
 {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    // Reload FFF0h, enabled with the IRQ bit and prescaler F/1: it overflows 16 cycles later.
    m.bus9().write32(TM0CNT, 0x00C0_FFF0);
    step9(&mut m, 1);
    let due = m.scheduler().next_time().expect("the overflow is queued");
    run_round(&mut m);
    assert_eq!((m.arm9().halted(), m.scheduler().now()), (true, due));
}

#[test]
fn given_a_timer_irq_enabled_before_the_arm9_halts_when_rounds_run_then_the_overflow_wakes_it() {
    let mut m = arm9_running(&[WFI_C0_4, MOV_R1, SPIN]);
    m.arm9.regs_mut().set_cpsr(0x9F);
    m.bus9().write32(IE, TIMER0);
    m.bus9().write32(TM0CNT, 0x00C0_FFF0);
    step9(&mut m, 1);
    clocks_after_rounds(&mut m, 3);
    step9(&mut m, 1);
    assert_eq!((m.arm9().halted(), m.arm9().regs().get(1)), (false, 0x66));
}

/// An ARM7 whose DMA channel 0 copies one halfword, 8000h, to POSTFLG/HALTCNT, so the high byte
/// (HALTCNT) is mode 2. Returns after the first round in which the ARM7 is halted or the request
/// is pending: `(halted, request still pending)`.
fn arm7_halted_by_dma() -> (bool, bool) {
    let mut m = arm7_running(&[SPIN]);
    m.bus7(0).write16(0x0220_0000, 0x8000);
    m.bus7(0).write32(0x0400_00B0, 0x0220_0000);
    m.bus7(0).write32(0x0400_00B4, 0x0400_0300);
    m.bus7(0).write32(0x0400_00B8, (1 << 31) | 1);
    (0..8)
        .map(|_| {
            run_round(&mut m);
            (m.arm7().halted(), m.io().power().halt_requested())
        })
        .find(|&(halted, pending)| halted || pending)
        .unwrap_or((false, false))
}

#[test]
fn given_a_dma_unit_writing_haltcnt_when_the_transfer_ends_then_the_arm7_is_halted_at_once() {
    assert_eq!(arm7_halted_by_dma(), (true, false));
}
