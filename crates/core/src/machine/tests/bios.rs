// SPDX-License-Identifier: Apache-2.0
//! The HLE BIOS through `Machine`: the SWI vector, the number found in memory, the return that
//! restores the CPSR, the IRQ dispatcher and the cases where the BIOS is not involved. Expected
//! values come from `docs/hw/hle-swi.md` ("SWI", "The IRQ dispatcher", "Edge cases") and
//! `docs/hw/arm-exceptions-modes.md` ("Return address"), never from running the code.
//!
//! The ARM9 starts at 0x0200_0000 and the ARM7 at 0x0238_0000 (`direct-boot.md`), in System mode
//! with I and F set (CPSR 0xDF). The tests set the CPSR themselves. After an exception entry the
//! refill leaves R15 = vector + 8, which is where the BIOS trap sits (`hle-swi.md`, "How the
//! machine takes the BIOS's place").

use crate::cart::test_image::{Image, words};
use crate::cpu::{Bank, Bus};
use crate::io::irq::Requests;
use crate::machine::*;

pub(super) const ENTRY9: u32 = 0x0200_0000;
pub(super) const ENTRY7: u32 = 0x0238_0000;
pub(super) const HANDLER: u32 = 0x0200_0100;
pub(super) const HANDLER7: u32 = 0x0238_0100;

pub(super) const IME: u32 = 0x0400_0208;
pub(super) const IE: u32 = 0x0400_0210;
/// A source both CPUs have: timer 0 overflow (bit 3).
pub(super) const TIMER0: u32 = 1 << 3;

/// CP15 control register bit 13: the vectors at `FFFF0000h`.
pub(super) const HIGH_VECTORS: u32 = 1 << 13;
/// CP15 control bits 16 (DTCM enable) and 18 (ITCM enable).
pub(super) const DTCM_ENABLE: u32 = 1 << 16;
const ITCM_ENABLE: u32 = 1 << 18;
/// The DTCM at `027C0000h`, 16 KiB (size field 5), and the 32 KiB ITCM (size field 6).
pub(super) const DTCM_REGISTER: u32 = 0x027C_000A;
const ITCM_REGISTER: u32 = 0x0000_000C;
pub(super) const DTCM_BASE: u32 = 0x027C_0000;

/// System mode, ARM state, I clear, with the N and C flags set so a lost CPSR shows.
pub(super) const SYSTEM_FLAGS: u32 = 0xA000_001F;
pub(super) const SYSTEM_I_CLEAR: u32 = 0x1F;
const SVC_MODE: u32 = 0x13;
pub(super) const IRQ_MODE: u32 = 0x12;
const THUMB: u32 = 1 << 5;
pub(super) const SP_IRQ9: u32 = 0x0200_1000;

/// `mov r0, #0x55`, `mov r1, #0x66`, `b .`.
pub(super) const PROGRAM: [u32; 3] = [0xE3A0_0055, 0xE3A0_1066, 0xEAFF_FFFE];
/// `bx lr`.
pub(super) const BX_LR: u32 = 0xE12F_FF1E;
/// `mov r6, #0x99`.
pub(super) const MOV_R6: u32 = 0xE3A0_6099;
/// `mov r5, #0x77`.
const MOV_R5: u32 = 0xE3A0_5077;

pub(super) fn machine_with(arm9: &[u32], arm7: &[u32]) -> Machine {
    let mut image = Image::valid();
    image.arm9_code = words(arm9);
    image.arm7_code = words(arm7);
    let mut m = Machine::new();
    m.load_rom(image.build()).expect("synthetic image");
    m
}

/// An ARM9 that runs `program` with the BIOS vectors on and the CPSR as `SYSTEM_FLAGS` says.
pub(super) fn arm9_running(program: &[u32]) -> Machine {
    let mut m = machine_with(program, &PROGRAM);
    m.arm9.write_cp15(1, 0, 0, HIGH_VECTORS);
    m.arm9.regs_mut().set_cpsr(SYSTEM_FLAGS);
    m
}

pub(super) fn arm7_running(program: &[u32]) -> Machine {
    let mut m = machine_with(&PROGRAM, program);
    m.arm7.regs_mut().set_cpsr(SYSTEM_FLAGS);
    m
}

