// SPDX-License-Identifier: Apache-2.0
//! The interrupt controller driving each CPU through `Machine`. Expected values come from
//! `docs/hw/interrupts.md` ("The IRQ line", "Edge cases") and `docs/hw/arm-exceptions-modes.md`
//! ("Return address": R14_irq = the address of the instruction that would have run next, + 4;
//! SPSR_irq = the old CPSR; the CPSR enters IRQ mode, ARM state, I set; PC = vector base + 18h),
//! never from running the code. The cost of a step that takes the IRQ is one instruction's
//! (`docs/hw/scheduler.md`, "Cost per instruction": ARM9 1 master cycle, ARM7 2).
//!
//! Design decision pinned here: a step that takes the IRQ does the exception entry INSTEAD of
//! executing the next instruction. The programs are `mov r0, #0x55; mov r1, #0x66; b .`, so an
//! executed first instruction shows as r0 = 0x55.
//!
//! The ARM9 starts at 0x0200_0000 and the ARM7 at 0x0238_0000 (`direct-boot.md`), in System mode
//! with I and F set (CPSR 0xDF). The tests clear I by writing the CPSR to 0x1F. R15 reads the
//! next instruction's address + 8 while the IRQ is taken, so R14_irq = that address + 4.
//! Both vector bases are 0 here (CP15 control is at power-on, `cp15.md`; the ARM7 has no
//! high vectors), and the refill leaves R15 = 0x18 + 8.
//!
//! Where the exception entry sits (step_arm9, step_arm7_instruction, or a shared helper) is the
//! implementation's choice: the tests drive `step_arm9` and `step_arm7_instruction`.

use crate::cart::test_image::{Image, words};
use crate::cpu::Bank;
use crate::cpu::Bus;
use crate::io::irq::Requests;
use crate::machine::*;

const ENTRY9: u32 = 0x0200_0000;
const ENTRY7: u32 = 0x0238_0000;
const IME: u32 = 0x0400_0208;
const IE: u32 = 0x0400_0210;
const IF: u32 = 0x0400_0214;
/// A source both CPUs have: timer 0 overflow (bit 3).
const TIMER0: u32 = 1 << 3;
const TIMER1: u32 = 1 << 4;

const SYSTEM_I_CLEAR: u32 = 0x1F;
const SYSTEM_I_SET: u32 = 0x9F;
/// IRQ mode, I set, ARM state, flags clear.
const IRQ_ENTRY_CPSR: u32 = 0x92;
const IRQ_MODE: u32 = 0x12;
/// The F flag, which an IRQ entry leaves alone.
const F_FLAG: u32 = 0x40;
const VECTOR_PC: u32 = 0x18 + 8;

/// `mov r0, #0x55`, `mov r1, #0x66`, `b .`.
const PROGRAM: [u32; 3] = [0xE3A0_0055, 0xE3A0_1066, 0xEAFF_FFFE];

fn machine() -> Machine {
    let mut image = Image::valid();
    image.arm9_code = words(&PROGRAM);
    image.arm7_code = words(&PROGRAM);
    let mut machine = Machine::new();
    machine.load_rom(image.build()).expect("synthetic image");
    machine
}

/// Writes IME and IE of the ARM9 through the bus.
fn enable9(m: &mut Machine, ime: u32, ie: u32) {
    m.bus9().write32(IME, ime);
    m.bus9().write32(IE, ie);
}

/// Writes IME and IE of the ARM7 through the bus.
fn enable7(m: &mut Machine, ime: u32, ie: u32) {
    m.bus7(0).write32(IME, ime);
    m.bus7(0).write32(IE, ie);
}

/// What a test needs to see of a CPU: (CPSR, R14_irq, SPSR_irq, R15, r0, r1).
type Seen = (u32, u32, Option<u32>, u32, u32, u32);

fn seen9(m: &Machine) -> Seen {
    let r = m.arm9().regs();
    (
        r.cpsr(),
        r.banked(Bank::Irq, 14),
        r.spsr(Bank::Irq),
        r.get(15),
        r.get(0),
        r.get(1),
    )
}

fn seen7(m: &Machine) -> Seen {
    let r = m.arm7().regs();
    (
        r.cpsr(),
        r.banked(Bank::Irq, 14),
        r.spsr(Bank::Irq),
        r.get(15),
        r.get(0),
        r.get(1),
    )
}

