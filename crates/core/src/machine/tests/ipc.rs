// SPDX-License-Identifier: Apache-2.0
//! IPC as the two CPUs reach it through the bus, and its IRQs driving each CPU through
//! `Machine`. Expected values come from `docs/hw/ipc.md` ("Registers", "Behavior", "Wider and
//! narrower accesses", "Unverified") and `docs/hw/interrupts.md`, never from running the code.
//!
//! The bus has two read paths (see `crate::bus::view`): a CPU's read through its view takes a word out
//! of the receive FIFO, the peeks (debugger, runner) only show it.
//!
//! ARM7 delivery (the IRQ exception) is pinned here rather than in a test ROM because the ARM7's
//! BIOS vectors are #27's (`docs/hw/interrupts.md`, "The IRQ dispatcher is not the controller").

use crate::cart::test_image::{Image, words};
use crate::cpu::Bus;
use crate::io::ipc::{IPCFIFOCNT_ADDR, IPCFIFORECV_ADDR, IPCFIFOSEND_ADDR, IPCSYNC_ADDR};
use crate::io::irq::{IPCSYNC_IRQ, RECV_NOT_EMPTY_IRQ, SEND_EMPTY_IRQ};
use crate::machine::*;

const ENTRY7: u32 = 0x0238_0000;
const ENABLE: u16 = 1 << 15;
const ERROR: u16 = 1 << 14;
const RECV_IRQ_ENABLE: u16 = 1 << 10;
const SEND_IRQ_ENABLE: u16 = 1 << 2;
const RECV_EMPTY: u16 = 1 << 8;
const IRQ_ENTRY_CPSR: u32 = 0x92;
const IRQ_VECTOR_PC: u32 = 0x18 + 8;
const SYSTEM_I_CLEAR: u32 = 0x1F;

const PROGRAM: [u32; 3] = [0xE3A0_0055, 0xE3A0_1066, 0xEAFF_FFFE];

fn machine() -> Machine {
    let mut image = Image::valid();
    image.arm9_code = words(&PROGRAM);
    image.arm7_code = words(&PROGRAM);
    let mut machine = Machine::new();
    machine.load_rom(image.build()).expect("synthetic image");
    machine
}

/// A machine whose both CPUs have switched their FIFOs on.
fn enabled() -> Machine {
    let mut m = machine();
    m.bus9().write16(IPCFIFOCNT_ADDR, ENABLE);
    m.bus7(0).write16(IPCFIFOCNT_ADDR, ENABLE);
    m
}

fn send9(m: &mut Machine, words: &[u32]) {
    words
        .iter()
        .for_each(|&w| m.bus9().write32(IPCFIFOSEND_ADDR, w));
}

fn cnt7(m: &Machine) -> u16 {
    m.arm7_peek16(0, IPCFIFOCNT_ADDR)
}

fn cnt9(m: &Machine) -> u16 {
    m.arm9_peek16(IPCFIFOCNT_ADDR)
}

// ----------------------------------------------------------------------- IPCSYNC

#[test]
fn given_the_arm9_writes_ipcsync_through_the_bus_when_the_arm7_reads_it_then_bits_0_to_3_hold_the_nibble()
 {
    let mut m = machine();
    m.bus9().write16(IPCSYNC_ADDR, 0x0A00);
    assert_eq!(m.arm7_peek16(0, IPCSYNC_ADDR), 0x000A);
}

#[test]
fn given_the_arm7_writes_ipcsync_through_the_bus_when_the_arm9_reads_it_through_the_cpu_path_then_bits_0_to_3_hold_the_nibble()
 {
    let mut m = machine();
    m.bus7(0).write16(IPCSYNC_ADDR, 0x0600);
    assert_eq!(m.bus9().read16(IPCSYNC_ADDR), 0x0006);
}

#[test]
fn given_a_word_write_to_ipcsync_when_read_back_then_the_two_bytes_past_it_are_not_a_register() {
    let mut m = machine();
    m.bus9().write32(IPCSYNC_ADDR, 0xFFFF_0F00);
    assert_eq!(m.arm9_peek32(IPCSYNC_ADDR), 0x0000_0F00);
}

