// SPDX-License-Identifier: Apache-2.0
use super::*;

/// A bus that serves nothing; the CPU executes no instruction yet.
struct NullBus;

impl Bus for NullBus {
    fn fetch32(&mut self, _addr: u32) -> u32 {
        0
    }
    fn fetch16(&mut self, _addr: u32) -> u16 {
        0
    }
    fn read8(&mut self, _addr: u32) -> u8 {
        0
    }
    fn read16(&mut self, _addr: u32) -> u16 {
        0
    }
    fn read32(&mut self, _addr: u32) -> u32 {
        0
    }
    fn write8(&mut self, _addr: u32, _value: u8) {}
    fn write16(&mut self, _addr: u32, _value: u16) {}
    fn write32(&mut self, _addr: u32, _value: u32) {}
}

/// Mode bits for the CPSR, with I and F set so the value looks like real code.
fn cpsr_in(mode_bits: u32) -> u32 {
    0xC0 | mode_bits
}

/// Mode-bit values and the mode each one must decode to; `None` where no mode exists.
fn mode_table() -> Vec<(u32, Option<Mode>)> {
    vec![
        (0x10, Some(Mode::User)),
        (0x11, Some(Mode::Fiq)),
        (0x12, Some(Mode::Irq)),
        (0x13, Some(Mode::Supervisor)),
        (0x17, Some(Mode::Abort)),
        (0x1B, Some(Mode::Undefined)),
        (0x1F, Some(Mode::System)),
        (0x00, None),
        (0x03, None),
        (0x14, None),
        (0x1A, None),
    ]
}

/// Decodes every row of the mode table and lists the rows that decode differently.
fn mode_decode_failures() -> Vec<(u32, Option<Mode>, Option<Mode>)> {
    mode_table()
        .into_iter()
        .map(|(bits, want)| (bits, want, Mode::from_bits(bits)))
        .filter(|(_, want, got)| want != got)
        .collect()
}

/// Banks and the SPSR slot each one owns, `None` for the bank with no SPSR.
fn spsr_slots() -> Vec<(Mode, Bank)> {
    vec![
        (Mode::Fiq, Bank::Fiq),
        (Mode::Supervisor, Bank::Supervisor),
        (Mode::Abort, Bank::Abort),
        (Mode::Irq, Bank::Irq),
        (Mode::Undefined, Bank::Undefined),
        (Mode::User, Bank::UserSystem),
        (Mode::System, Bank::UserSystem),
    ]
}

fn mode_bank_failures() -> Vec<Mode> {
    spsr_slots()
        .into_iter()
        .filter(|(mode, bank)| mode.bank() != *bank)
        .map(|(mode, _)| mode)
        .collect()
}

/// Writes a distinct value into R0–R14 of the mode, then returns every register in the file
/// after a round trip through the other modes.
fn registers_after_round_trip(first: u32, via: u32) -> Vec<u32> {
    let mut regs = Registers::new();
    regs.set_cpsr(cpsr_in(first));
    (0..15).for_each(|i| regs.set(i, 100 + u32::try_from(i).unwrap()));
    regs.set_cpsr(cpsr_in(via));
    regs.set_cpsr(cpsr_in(first));
    (0..15).map(|i| regs.get(i)).collect()
}

#[test]
fn given_mode_bit_patterns_when_decoding_then_no_pattern_decodes_wrongly() {
    assert_eq!(mode_decode_failures(), vec![]);
}

#[test]
fn given_each_mode_when_asking_for_its_bank_then_user_and_system_share_one() {
    assert_eq!(mode_bank_failures(), Vec::<Mode>::new());
}

#[test]
fn given_mode_bits_with_high_bits_set_when_decoding_then_only_the_low_five_count() {
    assert_eq!(Mode::from_bits(0xFFFF_FFF3), Some(Mode::Supervisor));
}

#[test]
fn given_new_registers_when_reading_cpsr_then_supervisor_with_interrupts_disabled() {
    let regs = Registers::new();
    assert_eq!((regs.cpsr(), regs.bank()), (0xD3, Bank::Supervisor));
}