fn in_irq_mode(cpsr: u32) -> bool {
    cpsr & 0x1F == IRQ_MODE
}

/// One ARM9 setup: IME, IE, the request and whether I is set in the CPSR.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Setup {
    ime: bool,
    ie: u32,
    request: u32,
    i_set: bool,
}

/// A booted machine whose ARM9 has `setup` applied.
fn prepared9(s: Setup) -> Machine {
    let mut m = machine();
    enable9(&mut m, u32::from(s.ime), s.ie);
    m.io.request_irq(Requests {
        arm9: s.request,
        arm7: 0,
    });
    m.arm9.regs_mut().set_cpsr(if s.i_set {
        SYSTEM_I_SET
    } else {
        SYSTEM_I_CLEAR
    });
    m
}

/// A booted machine whose ARM7 has `setup` applied.
fn prepared7(s: Setup) -> Machine {
    let mut m = machine();
    enable7(&mut m, u32::from(s.ime), s.ie);
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: s.request,
    });
    m.arm7.regs_mut().set_cpsr(if s.i_set {
        SYSTEM_I_SET
    } else {
        SYSTEM_I_CLEAR
    });
    m
}

/// The sixteen on/off combinations of IME, IE, IF and (not I), plus one with the wrong IE bit.
/// The IRQ is taken only when all four hold (`interrupts.md`, "The IRQ line").
fn gating_cases() -> Vec<(Setup, bool)> {
    let mut cases: Vec<(Setup, bool)> = (0..16u32)
        .map(|n| {
            let (ime, ie, req, i_clear) = (n & 1 != 0, n & 2 != 0, n & 4 != 0, n & 8 != 0);
            (
                Setup {
                    ime,
                    ie: if ie { TIMER0 } else { 0 },
                    request: if req { TIMER0 } else { 0 },
                    i_set: !i_clear,
                },
                ime && ie && req && i_clear,
            )
        })
        .collect();
    cases.push((
        Setup {
            ime: true,
            ie: TIMER1,
            request: TIMER0,
            i_set: false,
        },
        false,
    ));
    cases
}

fn gating_failures9() -> Vec<Setup> {
    gating_cases()
        .into_iter()
        .filter(|&(s, taken)| {
            let mut m = prepared9(s);
            m.step_arm9();
            in_irq_mode(m.arm9().regs().cpsr()) != taken
        })
        .map(|(s, _)| s)
        .collect()
}

fn gating_failures7() -> Vec<Setup> {
    gating_cases()
        .into_iter()
        .filter(|&(s, taken)| {
            let mut m = prepared7(s);
            m.step_arm7_instruction();
            in_irq_mode(m.arm7().regs().cpsr()) != taken
        })
        .map(|(s, _)| s)
        .collect()
}

const ALL_ON: Setup = Setup {
    ime: true,
    ie: TIMER0,
    request: TIMER0,
    i_set: false,
};

// ---------------------------------------------------------------- the ARM9 takes the IRQ

#[test]
fn given_the_four_conditions_in_every_combination_when_the_arm9_steps_then_the_irq_is_taken_only_when_all_hold()
 {
    assert_eq!(gating_failures9(), Vec::<Setup>::new());
}

#[test]
fn given_the_four_conditions_in_every_combination_when_the_arm7_steps_then_the_irq_is_taken_only_when_all_hold()
 {
    assert_eq!(gating_failures7(), Vec::<Setup>::new());
}

#[test]
fn given_an_enabled_pending_irq_when_the_arm9_steps_then_it_enters_irq_mode_with_r14_spsr_cpsr_and_pc_per_the_note()
 {
    let mut m = prepared9(ALL_ON);
    m.step_arm9();
    // R14 = first instruction + 4; SPSR = the old CPSR; I set; at the vector, refilled.
    assert_eq!(
        seen9(&m),
        (
            IRQ_ENTRY_CPSR,
            ENTRY9 + 4,
            Some(SYSTEM_I_CLEAR),
            VECTOR_PC,
            0,
            0
        )
    );
}

#[test]
fn given_an_enabled_pending_irq_when_the_arm9_steps_then_the_entry_replaces_the_instruction_and_costs_one_cycle()
 {
    let mut m = prepared9(ALL_ON);
    m.step_arm9();
    // r0 is still 0: `mov r0, #0x55` did not run. ARM9 cost 1 master cycle.
    assert_eq!((m.arm9().regs().get(0), m.scheduler().now()), (0, 1));
}