#[test]
fn given_the_arm9_sends_the_sync_irq_when_the_arm7_enabled_it_then_only_the_arm7_if_bit_16_is_set()
{
    let mut m = machine();
    m.bus7(0).write16(IPCSYNC_ADDR, 0x4000);
    m.bus9().write16(IPCSYNC_ADDR, 0x2000);
    assert_eq!((m.irq9().if_(), m.irq7().if_()), (0, IPCSYNC_IRQ));
}

#[test]
fn given_the_arm7_sends_the_sync_irq_when_the_arm9_enabled_it_then_only_the_arm9_if_bit_16_is_set()
{
    let mut m = machine();
    m.bus9().write16(IPCSYNC_ADDR, 0x4000);
    m.bus7(0).write16(IPCSYNC_ADDR, 0x2000);
    assert_eq!((m.irq9().if_(), m.irq7().if_()), (IPCSYNC_IRQ, 0));
}

// ----------------------------------------------------------------- the FIFO through the bus

#[test]
fn given_a_word_written_to_ipcfifosend_by_the_arm9_when_the_arm7_reads_ipcfiforecv_then_it_is_the_word()
 {
    let mut m = enabled();
    send9(&mut m, &[0xCAFE_F00D]);
    assert_eq!(m.bus7(0).read32(IPCFIFORECV_ADDR), 0xCAFE_F00D);
}

#[test]
fn given_a_word_written_to_ipcfifosend_by_the_arm7_when_the_arm9_reads_ipcfiforecv_then_it_is_the_word()
 {
    let mut m = enabled();
    m.bus7(0).write32(IPCFIFOSEND_ADDR, 0x1234_5678);
    assert_eq!(m.bus9().read32(IPCFIFORECV_ADDR), 0x1234_5678);
}

#[test]
fn given_a_word_in_the_fifo_when_the_arm9_peeks_ipcfiforecv_twice_then_both_peeks_show_it_and_the_cpu_read_still_gets_it()
 {
    let mut m = enabled();
    m.bus7(0).write32(IPCFIFOSEND_ADDR, 0x1234_5678);
    let peeks = [
        m.arm9_peek32(IPCFIFORECV_ADDR),
        m.arm9_peek32(IPCFIFORECV_ADDR),
    ];
    assert_eq!(
        (peeks, m.bus9().read32(IPCFIFORECV_ADDR)),
        ([0x1234_5678; 2], 0x1234_5678)
    );
}

#[test]
fn given_two_words_in_flight_when_the_arm7_reads_one_word_then_exactly_one_is_taken() {
    let mut m = enabled();
    send9(&mut m, &[0xA1, 0xA2]);
    let first = m.bus7(0).read32(IPCFIFORECV_ADDR);
    assert_eq!((first, cnt7(&m) & RECV_EMPTY), (0xA1, 0));
}

#[test]
fn given_two_words_in_flight_when_the_arm7_reads_two_words_then_the_second_comes_second_and_the_fifo_is_empty()
 {
    let mut m = enabled();
    send9(&mut m, &[0xA1, 0xA2]);
    let first = m.bus7(0).read32(IPCFIFORECV_ADDR);
    let second = m.bus7(0).read32(IPCFIFORECV_ADDR);
    assert_eq!(
        (first, second, cnt7(&m) & RECV_EMPTY),
        (0xA1, 0xA2, RECV_EMPTY)
    );
}

#[test]
fn given_a_word_in_flight_when_it_is_peeked_twice_then_both_peeks_show_it_and_the_fifo_keeps_it() {
    let mut m = enabled();
    send9(&mut m, &[0xB7]);
    let first = m.arm7_peek32(0, IPCFIFORECV_ADDR);
    let second = m.arm7_peek32(0, IPCFIFORECV_ADDR);
    assert_eq!((first, second, cnt7(&m) & RECV_EMPTY), (0xB7, 0xB7, 0));
}

#[test]
fn given_a_word_in_flight_when_the_arm7_peeks_and_then_reads_then_the_read_still_gets_the_word() {
    let mut m = enabled();
    send9(&mut m, &[0xB7]);
    m.arm7_peek32(0, IPCFIFORECV_ADDR);
    assert_eq!(m.bus7(0).read32(IPCFIFORECV_ADDR), 0xB7);
}