#[test]
fn given_new_registers_when_reading_every_register_then_all_are_zero() {
    let regs = Registers::default();
    let all: Vec<u32> = (0..16).map(|i| regs.get(i)).collect();
    assert_eq!(all, vec![0; 16]);
}

#[test]
fn given_r0_written_in_supervisor_when_switching_to_fiq_then_r0_is_unchanged() {
    let mut regs = Registers::new();
    regs.set(0, 0xAAAA);
    regs.set_cpsr(cpsr_in(0x11));
    assert_eq!(regs.get(0), 0xAAAA);
}

#[test]
fn given_r13_written_in_supervisor_when_switching_to_irq_then_irq_sees_its_own_r13() {
    let mut regs = Registers::new();
    regs.set(13, 0x1111);
    regs.set_cpsr(cpsr_in(0x12));
    assert_eq!(regs.get(13), 0);
}

/// Writes a distinct R13 in each mode, then reads them all back in the same order.
fn r13_per_mode(modes: &[u32]) -> Vec<u32> {
    let mut regs = Registers::new();
    modes.iter().for_each(|&m| {
        regs.set_cpsr(cpsr_in(m));
        regs.set(13, m * 0x100);
    });
    modes
        .iter()
        .map(|&m| {
            regs.set_cpsr(cpsr_in(m));
            regs.get(13)
        })
        .collect()
}

/// Writes R8–R12 in Supervisor, enters IRQ, and reads R8–R12 there.
fn shared_high_registers_seen_by_irq() -> Vec<u32> {
    let mut regs = Registers::new();
    (8..=12).for_each(|i| regs.set(i, 50 + u32::try_from(i).unwrap()));
    regs.set_cpsr(cpsr_in(0x12));
    (8..=12).map(|i| regs.get(i)).collect()
}

#[test]
fn given_r13_written_in_each_mode_when_returning_then_every_mode_keeps_its_value() {
    let seen = r13_per_mode(&[0x11, 0x12, 0x13, 0x17, 0x1B, 0x1F]);
    assert_eq!(seen, vec![0x1100, 0x1200, 0x1300, 0x1700, 0x1B00, 0x1F00]);
}

#[test]
fn given_r8_written_in_supervisor_when_entering_fiq_then_fiq_has_its_own_r8_and_supervisor_keeps_its_value()
 {
    let mut regs = Registers::new();
    regs.set(8, 1);
    regs.set_cpsr(cpsr_in(0x11));
    let in_fiq = regs.get(8);
    regs.set(8, 2);
    regs.set_cpsr(cpsr_in(0x13));
    assert_eq!((in_fiq, regs.get(8), regs.banked(Bank::Fiq, 8)), (0, 1, 2));
}

#[test]
fn given_r8_to_r12_written_in_supervisor_when_entering_irq_then_irq_shares_them() {
    assert_eq!(
        shared_high_registers_seen_by_irq(),
        vec![58, 59, 60, 61, 62]
    );
}

#[test]
fn given_user_registers_when_going_through_fiq_and_back_then_all_fifteen_survive() {
    let want: Vec<u32> = (0..15).map(|i| 100 + i).collect();
    assert_eq!(registers_after_round_trip(0x10, 0x11), want);
}

#[test]
fn given_supervisor_registers_when_going_through_each_other_mode_and_back_then_all_fifteen_survive()
{
    let want: Vec<u32> = (0..15).map(|i| 100 + i).collect();
    let bad: Vec<u32> = [0x10, 0x11, 0x12, 0x17, 0x1B, 0x1F]
        .into_iter()
        .filter(|&via| registers_after_round_trip(0x13, via) != want)
        .collect();
    assert_eq!(bad, Vec::<u32>::new());
}

#[test]
fn given_system_mode_when_switching_to_user_then_r13_and_r14_are_shared() {
    let mut regs = Registers::new();
    regs.set_cpsr(cpsr_in(0x1F));
    regs.set(13, 0x7777);
    regs.set(14, 0x8888);
    regs.set_cpsr(cpsr_in(0x10));
    assert_eq!((regs.get(13), regs.get(14)), (0x7777, 0x8888));
}

