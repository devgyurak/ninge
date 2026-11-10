// SPDX-License-Identifier: Apache-2.0
//! Tests of `regs`.

use super::*;

#[test]
fn given_power_on_registers_when_checked_then_they_are_consistent() {
    assert!(Registers::new().is_consistent());
}

#[test]
fn given_a_cpsr_written_with_irq_mode_when_checked_then_the_banks_follow_and_it_is_consistent() {
    let mut regs = Registers::new();
    regs.set_cpsr(0x12);
    assert!(regs.is_consistent());
}

#[test]
fn given_irq_mode_bits_but_the_user_bank_when_checked_then_it_is_inconsistent() {
    let mut regs = Registers::new();
    regs.cpsr = 0x12;
    regs.bank = Bank::UserSystem;
    assert!(!regs.is_consistent());
}

#[test]
fn given_mode_bits_naming_no_mode_when_checked_then_any_bank_is_consistent() {
    let mut regs = Registers::new();
    regs.cpsr = 0x00;
    regs.bank = Bank::Fiq;
    assert!(regs.is_consistent());
}