#[test]
fn given_an_empty_receive_fifo_when_it_is_peeked_then_no_error_is_set() {
    let m = enabled();
    m.arm7_peek32(0, IPCFIFORECV_ADDR);
    assert_eq!(cnt7(&m) & ERROR, 0);
}

#[test]
fn given_an_empty_receive_fifo_when_the_arm7_reads_it_with_the_cpu_path_then_the_error_is_set() {
    let mut m = enabled();
    m.bus7(0).read32(IPCFIFORECV_ADDR);
    assert_eq!(cnt7(&m) & ERROR, ERROR);
}

#[test]
fn given_the_arm9_peeks_ipcfifocnt_through_arm9_peek16_when_the_fifo_is_empty_then_it_shows_the_status_without_side_effects()
 {
    let m = enabled();
    assert_eq!(
        (
            m.arm9_peek16(IPCFIFOCNT_ADDR),
            m.arm9_peek32(IPCFIFORECV_ADDR),
            cnt9(&m) & ERROR
        ),
        (ENABLE | 0x0101, 0, 0)
    );
}

#[test]
fn given_a_halfword_write_to_ipcfifosend_when_the_arm7_checks_then_no_word_was_sent_unverified() {
    let mut m = enabled();
    m.bus9().write16(IPCFIFOSEND_ADDR, 0x1234);
    assert_eq!((cnt7(&m) & RECV_EMPTY, cnt9(&m) & ERROR), (RECV_EMPTY, 0));
}

#[test]
fn given_a_byte_write_to_ipcfifosend_when_the_arm7_checks_then_no_word_was_sent_unverified() {
    let mut m = enabled();
    m.bus9().write8(IPCFIFOSEND_ADDR + 3, 0x12);
    assert_eq!((cnt7(&m) & RECV_EMPTY, cnt9(&m) & ERROR), (RECV_EMPTY, 0));
}

#[test]
fn given_two_words_in_flight_when_the_arm7_reads_a_halfword_then_one_word_is_taken_and_its_low_half_returned_unverified()
 {
    let mut m = enabled();
    send9(&mut m, &[0xAABB_CCDD, 0x1122_3344]);
    let half = m.bus7(0).read16(IPCFIFORECV_ADDR);
    let next = m.bus7(0).read32(IPCFIFORECV_ADDR);
    assert_eq!((half, next), (0xCCDD, 0x1122_3344));
}

#[test]
fn given_a_word_in_flight_when_the_arm7_reads_a_byte_at_offset_1_then_that_byte_of_the_word_comes_back_unverified()
 {
    let mut m = enabled();
    send9(&mut m, &[0xAABB_CCDD]);
    assert_eq!(m.bus7(0).read8(IPCFIFORECV_ADDR + 1), 0xCC);
}

#[test]
fn given_an_arm9_cpu_read_of_ipcfiforecv_when_the_fifo_has_a_word_then_it_is_taken() {
    let mut m = enabled();
    m.bus7(0).write32(IPCFIFOSEND_ADDR, 0x77);
    m.bus9().read32(IPCFIFORECV_ADDR);
    assert_eq!(cnt9(&m) & RECV_EMPTY, RECV_EMPTY);
}

#[test]
fn given_a_word_at_ipcfiforecv_plus_4_when_read_then_it_is_not_the_register_unverified() {
    let mut m = enabled();
    send9(&mut m, &[0x99]);
    assert_eq!(m.bus7(0).read32(IPCFIFORECV_ADDR + 4), 0);
}

// -------------------------------------------------------------------- IRQs reach the IF

#[test]
fn given_the_receive_irq_enabled_when_the_arm9_sends_then_if18_is_set_on_the_arm7_only() {
    let mut m = enabled();
    m.bus7(0).write16(IPCFIFOCNT_ADDR, ENABLE | RECV_IRQ_ENABLE);
    send9(&mut m, &[1]);
    assert_eq!((m.irq9().if_(), m.irq7().if_()), (0, RECV_NOT_EMPTY_IRQ));
}

#[test]
fn given_the_send_empty_irq_enabled_with_a_word_in_flight_when_the_arm7_takes_it_then_if17_is_set_on_the_arm9_only()
 {
    let mut m = enabled();
    send9(&mut m, &[1]);
    m.bus9().write16(IPCFIFOCNT_ADDR, ENABLE | SEND_IRQ_ENABLE);
    m.bus7(0).read32(IPCFIFORECV_ADDR);
    assert_eq!((m.irq9().if_(), m.irq7().if_()), (SEND_EMPTY_IRQ, 0));
}