#[test]
fn given_irq_active_when_reading_the_supervisor_bank_then_its_stored_r13_is_returned() {
    let mut regs = Registers::new();
    regs.set(13, 0x5000);
    regs.set_cpsr(cpsr_in(0x12));
    assert_eq!(regs.banked(Bank::Supervisor, 13), 0x5000);
}

#[test]
fn given_irq_active_when_writing_the_supervisor_bank_then_the_value_appears_on_entering_it() {
    let mut regs = Registers::new();
    regs.set_cpsr(cpsr_in(0x12));
    regs.set_banked(Bank::Supervisor, 14, 0x6000);
    regs.set_cpsr(cpsr_in(0x13));
    assert_eq!(regs.get(14), 0x6000);
}

#[test]
fn given_active_bank_when_reading_it_through_banked_then_the_visible_value_is_returned() {
    let mut regs = Registers::new();
    regs.set(9, 0x42);
    assert_eq!(regs.banked(Bank::Supervisor, 9), 0x42);
}

#[test]
fn given_fiq_active_when_reading_another_bank_r8_then_the_shared_user_copy_is_returned() {
    let mut regs = Registers::new();
    regs.set(8, 0x99);
    regs.set_cpsr(cpsr_in(0x11));
    assert_eq!(regs.banked(Bank::Irq, 8), 0x99);
}

#[test]
fn given_unknown_mode_bits_when_writing_cpsr_then_bits_are_kept_and_bank_is_unchanged() {
    let mut regs = Registers::new();
    regs.set(13, 0x1234);
    regs.set_cpsr(0xF000_0000 | 0x14);
    assert_eq!(
        (regs.cpsr(), regs.bank(), regs.get(13)),
        (0xF000_0014, Bank::Supervisor, 0x1234)
    );
}

#[test]
fn given_spsr_written_for_irq_when_reading_it_then_other_banks_are_untouched() {
    let mut regs = Registers::new();
    regs.set_spsr(Bank::Irq, 0xABCD);
    let all: Vec<Option<u32>> = [
        Bank::Fiq,
        Bank::Supervisor,
        Bank::Abort,
        Bank::Irq,
        Bank::Undefined,
    ]
    .iter()
    .map(|&b| regs.spsr(b))
    .collect();
    assert_eq!(all, vec![Some(0), Some(0), Some(0), Some(0xABCD), Some(0)]);
}

#[test]
fn given_user_system_bank_when_reading_or_writing_spsr_then_there_is_none() {
    let mut regs = Registers::new();
    regs.set_spsr(Bank::UserSystem, 0xFFFF);
    assert_eq!(regs.spsr(Bank::UserSystem), None);
}

#[test]
fn given_register_index_above_fifteen_when_reading_then_only_the_low_four_bits_count() {
    let mut regs = Registers::new();
    regs.set(0x13, 7);
    assert_eq!(regs.get(3), 7);
}

#[test]
fn given_new_cpu_when_stepping_then_registers_are_unchanged() {
    let mut cpu = Cpu::<false>::new();
    let before = cpu.clone();
    cpu.step(&mut NullBus);
    assert_eq!(cpu, before);
}

#[test]
fn given_new_cpu_when_editing_registers_through_regs_mut_then_regs_shows_the_edit() {
    let mut cpu = Cpu::<true>::default();
    cpu.regs_mut().set(5, 55);
    assert_eq!(cpu.regs().get(5), 55);
}

#[test]
fn given_a_bus_when_called_through_the_trait_then_every_method_is_reachable() {
    let mut bus = NullBus;
    bus.write8(0, 1);
    bus.write16(0, 1);
    bus.write32(0, 1);
    let read = (
        bus.fetch32(0),
        bus.fetch16(0),
        bus.read8(0),
        bus.read16(0),
        bus.read32(0),
    );
    assert_eq!(read, (0, 0, 0, 0, 0));
}
