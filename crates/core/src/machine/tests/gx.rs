// SPDX-License-Identifier: Apache-2.0
//! The 3D engine stub through `Machine` (`docs/hw/gx-stub.md`): the ARM9's bus reaching it, the ARM7
//! still reaching the sound registers at the same addresses, POWCNT1 bit 3 gating the geometry
//! block, the level-driven IF bit 21, and a savestate taken in the middle of a command.

use super::dma::machine;
use super::*;
use crate::cpu::Bus;

const GXFIFO: u32 = 0x0400_0400;
const GXSTAT: u32 = 0x0400_0600;
const POWCNT1: u32 = 0x0400_0304;
const IF: u32 = 0x0400_0214;
const SOUND0CNT: u32 = 0x0400_0400;
const MTX_MODE: u32 = 0x10;
const MTX_PUSH: u32 = 0x11;
/// FIFO empty and less than half full.
const RESET: u32 = 0x0600_0000;
const ERROR: u32 = 1 << 15;
/// GXSTAT bit 13: the projection pointer.
const PROJECTION_LEVEL: u32 = 1 << 13;
const GXFIFO_IRQ: u32 = 1 << 21;
/// POWCNT1 as direct boot leaves it (`820Fh`) with the geometry engine bit (3) cleared.
const POWCNT1_GEOMETRY_OFF: u16 = 0x820F & !(1 << 3);

#[test]
fn given_a_booted_machine_when_the_arm9_reads_gxstat_then_the_fifo_reads_empty() {
    let mut m = machine();
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_the_arm7_when_it_reads_the_geometry_addresses_then_it_sees_the_sound_registers_not_gxstat()
{
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write32(SOUND0CNT, 0x7F);
    let arm7 = (
        m.bus7(at + 1).read32(SOUND0CNT),
        m.bus7(at + 1).read32(GXSTAT),
    );
    assert_eq!(arm7, (0x7F, 0));
}

#[test]
fn given_the_arm7_writes_a_command_word_when_the_arm9_reads_gxstat_then_nothing_changed() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write32(GXFIFO, MTX_PUSH);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_a_push_in_projection_mode_when_the_arm9_reads_gxstat_then_the_projection_level_is_one() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    assert_eq!(m.bus9().read32(GXSTAT), RESET | PROJECTION_LEVEL);
}

#[test]
fn given_a_push_when_inspected_then_the_pointer_is_visible() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    assert_eq!(m.inspect().gpu3d().projection_pointer(), 1);
}

#[test]
fn given_the_geometry_engine_is_off_when_a_command_is_written_then_it_is_dropped() {
    let mut m = machine();
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_the_geometry_engine_is_off_when_a_port_is_written_then_it_is_dropped() {
    let mut m = machine();
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(0x0400_0444, 0);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_an_error_and_the_engine_off_when_bit_15_is_written_then_the_error_stays() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(GXSTAT, ERROR);
    assert_eq!(m.bus9().read32(GXSTAT), RESET | ERROR);
}

#[test]
fn given_an_error_and_the_engine_on_when_bit_15_is_written_then_the_error_clears() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write32(GXSTAT, ERROR);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_the_engine_was_off_when_it_is_powered_on_again_then_commands_run() {
    let mut m = machine();
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write16(POWCNT1, 0x820F);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    assert_eq!(m.bus9().read32(GXSTAT), RESET | PROJECTION_LEVEL);
}

#[test]
fn given_the_fifo_irq_mode_is_half_full_when_it_is_set_then_if_bit_21_is_raised() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_mode_is_empty_when_it_is_set_then_if_bit_21_is_raised() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 2 << 30);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_mode_is_off_when_gxstat_is_written_then_if_bit_21_is_not_raised() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 0);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, 0);
}

#[test]
fn given_the_fifo_irq_is_on_when_if_bit_21_is_acknowledged_then_it_stays_set() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(IF, GXFIFO_IRQ);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_is_on_when_other_if_bits_are_acknowledged_then_bit_21_stays_set() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(IF, !0);
    assert_eq!(m.inspect().irq9().if_(), GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_mode_is_turned_off_when_if_bit_21_is_acknowledged_then_it_clears() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(GXSTAT, 0);
    m.bus9().write32(IF, GXFIFO_IRQ);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, 0);
}

#[test]
fn given_the_fifo_irq_mode_is_turned_off_when_nothing_is_acknowledged_then_the_bit_already_set_stays()
 {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(GXSTAT, 0);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_is_on_when_the_arm7_acknowledges_its_own_if_then_the_arm9_bit_is_unaffected()
{
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write32(IF, GXFIFO_IRQ);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

/// A machine whose ARM9 sent `MTX_MODE` as a packed word with two pushes after it, and not yet the
/// mode's parameter.
fn mid_command() -> Machine {
    let mut m = machine();
    m.bus9()
        .write32(GXFIFO, MTX_MODE | MTX_PUSH << 8 | MTX_PUSH << 16);
    m
}

#[test]
fn given_a_savestate_in_the_middle_of_a_packed_word_when_loaded_then_the_parameter_completes_it() {
    let m = mid_command();
    let state = m.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    loaded.bus9().write32(GXFIFO, 1);
    // Mode 1 and two pushes: the coordinate level is 2, GXSTAT bits 8-12.
    assert_eq!(
        (result, loaded.bus9().read32(GXSTAT)),
        (Ok(()), RESET | 2 << 8)
    );
}

#[test]
fn given_the_same_geometry_writes_when_two_machines_save_then_the_states_are_identical() {
    let (a, b) = (mid_command(), mid_command());
    assert_eq!(a.save_state(), b.save_state());
}
