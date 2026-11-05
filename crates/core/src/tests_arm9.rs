// SPDX-License-Identifier: Apache-2.0
//! The ARM9 executing hand-encoded opcodes out of main RAM through the memory map. Expected
//! values come from `docs/hw/arm9-memory-map.md`, `docs/hw/vram-banks.md`, `docs/hw/cp15.md`,
//! `docs/hw/arm-load-store.md` and `docs/hw/arm-branch-pipeline.md`.
//!
//! The program sits at `ENTRY`. R15 reads `ENTRY + 8` while the first opcode executes and the
//! pipeline already holds the first two opcodes (`arm-branch-pipeline.md`, "The prefetch
//! pipeline"); each step fetches the opcode at R15 from the bus.

use super::*;
use crate::cpu::{Bus, Tcm};

const ENTRY: u32 = 0x0200_0000;
const SVC: u32 = 0x13;
const RAM_PERIOD: u32 = 0x0040_0000;
const X: u32 = 0x1122_3344;

// Encodings, condition AL.
/// `MCR p15, 0, R0, C9, C1, 0` (DTCM region register).
const MCR_DTCM_REGION: u32 = 0xEE09_0F11;
/// `MCR p15, 0, R0, C9, C1, 1` (ITCM region register).
const MCR_ITCM_REGION: u32 = 0xEE09_0F31;
/// `MCR p15, 0, R1, C1, C0, 0` (control register).
const MCR_CONTROL_R1: u32 = 0xEE01_1F10;
/// `STR R3, [R2]`.
const STR_R3_R2: u32 = 0xE582_3000;
/// `STR R4, [R2, #4]`.
const STR_R4_R2_4: u32 = 0xE582_4004;
/// `LDR R4, [R2]`.
const LDR_R4_R2: u32 = 0xE592_4000;
/// `LDR R5, [R2, #1]`.
const LDR_R5_R2_1: u32 = 0xE592_5001;
/// `STRB R3, [R2]`.
const STRB_R3_R2: u32 = 0xE5C2_3000;
/// `STR R6, [R5]`.
const STR_R6_R5: u32 = 0xE585_6000;
/// `STRH R6, [R5, #4]`.
const STRH_R6_R5_4: u32 = 0xE1C5_60B4;
/// `STRB R6, [R5, #8]`.
const STRB_R6_R5_8: u32 = 0xE5C5_6008;
/// `MOV PC, #0`.
const MOV_PC_0: u32 = 0xE3A0_F000;
/// `MOV R6, #0x55`.
const MOV_R6_55: u32 = 0xE3A0_6055;
/// `MOV R7, #0x66`.
const MOV_R7_66: u32 = 0xE3A0_7066;

fn plain() -> TcmConfig {
    let off = Tcm {
        base: 0,
        size: 0,
        enabled: false,
        load_mode: false,
    };
    TcmConfig {
        itcm: off,
        dtcm: off,
    }
}

/// A machine with `program` in main RAM at `ENTRY`, Supervisor mode in ARM state, the pipeline
/// full, and the listed registers set.
fn boot(program: &[u32], regs: &[(usize, u32)]) -> Machine {
    let mut machine = Machine::new();
    let mut view = Arm9View::new(&mut machine.memory, plain());
    program.iter().enumerate().for_each(|(i, &op)| {
        view.write32(ENTRY + 4 * i as u32, op);
    });
    let first = [program[0], program.get(1).copied().unwrap_or(0)];
    machine.arm9.regs_mut().set_cpsr(SVC);
    machine.arm9.regs_mut().set(15, ENTRY + 8);
    machine.arm9.set_pipeline(first);
    regs.iter()
        .for_each(|&(i, v)| machine.arm9.regs_mut().set(i, v));
    machine
}

/// Boots `program` and steps `steps` instructions.
fn run(program: &[u32], regs: &[(usize, u32)], steps: usize) -> Machine {
    let mut machine = boot(program, regs);
    (0..steps).for_each(|_| machine.step_arm9());
    machine
}

fn reg(machine: &Machine, i: usize) -> u32 {
    machine.arm9.regs().get(i)
}

#[test]
fn given_an_mcr_enabling_the_dtcm_when_the_next_str_runs_then_the_word_is_in_the_tcm_and_not_in_the_ram_mirror()
 {
    // DTCM at 0x0210_0000, N = 5 (512 << 5 = 16 KiB); control bit 16 enables it.
    let dtcm = 0x0210_0000;
    let machine = run(
        &[MCR_DTCM_REGION, MCR_CONTROL_R1, STR_R3_R2],
        &[(0, dtcm | 5 << 1), (1, 1 << 16), (2, dtcm), (3, X)],
        3,
    );
    // The same RAM byte through the mirror 4 MiB up lies outside the DTCM region.
    let seen = (
        machine.arm9_peek32(dtcm),
        machine.arm9_peek32(dtcm + RAM_PERIOD),
    );
    assert_eq!(seen, (X, 0));
}

#[test]
fn given_a_dtcm_enabled_by_the_cpu_when_a_ldr_follows_the_str_then_the_register_holds_the_stored_word()
 {
    let dtcm = 0x0210_0000;
    let machine = run(
        &[MCR_DTCM_REGION, MCR_CONTROL_R1, STR_R3_R2, LDR_R4_R2],
        &[(0, dtcm | 5 << 1), (1, 1 << 16), (2, dtcm), (3, X)],
        4,
    );
    assert_eq!(reg(&machine, 4), X);
}