#[test]
fn given_an_enabled_pending_irq_when_the_arm7_steps_then_it_enters_irq_mode_with_r14_spsr_cpsr_and_pc_per_the_note()
 {
    let mut m = prepared7(ALL_ON);
    m.step_arm7_instruction();
    assert_eq!(
        seen7(&m),
        (
            IRQ_ENTRY_CPSR,
            ENTRY7 + 4,
            Some(SYSTEM_I_CLEAR),
            VECTOR_PC,
            0,
            0
        )
    );
}

#[test]
fn given_an_enabled_pending_irq_when_the_arm7_steps_then_the_entry_replaces_the_instruction_and_costs_one_instruction()
 {
    let mut m = prepared7(ALL_ON);
    m.step_arm7_instruction();
    // ARM7 cost 2 master cycles.
    assert_eq!((m.arm7().regs().get(0), m.arm7_clock()), (0, 2));
}

#[test]
fn given_an_irq_taken_when_if_is_read_then_the_entry_has_not_cleared_it_and_the_line_is_still_high()
{
    let mut m = prepared9(ALL_ON);
    m.step_arm9();
    assert_eq!(
        (m.irq9().if_(), m.irq9().line(), m.arm9_peek32(IF)),
        (TIMER0, true, TIMER0)
    );
}

#[test]
fn given_an_irq_taken_on_the_arm7_when_if_is_read_then_the_entry_has_not_cleared_it() {
    let mut m = prepared7(ALL_ON);
    m.step_arm7_instruction();
    assert_eq!((m.irq7().if_(), m.irq7().line()), (TIMER0, true));
}

// ---------------------------------------------------------------- IME = 0

#[test]
fn given_ime_off_and_a_request_when_the_arm9_steps_then_the_instruction_runs_and_if_still_holds_the_request()
 {
    let mut m = prepared9(Setup {
        ime: false,
        ..ALL_ON
    });
    m.step_arm9();
    assert_eq!(
        (
            m.arm9().regs().get(0),
            m.arm9().regs().cpsr(),
            m.irq9().if_(),
            m.irq9().line()
        ),
        (0x55, SYSTEM_I_CLEAR, TIMER0, false)
    );
}

#[test]
fn given_a_request_taken_while_ime_was_off_when_ime_is_set_later_then_the_pending_irq_is_taken_after_the_running_instruction()
 {
    let mut m = prepared9(Setup {
        ime: false,
        ..ALL_ON
    });
    m.step_arm9();
    m.bus9().write32(IME, 1);
    m.step_arm9();
    // The first instruction ran; the IRQ replaced the second: R14 = second + 4 = ENTRY + 8.
    assert_eq!(
        seen9(&m),
        (
            IRQ_ENTRY_CPSR,
            ENTRY9 + 8,
            Some(SYSTEM_I_CLEAR),
            VECTOR_PC,
            0x55,
            0
        )
    );
}

// ---------------------------------------------------------------- I set

#[test]
fn given_i_set_and_a_pending_irq_when_the_arm9_steps_then_the_instruction_runs_and_the_line_stays_asserted()
 {
    let mut m = prepared9(Setup {
        i_set: true,
        ..ALL_ON
    });
    m.step_arm9();
    assert_eq!(
        (
            m.arm9().regs().get(0),
            m.arm9().regs().cpsr(),
            m.irq9().line(),
            m.irq9().if_()
        ),
        (0x55, SYSTEM_I_SET, true, TIMER0)
    );
}

#[test]
fn given_a_request_waiting_behind_i_when_i_is_cleared_then_the_next_step_takes_it() {
    let mut m = prepared9(Setup {
        i_set: true,
        ..ALL_ON
    });
    m.step_arm9();
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m.step_arm9();
    // The first instruction ran under I; the IRQ replaced the second: R14 = ENTRY + 8.
    assert_eq!(
        seen9(&m),
        (
            IRQ_ENTRY_CPSR,
            ENTRY9 + 8,
            Some(SYSTEM_I_CLEAR),
            VECTOR_PC,
            0x55,
            0
        )
    );
}