#[test]
fn given_if18_acknowledged_while_the_condition_holds_when_a_second_word_arrives_then_it_is_not_set_again()
 {
    let mut m = enabled();
    m.bus7(0).write16(IPCFIFOCNT_ADDR, ENABLE | RECV_IRQ_ENABLE);
    send9(&mut m, &[1]);
    m.bus7(0)
        .write32(crate::io::irq::IF_ADDR, RECV_NOT_EMPTY_IRQ);
    send9(&mut m, &[2]);
    assert_eq!(m.irq7().if_(), 0);
}

// -------------------------------------------------------------- ARM7 delivery

/// A machine whose ARM7 has IME = 1, IE = `ie` and I clear.
fn arm7_listening(ie: u32) -> Machine {
    let mut m = enabled();
    m.bus7(0).write32(crate::io::irq::IME_ADDR, 1);
    m.bus7(0).write32(crate::io::irq::IE_ADDR, ie);
    m.arm7.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m
}

/// (CPSR, r0) of the ARM7 after one step.
fn arm7_after_step(m: &mut Machine) -> (u32, u32) {
    m.step_arm7_instruction();
    (m.arm7().regs().cpsr(), m.arm7().regs().get(0))
}

#[test]
fn given_the_arm9_sends_the_sync_irq_when_the_arm7_listens_then_the_arm7_enters_irq_mode_instead_of_running()
 {
    let mut m = arm7_listening(IPCSYNC_IRQ);
    m.bus7(0).write16(IPCSYNC_ADDR, 0x4000);
    m.bus9().write16(IPCSYNC_ADDR, 0x2000);
    assert_eq!(arm7_after_step(&mut m), (IRQ_ENTRY_CPSR, 0));
}

#[test]
fn given_the_arm9_sends_a_word_when_the_arm7_listens_for_the_receive_irq_then_the_arm7_enters_irq_mode_at_the_vector()
 {
    let mut m = arm7_listening(RECV_NOT_EMPTY_IRQ);
    m.bus7(0).write16(IPCFIFOCNT_ADDR, ENABLE | RECV_IRQ_ENABLE);
    send9(&mut m, &[0x42]);
    m.step_arm7_instruction();
    let r = m.arm7().regs();
    assert_eq!(
        (r.cpsr(), r.banked(crate::cpu::Bank::Irq, 14), r.get(15)),
        (IRQ_ENTRY_CPSR, ENTRY7 + 4, IRQ_VECTOR_PC)
    );
}

#[test]
fn given_the_arm9_sends_the_sync_irq_when_the_arm7_does_not_listen_for_bit_16_then_the_arm7_runs_on()
 {
    let mut m = arm7_listening(RECV_NOT_EMPTY_IRQ);
    m.bus7(0).write16(IPCSYNC_ADDR, 0x4000);
    m.bus9().write16(IPCSYNC_ADDR, 0x2000);
    assert_eq!(arm7_after_step(&mut m), (SYSTEM_I_CLEAR, 0x55));
}

#[test]
fn given_the_arm9_sends_the_sync_irq_when_only_the_arm7_listens_then_the_arm9_is_not_interrupted() {
    let mut m = arm7_listening(IPCSYNC_IRQ);
    m.bus7(0).write16(IPCSYNC_ADDR, 0x4000);
    m.bus9().write16(IPCSYNC_ADDR, 0x2000);
    m.arm9.regs_mut().set_cpsr(SYSTEM_I_CLEAR);
    m.step_arm9_instruction();
    assert_eq!(m.arm9().regs().get(0), 0x55);
}

// --------------------------------------------------------------- debugger accessor

#[test]
fn given_a_word_in_flight_when_the_debugger_reads_the_ipc_accessor_then_it_shows_both_halves() {
    let mut m = enabled();
    send9(&mut m, &[1]);
    m.bus9().write16(IPCSYNC_ADDR, 0x0300);
    assert_eq!(
        (m.ipc().control::<true>(), m.ipc().sync::<false>()),
        (ENABLE | RECV_EMPTY, 0x0003)
    );
}