#[test]
fn given_a_str_to_main_ram_when_read_through_a_mirror_then_the_word_is_there() {
    let machine = run(&[STR_R3_R2], &[(2, ENTRY + 0x100), (3, X)], 1);
    let seen = (
        machine.arm9_peek32(ENTRY + 0x100),
        machine.arm9_peek32(ENTRY + 0x100 + RAM_PERIOD),
        machine.arm9_peek32(ENTRY + 0x100 + 3 * RAM_PERIOD),
    );
    assert_eq!(seen, (X, X, X));
}

#[test]
fn given_a_str_to_main_ram_when_ldr_loads_it_back_then_the_register_holds_it_and_an_odd_address_rotates_it()
 {
    let machine = run(
        &[STR_R3_R2, LDR_R4_R2, LDR_R5_R2_1],
        &[(2, ENTRY + 0x100), (3, X)],
        3,
    );
    // LDR at address + 1 reads the aligned word rotated right by 8 (arm-load-store.md).
    assert_eq!((reg(&machine, 4), reg(&machine, 5)), (X, 0x4411_2233));
}

/// Program for the ITCM: set the region (base 0, N = 6 = 32 KiB), enable it (control bit 18),
/// store two opcodes at 0 and 4, jump to 0.
fn itcm_program() -> [u32; 5] {
    [
        MCR_ITCM_REGION,
        MCR_CONTROL_R1,
        STR_R3_R2,
        STR_R4_R2_4,
        MOV_PC_0,
    ]
}

/// Runs the ITCM program and then the two opcodes that were stored into the ITCM.
fn run_from_itcm() -> Machine {
    run(
        &itcm_program(),
        &[
            (0, 6 << 1),
            (1, 1 << 18),
            (2, 0),
            (3, MOV_R6_55),
            (4, MOV_R7_66),
        ],
        7,
    )
}

#[test]
fn given_opcodes_stored_into_an_enabled_itcm_when_the_cpu_jumps_to_address_0_then_it_executes_them()
{
    let machine = run_from_itcm();
    assert_eq!((reg(&machine, 6), reg(&machine, 7)), (0x55, 0x66));
}

#[test]
fn given_opcodes_stored_into_an_enabled_itcm_when_read_by_the_peek_then_the_itcm_holds_them() {
    let machine = run_from_itcm();
    assert_eq!(
        (machine.arm9_peek32(0), machine.arm9_peek32(4)),
        (MOV_R6_55, MOV_R7_66)
    );
}

/// The program that enables bank A with a STRB and then stores through the window.
fn vram_program() -> [u32; 4] {
    [STRB_R3_R2, STR_R6_R5, STRH_R6_R5_4, STRB_R6_R5_8]
}

fn run_vram_stores() -> Machine {
    run(
        &vram_program(),
        &[
            (2, 0x0400_0240),
            (3, 0x80),
            (5, 0x0680_0000),
            (6, 0xA1B2_C3D4),
        ],
        4,
    )
}

#[test]
fn given_a_strb_to_vramcnt_a_when_executed_then_bank_a_is_enabled_in_lcdc_mode() {
    let machine = run_vram_stores();
    assert_eq!(
        machine.vram_a(),
        bus::VramBankState {
            enabled: true,
            mst: 0,
            offset: 0
        }
    );
}

#[test]
fn given_vram_a_enabled_by_the_cpu_when_str_and_strh_store_then_the_word_and_halfword_land() {
    let machine = run_vram_stores();
    assert_eq!(
        (
            machine.arm9_peek32(0x0680_0000),
            machine.arm9_peek16(0x0680_0004)
        ),
        (0xA1B2_C3D4, 0xC3D4)
    );
}

#[test]
fn given_vram_a_enabled_by_the_cpu_when_a_strb_stores_to_it_then_the_byte_is_ignored() {
    let machine = run_vram_stores();
    assert_eq!(machine.arm9_peek8(0x0680_0008), 0);
}

#[test]
fn given_vram_a_enabled_by_the_cpu_when_a_byte_is_peeked_then_it_is_the_stored_low_byte() {
    let machine = run_vram_stores();
    assert_eq!(machine.arm9_peek8(0x0680_0000), 0xD4);
}

/// A machine with `X` stored at `ENTRY + 0x20` through the bus.
fn machine_with_word() -> Machine {
    let mut machine = Machine::new();
    Arm9View::new(&mut machine.memory, plain()).write32(ENTRY + 0x20, X);
    machine
}

#[test]
fn given_a_word_in_main_ram_when_peeked_at_each_width_then_the_bytes_and_alignment_match_the_bus() {
    let machine = machine_with_word();
    let seen = (
        machine.arm9_peek32(ENTRY + 0x20),
        machine.arm9_peek32(ENTRY + 0x23),
        machine.arm9_peek16(ENTRY + 0x22),
        machine.arm9_peek16(ENTRY + 0x23),
        machine.arm9_peek8(ENTRY + 0x21),
    );
    assert_eq!(seen, (X, X, 0x1122, 0x1122, 0x33));
}

#[test]
fn given_a_peek_through_a_dtcm_when_the_tcm_is_enabled_then_it_sees_the_tcm_word() {
    let dtcm = 0x0210_0000;
    let machine = run(
        &[MCR_DTCM_REGION, MCR_CONTROL_R1, STR_R3_R2],
        &[(0, dtcm | 5 << 1), (1, 1 << 16), (2, dtcm + 4), (3, X)],
        3,
    );
    assert_eq!(
        (machine.arm9_peek32(dtcm + 4), machine.arm9_peek32(dtcm)),
        (X, 0)
    );
}