#[test]
fn given_a_request_waiting_behind_i_on_the_arm7_when_i_is_cleared_then_the_next_step_takes_it() {
    let mut m = prepared7(Setup {
        i_set: true,
        ..ALL_ON
    });
    m.step_arm7_instruction();
    m.arm7.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m.step_arm7_instruction();
    assert_eq!(
        seen7(&m),
        (
            IRQ_ENTRY_CPSR,
            ENTRY7 + 8,
            Some(SYSTEM_I_CLEAR),
            VECTOR_PC,
            0x55,
            0
        )
    );
}

// ---------------------------------------------------------------- IE and acknowledged requests

#[test]
fn given_a_request_for_a_source_ie_does_not_enable_when_the_arm9_steps_then_it_is_not_taken_and_if_holds_it()
 {
    let mut m = prepared9(Setup {
        ie: TIMER1,
        ..ALL_ON
    });
    m.step_arm9();
    assert_eq!(
        (
            m.arm9().regs().get(0),
            m.arm9().regs().cpsr(),
            m.irq9().if_()
        ),
        (0x55, SYSTEM_I_CLEAR, TIMER0)
    );
}

#[test]
fn given_a_request_acknowledged_before_the_step_when_the_arm9_steps_then_the_request_is_dropped() {
    let mut m = prepared9(ALL_ON);
    m.bus9().write32(IF, TIMER0);
    m.step_arm9();
    assert_eq!(
        (
            m.arm9().regs().get(0),
            m.arm9().regs().cpsr(),
            m.irq9().if_()
        ),
        (0x55, SYSTEM_I_CLEAR, 0)
    );
}

// ---------------------------------------------------------------- independence

#[test]
fn given_a_request_on_the_arm9_when_both_cpus_run_then_the_arm7_is_never_interrupted() {
    let mut m = prepared9(ALL_ON);
    // The ARM7 is ready to take bit 3 too, but nothing was requested on it.
    enable7(&mut m, 1, TIMER0);
    m.arm7.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m.step_arm9();
    m.step_arm9();
    assert_eq!(
        (
            in_irq_mode(m.arm9().regs().cpsr()),
            m.arm7().regs().cpsr(),
            m.irq7().if_()
        ),
        (true, SYSTEM_I_CLEAR, 0)
    );
}

#[test]
fn given_a_request_on_the_arm7_when_both_cpus_run_then_the_arm9_is_never_interrupted() {
    let mut m = prepared7(ALL_ON);
    enable9(&mut m, 1, TIMER0);
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    // Only the bare steps: `step_arm9` would also run the ARM7 up to the ARM9's time.
    m.step_arm7_instruction();
    m.step_arm9_instruction();
    m.step_arm9_instruction();
    assert_eq!(
        (
            in_irq_mode(m.arm7().regs().cpsr()),
            m.arm9().regs().cpsr(),
            m.arm9().regs().get(0),
            m.irq9().if_()
        ),
        (true, SYSTEM_I_CLEAR, 0x55, 0)
    );
}

#[test]
fn given_the_same_request_on_both_cpus_with_only_the_arm9_enabled_when_both_run_then_only_the_arm9_enters_irq_mode()
 {
    let mut m = prepared9(ALL_ON);
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: TIMER0,
    });
    m.arm7.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    // The ARM7's IME is off: its request stays latched.
    m.step_arm9();
    m.step_arm7_instruction();
    assert_eq!(
        (
            in_irq_mode(m.arm9().regs().cpsr()),
            in_irq_mode(m.arm7().regs().cpsr()),
            m.irq7().if_()
        ),
        (true, false, TIMER0)
    );
}

// ---------------------------------------------------------------- the scheduler's own loop

#[test]
fn given_a_pending_enabled_request_on_each_cpu_when_a_frame_runs_then_both_enter_irq_mode_at_the_start()
 {
    let mut m = machine();
    enable9(&mut m, 1, TIMER0);
    enable7(&mut m, 1, TIMER0);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: TIMER0,
    });
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m.arm7.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m.run_frame();
    // The entry replaced the first instruction (r0 stays 0), LR_irq = start + 4, and the
    // unacknowledged request is still latched.
    let (s9, s7) = (seen9(&m), seen7(&m));
    assert_eq!(
        (
            (s9.0, s9.1, s9.4),
            (s7.0, s7.1, s7.4),
            (m.irq9().if_(), m.irq7().if_())
        ),
        (
            (IRQ_ENTRY_CPSR, ENTRY9 + 4, 0),
            (IRQ_ENTRY_CPSR, ENTRY7 + 4, 0),
            (TIMER0, TIMER0)
        )
    );
}