/// r0-r3 = 0xA0-0xA3.
fn seed_low_registers(m: &mut Machine) {
    (0..4).for_each(|i| m.arm9.regs_mut().set(i, 0xA0 + i as u32));
}

pub(super) fn step9(m: &mut Machine, count: usize) {
    (0..count).for_each(|_| m.step_arm9());
}

pub(super) fn step7(m: &mut Machine, count: usize) {
    (0..count).for_each(|_| m.step_arm7_instruction());
}

/// `add r0, pc, #1`, `bx r0`, then Thumb: `swi number`, `movs r1, #0x66`, `b .`.
fn thumb_swi_program(number: u16) -> [u32; 3] {
    [
        0xE28F_0001,
        0xE12F_FF10,
        0x2166_0000 | (0xDF00 | u32::from(number)),
    ]
}

pub(super) fn arm_swi(comment: u32) -> u32 {
    0xEF00_0000 | comment
}

// --------------------------------------------------------------------------- SWI numbers

#[test]
fn given_an_arm_swi_with_comment_050000h_when_the_bios_traps_then_the_number_is_5() {
    let mut m = arm9_running(&[arm_swi(0x05_0000), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 2);
    assert_eq!(m.inspect().swi_record9().last(), Some(5));
}

#[test]
fn given_an_arm_swi_with_a_comment_in_every_byte_when_trapped_then_only_bits_23_to_16_count() {
    let mut m = arm9_running(&[arm_swi(0x12_3456), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 2);
    assert_eq!(m.inspect().swi_record9().last(), Some(0x12));
}

#[test]
fn given_a_thumb_swi_7_when_the_bios_traps_then_the_number_is_7() {
    let mut m = arm9_running(&thumb_swi_program(7));
    step9(&mut m, 4);
    assert_eq!(m.inspect().swi_record9().last(), Some(7));
}

#[test]
fn given_an_arm7_swi_when_the_bios_traps_then_the_arm7_record_holds_the_number() {
    let mut m = arm7_running(&[arm_swi(0x06_0000), PROGRAM[1], PROGRAM[2]]);
    step7(&mut m, 2);
    assert_eq!(
        (
            m.inspect().swi_record7().last(),
            m.inspect().swi_record9().last()
        ),
        (Some(6), None)
    );
}

// --------------------------------------------------------------------------- the SWI vector

#[test]
fn given_an_arm_swi_when_executed_then_the_cpu_is_in_supervisor_mode_at_the_trap_address() {
    let mut m = arm9_running(&[arm_swi(0x1E_0000), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 1);
    let regs = m.inspect().arm9().regs();
    assert_eq!(
        (
            regs.cpsr() & 0x1F,
            regs.get(15),
            regs.banked(Bank::Supervisor, 14)
        ),
        (SVC_MODE, 0xFFFF_0008 + 8, ENTRY9 + 4)
    );
}

#[test]
fn given_an_arm_swi_when_the_bios_returns_then_the_cpsr_is_the_one_before_the_swi() {
    let mut m = arm9_running(&[arm_swi(0x1E_0000), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 2);
    assert_eq!(m.inspect().arm9().regs().cpsr(), SYSTEM_FLAGS);
}

#[test]
fn given_an_arm_swi_when_the_bios_returns_then_the_next_instruction_runs() {
    let mut m = arm9_running(&[arm_swi(0x1E_0000), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 3);
    assert_eq!(m.inspect().arm9().regs().get(1), 0x66);
}

#[test]
fn given_a_thumb_swi_when_the_bios_returns_then_t_is_set_again_and_the_next_thumb_instruction_runs()
{
    let mut m = arm9_running(&thumb_swi_program(0x1E));
    step9(&mut m, 5);
    let regs = m.inspect().arm9().regs();
    assert_eq!((regs.cpsr() & THUMB, regs.get(1)), (THUMB, 0x66));
}

#[test]
fn given_a_thumb_swi_when_executed_then_the_vector_is_entered_in_arm_state() {
    let mut m = arm9_running(&thumb_swi_program(0x1E));
    step9(&mut m, 3);
    let regs = m.inspect().arm9().regs();
    assert_eq!(
        (regs.cpsr() & THUMB, regs.cpsr() & 0x1F, regs.get(15)),
        (0, SVC_MODE, 0xFFFF_0008 + 8)
    );
}

#[test]
fn given_an_unimplemented_swi_when_trapped_then_it_is_recorded_and_the_registers_are_unchanged() {
    let mut m = arm9_running(&[arm_swi(0x1E_0000), PROGRAM[1], PROGRAM[2]]);
    seed_low_registers(&mut m);
    step9(&mut m, 2);
    let regs = m.inspect().arm9().regs();
    assert_eq!(
        (
            [regs.get(0), regs.get(1), regs.get(2), regs.get(3)],
            m.inspect().swi_record9().last_unimplemented(),
        ),
        ([0xA0, 0xA1, 0xA2, 0xA3], Some(0x1E))
    );
}

#[test]
fn given_a_swi_number_above_1fh_when_trapped_then_it_returns_without_effect() {
    let mut m = arm9_running(&[arm_swi(0xFF_0000), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 3);
    assert_eq!(
        (
            m.inspect().arm9().regs().get(1),
            m.inspect().swi_record9().last_unimplemented()
        ),
        (0x66, Some(0xFF))
    );
}

#[test]
fn given_a_machine_that_never_called_swi_when_reading_the_record_then_it_is_empty() {
    let m = arm9_running(&PROGRAM);
    assert_eq!(
        (
            m.inspect().swi_record9().last(),
            m.inspect().swi_record9().last_unimplemented()
        ),
        (None, None)
    );
}

// ----------------------------------------------------- the game's own vectors (no BIOS)

/// An ARM9 with the vectors low and the ITCM holding `handler` at the vector `at`.
fn arm9_with_itcm_handler(program: &[u32], at: u32, handler: u32) -> Machine {
    let mut m = machine_with(program, &PROGRAM);
    m.arm9.write_cp15(9, 1, 1, ITCM_REGISTER);
    m.arm9.write_cp15(1, 0, 0, ITCM_ENABLE);
    m.bus9().write32(at, handler);
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m
}

#[test]
fn given_arm9_vectors_at_zero_when_swi_executes_then_the_game_handler_runs_and_nothing_is_recorded()
{
    let mut m = arm9_with_itcm_handler(&[arm_swi(0x05_0000), PROGRAM[1], PROGRAM[2]], 0x08, MOV_R5);
    step9(&mut m, 2);
    assert_eq!(
        (
            m.inspect().arm9().regs().get(5),
            m.inspect().swi_record9().last()
        ),
        (0x77, None)
    );
}

#[test]
fn given_arm9_vectors_at_zero_when_irq_is_taken_then_the_game_handler_runs_and_the_stack_is_untouched()
 {
    let mut m = arm9_with_itcm_handler(&PROGRAM, 0x18, MOV_R5);
    m.arm9.regs_mut().set_banked(Bank::Irq, 13, SP_IRQ9);
    request_timer0_on_arm9(&mut m);
    step9(&mut m, 2);
    assert_eq!(
        (
            m.inspect().arm9().regs().get(5),
            m.inspect().arm9().regs().get(13)
        ),
        (0x77, SP_IRQ9)
    );
}

// ------------------------------------------------------------------------ the IRQ dispatcher

pub(super) fn request_timer0_on_arm9(m: &mut Machine) {
    m.bus9().write32(IME, 1);
    m.bus9().write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: TIMER0,
        arm7: 0,
    });
}

pub(super) fn request_timer0_on_arm7(m: &mut Machine) {
    m.bus7(0).write32(IME, 1);
    m.bus7(0).write32(IE, TIMER0);
    m.io.request_irq(Requests {
        arm9: 0,
        arm7: TIMER0,
    });
}

/// An ARM9 interrupted at `ENTRY9` with the DTCM at `DTCM_BASE`, the user handler pointer set to
/// `pointer` and the handler `handler_words` at `HANDLER`; r0-r3 and r12 hold 1-4 and 12.
fn arm9_taking_irq(pointer: u32, handler_words: &[u32]) -> Machine {
    let mut m = arm9_running(&PROGRAM);
    m.arm9.write_cp15(9, 1, 0, DTCM_REGISTER);
    m.arm9.write_cp15(1, 0, 0, HIGH_VECTORS | DTCM_ENABLE);
    m.bus9().write32(DTCM_BASE + 0x3FFC, pointer);
    handler_words
        .iter()
        .enumerate()
        .for_each(|(i, &word)| m.bus9().write32(HANDLER + 4 * i as u32, word));
    m.arm9.regs_mut().set_cpsr(SYSTEM_FLAGS & !0x80);
    [1, 2, 3, 4]
        .iter()
        .enumerate()
        .for_each(|(i, &v)| m.arm9.regs_mut().set(i, v));
    m.arm9.regs_mut().set(12, 12);
    m.arm9.regs_mut().set_banked(Bank::Irq, 13, SP_IRQ9);
    request_timer0_on_arm9(&mut m);
    m
}

#[test]
fn given_an_arm9_irq_when_the_vector_is_trapped_then_r0_to_r3_r12_and_lr_are_pushed_on_the_irq_stack()
 {
    let mut m = arm9_taking_irq(HANDLER, &[MOV_R6, BX_LR]);
    step9(&mut m, 2);
    let base = SP_IRQ9 - 24;
    let pushed: Vec<u32> = (0..6)
        .map(|i| m.inspect().arm9_peek32(base + 4 * i))
        .collect();
    assert_eq!(pushed, [1, 2, 3, 4, 12, ENTRY9 + 4]);
}

#[test]
fn given_an_arm9_irq_when_the_vector_is_trapped_then_sp_irq_drops_by_24() {
    let mut m = arm9_taking_irq(HANDLER, &[MOV_R6, BX_LR]);
    step9(&mut m, 2);
    assert_eq!(m.inspect().arm9().regs().get(13), SP_IRQ9 - 24);
}

#[test]
fn given_an_arm9_irq_when_the_vector_is_trapped_then_the_handler_runs_in_irq_mode_with_the_return_in_lr()
 {
    let mut m = arm9_taking_irq(HANDLER, &[MOV_R6, BX_LR]);
    step9(&mut m, 2);
    let regs = m.inspect().arm9().regs();
    assert_eq!(
        (regs.cpsr() & 0x9F, regs.get(0), regs.get(14), regs.get(15)),
        (IRQ_MODE | 0x80, 0x0400_0000, 0xFFFF_0020, HANDLER + 8)
    );
}

#[test]
fn given_an_arm9_irq_when_the_handler_returns_then_the_interrupted_instruction_is_next() {
    let mut m = arm9_taking_irq(HANDLER, &[MOV_R6, BX_LR]);
    step9(&mut m, 5);
    assert_eq!(
        (
            m.inspect().arm9().regs().cpsr(),
            m.inspect().arm9().regs().get(15)
        ),
        (SYSTEM_FLAGS & !0x80, ENTRY9 + 8)
    );
}

#[test]
fn given_an_arm9_irq_when_the_handler_returns_then_the_saved_registers_and_the_stack_pointer_are_back()
 {
    let mut m = arm9_taking_irq(HANDLER, &[MOV_R6, BX_LR]);
    step9(&mut m, 5);
    let regs = m.inspect().arm9().regs();
    assert_eq!(
        (
            [
                regs.get(0),
                regs.get(1),
                regs.get(2),
                regs.get(3),
                regs.get(12)
            ],
            regs.banked(Bank::Irq, 13),
            regs.get(6),
        ),
        ([1, 2, 3, 4, 12], SP_IRQ9, 0x99)
    );
}

#[test]
fn given_an_arm9_handler_pointer_with_bit_0_set_when_dispatched_then_the_cpu_is_in_thumb_state() {
    // Thumb: `movs r6, #0x99`, `bx lr`.
    let mut m = arm9_taking_irq(HANDLER | 1, &[0x4770_2699]);
    step9(&mut m, 2);
    let regs = m.inspect().arm9().regs();
    assert_eq!((regs.cpsr() & THUMB, regs.get(15)), (THUMB, HANDLER + 4));
}

#[test]
fn given_a_thumb_arm9_handler_when_it_returns_then_the_interrupted_instruction_is_next() {
    let mut m = arm9_taking_irq(HANDLER | 1, &[0x4770_2699]);
    step9(&mut m, 5);
    assert_eq!(
        (
            m.inspect().arm9().regs().get(6),
            m.inspect().arm9().regs().get(15)
        ),
        (0x99, ENTRY9 + 8)
    );
}

#[test]
fn given_a_dispatched_irq_when_the_trap_runs_then_if_ie_and_ime_are_untouched() {
    let mut m = arm9_taking_irq(HANDLER, &[MOV_R6, BX_LR]);
    step9(&mut m, 5);
    let irq = m.inspect().irq9();
    assert_eq!((irq.ime(), irq.ie(), irq.if_()), (true, TIMER0, TIMER0));
}

/// An ARM7 interrupted at `ENTRY7`, the pointer at `0380FFFCh` set to `pointer`.
fn arm7_taking_irq(pointer: u32) -> Machine {
    let mut m = arm7_running(&PROGRAM);
    m.arm7.regs_mut().set_cpsr(SYSTEM_FLAGS & !0x80);
    [1, 2, 3, 4]
        .iter()
        .enumerate()
        .for_each(|(i, &v)| m.arm7.regs_mut().set(i, v));
    m.arm7.regs_mut().set(12, 12);
    m.bus7(0).write32(0x0380_FFFC, pointer);
    [MOV_R6, BX_LR]
        .iter()
        .enumerate()
        .for_each(|(i, &word)| m.bus7(0).write32(HANDLER7 + 4 * i as u32, word));
    request_timer0_on_arm7(&mut m);
    m
}

#[test]
fn given_an_arm7_irq_when_the_vector_is_trapped_then_the_registers_are_pushed_on_the_irq_stack() {
    let mut m = arm7_taking_irq(HANDLER7);
    let sp = m.inspect().arm7().regs().banked(Bank::Irq, 13);
    step7(&mut m, 2);
    let pushed: Vec<u32> = (0..6)
        .map(|i| m.inspect().arm7_peek32(0, sp - 24 + 4 * i))
        .collect();
    assert_eq!(pushed, [1, 2, 3, 4, 12, ENTRY7 + 4]);
}

#[test]
fn given_an_arm7_irq_when_the_handler_returns_then_the_interrupted_instruction_is_next() {
    let mut m = arm7_taking_irq(HANDLER7);
    step7(&mut m, 5);
    assert_eq!(
        (
            m.inspect().arm7().regs().cpsr(),
            m.inspect().arm7().regs().get(15),
            m.inspect().arm7().regs().get(6)
        ),
        (SYSTEM_FLAGS & !0x80, ENTRY7 + 8, 0x99)
    );
}

#[test]
fn given_an_arm7_handler_pointer_with_bit_0_set_when_dispatched_then_the_arm7_stays_in_arm_state() {
    let mut m = arm7_taking_irq(HANDLER7 | 1);
    step7(&mut m, 2);
    let regs = m.inspect().arm7().regs();
    assert_eq!((regs.cpsr() & THUMB, regs.get(15)), (0, HANDLER7 + 8));
}

// ---------------------------------------------------------------------- savestate and cost

#[test]
fn given_a_recorded_swi_when_saved_and_loaded_then_the_record_survives() {
    let mut m = arm9_running(&[arm_swi(0x1E_0000), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 2);
    let mut loaded = Machine::new();
    loaded.load_rom(m.rom().to_vec()).expect("same image");
    let result = loaded.load_state(&m.save_state());
    assert_eq!(
        (result, loaded.inspect().swi_record9().last_unimplemented()),
        (Ok(()), Some(0x1E))
    );
}

#[test]
fn given_an_arm9_swi_when_the_bios_runs_then_the_master_clock_advances_by_one_instruction_cost() {
    let mut m = arm9_running(&[arm_swi(0x05_0000), PROGRAM[1], PROGRAM[2]]);
    step9(&mut m, 1);
    let before = m.inspect().scheduler().now();
    step9(&mut m, 1);
    assert_eq!(m.inspect().scheduler().now() - before, 1);
}