// ---------------------------------------------------------------- Thumb state

/// `add r0, pc, #1`, `bx r0` (to the Thumb code at +8), then Thumb `movs r0, #0x55`,
/// `movs r1, #0x66`, `b .`.
const THUMB_PROGRAM: [u32; 4] = [0xE28F_0001, 0xE12F_FF10, 0x2166_2055, 0x0000_E7FE];

#[test]
fn given_the_arm9_in_thumb_state_when_an_irq_is_taken_then_lr_is_the_next_instruction_plus_4_and_the_entry_is_arm_state()
 {
    let mut image = Image::valid();
    image.arm9_code = words(&THUMB_PROGRAM);
    image.arm7_code = words(&PROGRAM);
    let mut m = Machine::new();
    m.load_rom(image.build()).expect("synthetic image");
    enable9(&mut m, 1, TIMER0);
    m.step_arm9_instruction();
    m.step_arm9_instruction();
    let thumb_cpsr = m.arm9().regs().cpsr() & !0x80;
    m.arm9.regs_mut().set_cpsr(thumb_cpsr);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
    m.step_arm9_instruction();
    // The Thumb code starts at ENTRY9 + 8; its first instruction did not run (r0 is still the
    // address `bx` jumped through), the CPSR is ARM state in IRQ mode with I set, SPSR_irq holds
    // the Thumb CPSR, F is left as it was (set at boot, an IRQ does not touch it), and
    // LR_irq = the next Thumb instruction's address + 4.
    let (cpsr, lr, spsr, _, r0, _) = seen9(&m);
    assert_eq!(
        (cpsr, lr, spsr, r0),
        (
            IRQ_ENTRY_CPSR | F_FLAG,
            ENTRY9 + 8 + 4,
            Some(thumb_cpsr),
            ENTRY9 + 9
        )
    );
}

// ------------------------------------------------------- every source reaches IF the same way
//
// The LCD (`display-timing.md`), IPC (`ipc.md`) and the timers (`timers.md`) each raise IF bits on
// the CPUs their own notes name; whatever path carries them, IF shows them and nothing else.

const DISPSTAT: u32 = 0x0400_0004;
const IPCSYNC: u32 = 0x0400_0180;
const TM0CNT_L: u32 = 0x0400_0100;
const TM0CNT_H: u32 = 0x0400_0102;
const HBLANK_IRQ_ENABLE: u16 = 1 << 4;
/// IPCSYNC bit 14, the IRQ enable; bit 13, the send strobe (`ipc.md`, "IPCSYNC").
const SYNC_IRQ_ENABLE: u16 = 1 << 14;
const SYNC_SEND_IRQ: u16 = 1 << 13;
const START_WITH_IRQ: u16 = 0xC0;
const HBLANK_BIT: u32 = 1 << 1;
const IPCSYNC_BIT: u32 = 1 << 16;

/// IF of both CPUs after the ARM9 enabled the HBlank IRQ and started timer 0, the ARM7 enabled
/// the sync IRQ, the ARM9 strobed it, and the ARM9 then ran past the first HBlank and the timer's
/// overflow (16 ticks, 32 cycles).
fn if_after_lcd_ipc_and_timer() -> (u32, u32) {
    let mut m = machine();
    m.bus9().write16(DISPSTAT, HBLANK_IRQ_ENABLE);
    m.bus9().write16(TM0CNT_L, 0xFFF0);
    m.bus9().write16(TM0CNT_H, START_WITH_IRQ);
    m.bus7(0).write16(IPCSYNC, SYNC_IRQ_ENABLE);
    m.bus9().write16(IPCSYNC, SYNC_SEND_IRQ);
    (0..4000).for_each(|_| m.step_arm9());
    (m.irq9().if_(), m.irq7().if_())
}

#[test]
fn given_lcd_ipc_and_timer_sources_when_each_raises_its_irq_then_if_shows_each_on_the_cpu_it_belongs_to()
 {
    assert_eq!(
        if_after_lcd_ipc_and_timer(),
        (HBLANK_BIT | TIMER0, IPCSYNC_BIT)
    );
}
