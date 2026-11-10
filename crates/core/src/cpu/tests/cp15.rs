// SPDX-License-Identifier: Apache-2.0
//! The ARM9's CP15 through `Cpu::step` (MCR and MRC words) and through `Cp15` itself. Expected
//! values come from `docs/hw/cp15.md`, never from running the implementation.
//!
//! A register is named `CRn,CRm,opc2` as in the note; `reg` builds that triple with opc1 = 0 and
//! `Reg` carries opc1 first for the combinations the note leaves unmapped. Every instruction runs
//! at `ADDR` in Supervisor mode, so R15 reads `ADDR + 8` while it executes and `ADDR + 12` is the
//! value an MCR with Rd = R15 sends (the note's "Access").

use crate::cpu::*;

/// Address of the instruction under test.
const ADDR: u32 = 0x1000;
/// R15 while an ARM instruction executes (address + 8).
const ARM_PC: u32 = ADDR + 8;
/// The opcode already sitting in `pipeline[1]`.
const NEXT: u32 = 0xAAAA_0001;
/// What a register holds before an MRC, so a missing write shows.
const SENTINEL: u32 = 0xDEAD_BEEF;
/// Supervisor mode, flags clear, interrupts enabled.
const SVC: u32 = 0x13;
const HIGH_BASE: u32 = 0xFFFF_0000;
/// Control register bit 13, the exception vector base (`docs/hw/cp15.md`, "C1,C0,0").
const CONTROL_VECTORS: u32 = 1 << 13;

const SWI: u32 = 0xEF00_0123;
const BKPT: u32 = 0xE120_0070;
/// A word in the undefined space (bits 27-25 = 011, bit 4 = 1).
const UNDEFINED: u32 = 0xE600_0010;
/// `MOV R2, #5`, the instruction that proves the core is still running.
const MOV_R2_5: u32 = 0xE3A0_2005;

/// One code fetch: width in bits and address.
type Fetch = (u8, u32);
/// A coprocessor register as (opc1, CRn, CRm, opc2).
type Reg = (u32, u32, u32, u32);

/// A bus that answers a fetch with its address and logs the fetches.
pub(super) struct Rec {
    pub(super) fetches: Vec<Fetch>,
}

impl Rec {
    fn new() -> Self {
        Self {
            fetches: Vec::new(),
        }
    }
}

impl Bus for Rec {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.fetches.push((32, addr));
        addr
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.fetches.push((16, addr));
        addr as u16
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

/// `MCR p15, opc1, Rd, CRn, CRm, opc2` (`read` false) or `MRC` (`read` true), condition AL.
fn transfer(read: bool, opc1: u32, crn: u32, rd: u32, opc2: u32, crm: u32) -> u32 {
    0xEE00_0F10 | u32::from(read) << 20 | opc1 << 21 | crn << 16 | rd << 12 | opc2 << 5 | crm
}

const fn reg(crn: u32, crm: u32, opc2: u32) -> Reg {
    (0, crn, crm, opc2)
}

fn mcr_word(r: Reg, rd: u32) -> u32 {
    transfer(false, r.0, r.1, rd, r.3, r.2)
}

fn mrc_word(r: Reg, rd: u32) -> u32 {
    transfer(true, r.0, r.1, rd, r.3, r.2)
}

/// An ARM9 in Supervisor mode.
pub(super) fn arm9() -> Cpu<true> {
    let mut cpu = Cpu::<true>::new();
    cpu.regs_mut().set_cpsr(SVC);
    cpu
}

/// Steps `opcode` as the instruction at `ADDR`.
fn exec<const A: bool>(cpu: &mut Cpu<A>, opcode: u32) -> Rec {
    cpu.regs_mut().set(15, ARM_PC);
    cpu.set_pipeline([opcode, NEXT]);
    let mut bus = Rec::new();
    let _ = cpu.step(&mut bus);
    bus
}

/// MCR of `value` (through R0) to `r`.
fn write_to(cpu: &mut Cpu<true>, r: Reg, value: u32) {
    cpu.regs_mut().set(0, value);
    exec(cpu, mcr_word(r, 0));
}

/// MRC of `r` into R1, which starts as `SENTINEL`.
fn read_from(cpu: &mut Cpu<true>, r: Reg) -> u32 {
    cpu.regs_mut().set(1, SENTINEL);
    exec(cpu, mrc_word(r, 1));
    cpu.regs().get(1)
}

/// MCR of `value` to the control register (C1,C0,0).
pub(super) fn write_control(cpu: &mut Cpu<true>, value: u32) {
    write_to(cpu, reg(1, 0, 0), value);
}

/// An ARM9 whose exception vectors sit at `FFFF0000h`, set by an MCR of control bit 13.
pub(super) fn arm9_with_high_vectors() -> Cpu<true> {
    let mut cpu = arm9();
    write_control(&mut cpu, CONTROL_VECTORS);
    cpu
}

// ---------------------------------------------------------------- C1 control register

/// R15, the pipeline and the fetches after `opcode` runs once the control register took
/// `controls` in order.
fn entry_after(controls: &[u32], opcode: u32) -> (u32, [u32; 2], Vec<Fetch>) {
    let mut cpu = arm9();
    controls.iter().for_each(|&c| write_control(&mut cpu, c));
    let bus = exec(&mut cpu, opcode);
    (cpu.regs().get(15), cpu.pipeline(), bus.fetches)
}

/// Every entry that does not enter at `base` + its vector, for SWI (8), BKPT (prefetch abort,
/// 0Ch) and an undefined word (4).
fn vector_problems(controls: &[u32], base: u32) -> Vec<String> {
    [(SWI, 0x08), (BKPT, 0x0C), (UNDEFINED, 0x04)]
        .iter()
        .filter(|&&(op, offset)| {
            let vector = base + offset;
            entry_after(controls, op)
                != (
                    vector + 8,
                    [vector, vector + 4],
                    vec![(32, ARM_PC), (32, vector), (32, vector + 4)],
                )
        })
        .map(|&(op, offset)| format!("{op:#010x}: want vector {:#x}", base + offset))
        .collect()
}

#[test]
fn given_control_bit_13_written_when_an_arm9_takes_swi_bkpt_and_undefined_then_they_enter_at_ffff_0008_000c_0004()
 {
    assert_eq!(
        vector_problems(&[CONTROL_VECTORS], HIGH_BASE),
        Vec::<String>::new()
    );
}

#[test]
fn given_control_bit_13_set_then_cleared_when_an_arm9_takes_swi_bkpt_and_undefined_then_they_enter_at_0_based_vectors()
 {
    assert_eq!(
        vector_problems(&[CONTROL_VECTORS, 0], 0),
        Vec::<String>::new()
    );
}

#[test]
fn given_every_control_bit_but_13_written_when_an_arm9_takes_swi_bkpt_and_undefined_then_they_enter_at_0_based_vectors()
 {
    assert_eq!(
        vector_problems(&[!CONTROL_VECTORS], 0),
        Vec::<String>::new()
    );
}

#[test]
fn given_power_on_state_when_an_arm9_reads_the_vector_base_then_it_is_0() {
    assert_eq!(arm9().vector_base(), 0);
}

#[test]
fn given_control_bit_13_written_when_an_arm9_reads_the_vector_base_then_it_is_ffff_0000() {
    assert_eq!(arm9_with_high_vectors().vector_base(), HIGH_BASE);
}

/// (written, read back): the note's `(written & 000F_F085h) | 0000_0078h`, spelled out.
const CONTROL_CASES: [(u32, u32); 14] = [
    (0x0000_0000, 0x0000_0078),
    (0xFFFF_FFFF, 0x000F_F0FD),
    (0x0000_0001, 0x0000_0079),
    (0x0000_0002, 0x0000_0078),
    (0x0000_0004, 0x0000_007C),
    (0x0000_0080, 0x0000_00F8),
    (0x0000_0100, 0x0000_0078),
    (0x0000_0800, 0x0000_0078),
    (0x0000_1000, 0x0000_1078),
    (0x0000_2000, 0x0000_2078),
    (0x0000_8000, 0x0000_8078),
    (0x0001_0000, 0x0001_0078),
    (0x0010_0000, 0x0000_0078),
    (0x8000_0000, 0x0000_0078),
];

fn control_problems() -> Vec<String> {
    CONTROL_CASES
        .iter()
        .filter(|&&(written, expected)| {
            let mut cpu = arm9();
            write_control(&mut cpu, written);
            read_from(&mut cpu, reg(1, 0, 0)) != expected
        })
        .map(|&(written, expected)| format!("wrote {written:#x}, want {expected:#x}"))
        .collect()
}

#[test]
fn given_a_table_of_values_when_each_is_written_to_the_control_register_then_mrc_reads_the_writable_bits_with_bits_3_to_6_set()
 {
    assert_eq!(control_problems(), Vec::<String>::new());
}

#[test]
fn given_power_on_state_when_mrc_reads_the_control_register_then_it_is_78() {
    assert_eq!(read_from(&mut arm9(), reg(1, 0, 0)), 0x78);
}

#[test]
fn given_a_control_register_with_bits_set_when_zero_is_written_then_only_the_fixed_bits_remain() {
    let mut cpu = arm9();
    write_control(&mut cpu, 0xFFFF_FFFF);
    write_control(&mut cpu, 0);
    assert_eq!(read_from(&mut cpu, reg(1, 0, 0)), 0x78);
}

// ---------------------------------------------------------------- C9 TCM and lockdown

/// The (DTCM, ITCM) views after `writes` of (CRn, CRm, opc2, value) reach a fresh `Cp15`.
fn tcm_after(writes: &[(u32, u32, u32, u32)]) -> (Tcm, Tcm) {
    let mut cp15 = Cp15::new();
    writes
        .iter()
        .for_each(|&(crn, crm, opc2, value)| cp15.write(crn, crm, opc2, value));
    (cp15.dtcm(), cp15.itcm())
}

const fn tcm(base: u32, size: u64, enabled: bool, load_mode: bool) -> Tcm {
    Tcm {
        base,
        size,
        enabled,
        load_mode,
    }
}

#[test]
fn given_a_dtcm_register_value_when_the_view_is_read_then_base_and_size_are_resolved() {
    // Base 00800000h, N = 5: 512 shl 5 = 16 KiB.
    let (dtcm, _) = tcm_after(&[(9, 1, 0, 0x0080_000A)]);
    assert_eq!(dtcm, tcm(0x0080_0000, 16 * 1024, false, false));
}

#[test]
fn given_an_itcm_register_value_with_a_base_when_the_view_is_read_then_the_base_is_0_and_the_size_is_kept()
 {
    // Base 03000000h is ignored ("always 00000000h"); N = 6: 512 shl 6 = 32 KiB.
    let (_, itcm) = tcm_after(&[(9, 1, 1, 0x0300_000C)]);
    assert_eq!(itcm, tcm(0, 32 * 1024, false, false));
}

#[test]
fn given_an_itcm_size_smaller_than_the_physical_one_when_the_view_is_read_then_the_new_size_is_reported()
 {
    // N = 3: 4 KiB.
    let (_, itcm) = tcm_after(&[(9, 1, 1, 0x0000_0006)]);
    assert_eq!(itcm, tcm(0, 4 * 1024, false, false));
}

#[test]
fn given_a_dtcm_and_an_itcm_value_when_the_views_are_read_then_each_keeps_its_own_region() {
    let views = tcm_after(&[(9, 1, 0, 0x0080_000A), (9, 1, 1, 0x0000_000C)]);
    assert_eq!(
        views,
        (
            tcm(0x0080_0000, 16 * 1024, false, false),
            tcm(0, 32 * 1024, false, false)
        )
    );
}

#[test]
fn given_a_dtcm_value_with_reserved_bits_when_the_view_is_read_then_they_do_not_count() {
    // Bit 0 and bits 6-11 are reserved; N = 9 from bits 1-5: 512 shl 9 = 256 KiB.
    let (dtcm, _) = tcm_after(&[(9, 1, 0, 0x0040_0FD3)]);
    assert_eq!(dtcm, tcm(0x0040_0000, 256 * 1024, false, false));
}

#[test]
fn given_a_dtcm_value_with_reserved_bits_when_mrc_reads_it_back_then_they_read_as_zero() {
    let mut cpu = arm9();
    write_to(&mut cpu, reg(9, 1, 0), 0x0040_0FD3);
    assert_eq!(read_from(&mut cpu, reg(9, 1, 0)), 0x0040_0012);
}

#[test]
fn given_all_ones_written_to_both_tcm_registers_when_the_views_are_read_then_n_is_not_clamped() {
    // N = 31 is outside 3-23 and is reported as written: 512 shl 31 = 1 shl 40. The ITCM base
    // still reads 0.
    let views = tcm_after(&[(9, 1, 0, 0xFFFF_FFFF), (9, 1, 1, 0xFFFF_FFFF)]);
    assert_eq!(
        views,
        (
            tcm(0xFFFF_F000, 1 << 40, false, false),
            tcm(0, 1 << 40, false, false)
        )
    );
}

/// (control value, [DTCM enabled, DTCM load mode, ITCM enabled, ITCM load mode]).
const TCM_CONTROL_CASES: [(u32, [bool; 4]); 7] = [
    (0x0000_0000, [false, false, false, false]),
    (0x0001_0000, [true, false, false, false]),
    (0x0002_0000, [false, true, false, false]),
    (0x0004_0000, [false, false, true, false]),
    (0x0008_0000, [false, false, false, true]),
    (0x000F_0000, [true, true, true, true]),
    (0xFFF0_FFFF, [false, false, false, false]),
];

fn tcm_control_problems() -> Vec<String> {
    TCM_CONTROL_CASES
        .iter()
        .filter(|&&(control, want)| {
            let (d, i) = tcm_after(&[(1, 0, 0, control)]);
            [d.enabled, d.load_mode, i.enabled, i.load_mode] != want
        })
        .map(|&(control, _)| format!("control {control:#x}"))
        .collect()
}

#[test]
fn given_control_bits_16_to_19_when_the_views_are_read_then_each_bit_sets_its_own_enable_or_load_flag()
 {
    assert_eq!(tcm_control_problems(), Vec::<String>::new());
}

#[test]
fn given_power_on_state_when_the_views_are_read_then_both_are_based_at_0_and_off() {
    let (d, i) = tcm_after(&[]);
    assert_eq!(
        (
            d.base,
            d.enabled,
            d.load_mode,
            i.base,
            i.enabled,
            i.load_mode
        ),
        (0, false, false, 0, false, false)
    );
}

#[test]
fn given_tcm_registers_and_enables_written_by_mcr_when_the_cpu_views_are_read_then_they_resolve() {
    let mut cpu = arm9();
    write_to(&mut cpu, reg(9, 1, 0), 0x0080_000A);
    write_to(&mut cpu, reg(9, 1, 1), 0x0000_000C);
    write_control(&mut cpu, 0x0005_0000);
    assert_eq!(
        (cpu.cp15().dtcm(), cpu.cp15().itcm()),
        (
            tcm(0x0080_0000, 16 * 1024, true, false),
            tcm(0, 32 * 1024, true, false)
        )
    );
}

#[test]
fn given_the_lockdown_registers_when_32_bit_values_are_written_then_each_reads_back_whole() {
    let mut cpu = arm9();
    write_to(&mut cpu, reg(9, 0, 0), 0xFFFF_FFFF);
    write_to(&mut cpu, reg(9, 0, 1), 0x1234_5678);
    assert_eq!(
        (
            read_from(&mut cpu, reg(9, 0, 0)),
            read_from(&mut cpu, reg(9, 0, 1))
        ),
        (0xFFFF_FFFF, 0x1234_5678)
    );
}

// ---------------------------------------------------------------- protection unit

/// (register, written, read back) with the masks of the note's "C2, C3, C5, C6" table.
fn mpu_mask_cases() -> Vec<(Reg, u32, u32)> {
    let mut cases = vec![
        (reg(2, 0, 0), 0xFFFF_FFFF, 0x0000_00FF),
        (reg(2, 0, 1), 0xFFFF_FFFF, 0x0000_00FF),
        (reg(2, 0, 0), 0x0000_0A5A, 0x0000_005A),
        (reg(3, 0, 0), 0xFFFF_FFFF, 0x0000_00FF),
        (reg(5, 0, 0), 0xFFFF_FFFF, 0x0000_FFFF),
        (reg(5, 0, 1), 0xFFFF_FFFF, 0x0000_FFFF),
        (reg(5, 0, 0), 0x1234_5678, 0x0000_5678),
        (reg(5, 0, 2), 0xFFFF_FFFF, 0xFFFF_FFFF),
        (reg(5, 0, 3), 0xFFFF_FFFF, 0xFFFF_FFFF),
    ];
    // C6: bit 0, bits 1-5 and bits 12-31 are held; 6-11 are reserved.
    cases.extend((0..8).flat_map(|region| {
        [0, 1]
            .into_iter()
            .map(move |opc2| (reg(6, region, opc2), 0xFFFF_FFFF, 0xFFFF_F03F))
    }));
    cases
}

fn mpu_mask_problems() -> Vec<String> {
    mpu_mask_cases()
        .iter()
        .filter(|&&(r, written, expected)| {
            let mut cpu = arm9();
            write_to(&mut cpu, r, written);
            read_from(&mut cpu, r) != expected
        })
        .map(|&(r, written, expected)| format!("{r:?}: wrote {written:#x}, want {expected:#x}"))
        .collect()
}

#[test]
fn given_the_protection_registers_when_all_ones_are_written_then_mrc_reads_back_only_the_held_bits()
{
    assert_eq!(mpu_mask_problems(), Vec::<String>::new());
}

/// Writes every (register, value) in order, then reads each (register, expected) and lists the
/// reads that differ.
fn sequence_problems(writes: &[(Reg, u32)], reads: &[(Reg, u32)]) -> Vec<String> {
    let mut cpu = arm9();
    writes.iter().for_each(|&(r, v)| write_to(&mut cpu, r, v));
    reads
        .iter()
        .filter(|&&(r, expected)| read_from(&mut cpu, r) != expected)
        .map(|&(r, expected)| format!("{r:?}: want {expected:#x}"))
        .collect()
}

#[test]
fn given_distinct_values_in_the_data_and_instruction_registers_when_read_back_then_none_shares_storage()
 {
    let cells = [
        (reg(2, 0, 0), 0x11),
        (reg(2, 0, 1), 0x22),
        (reg(3, 0, 0), 0x33),
        (reg(5, 0, 0), 0x1111),
        (reg(5, 0, 1), 0x2222),
        (reg(5, 0, 2), 0x3333_3333),
        (reg(5, 0, 3), 0x4444_4444),
        (reg(9, 0, 0), 0xAAAA_AAAA),
        (reg(9, 0, 1), 0xBBBB_BBBB),
    ];
    assert_eq!(sequence_problems(&cells, &cells), Vec::<String>::new());
}

/// Region `r` gets a value of its own: bit 0 set and a base of `(r + 1) * 0x10000`.
fn region_value(region: u32) -> u32 {
    ((region + 1) * 0x0001_0000) | 1
}

#[test]
fn given_a_value_per_region_written_through_opc2_0_when_read_through_opc2_1_then_it_is_the_same_value()
 {
    let writes: Vec<(Reg, u32)> = (0..8).map(|r| (reg(6, r, 0), region_value(r))).collect();
    let reads: Vec<(Reg, u32)> = (0..8).map(|r| (reg(6, r, 1), region_value(r))).collect();
    assert_eq!(sequence_problems(&writes, &reads), Vec::<String>::new());
}

#[test]
fn given_a_value_per_region_written_through_opc2_1_when_read_through_opc2_0_then_it_is_the_same_value()
 {
    let writes: Vec<(Reg, u32)> = (0..8).map(|r| (reg(6, r, 1), region_value(r))).collect();
    let reads: Vec<(Reg, u32)> = (0..8).map(|r| (reg(6, r, 0), region_value(r))).collect();
    assert_eq!(sequence_problems(&writes, &reads), Vec::<String>::new());
}

#[test]
fn given_c5_opc2_2_written_when_c5_opc2_0_is_read_then_it_is_still_zero() {
    // The two-bit and four-bit permission views are separate storage (note, "Unverified").
    let mut cpu = arm9();
    write_to(&mut cpu, reg(5, 0, 2), 0x1234_5678);
    assert_eq!(read_from(&mut cpu, reg(5, 0, 0)), 0);
}

/// Every protection register written and read; the problems are the instructions after which
/// the core was not simply one step on: an abort would change the mode and move R15.
fn mpu_no_abort_problems() -> Vec<String> {
    let regs: Vec<Reg> = mpu_mask_cases().iter().map(|&(r, ..)| r).collect();
    let mut cpu = arm9();
    regs.iter()
        .flat_map(|&r| {
            write_to(&mut cpu, r, 0xFFFF_FFFF);
            let after_write = (cpu.regs().cpsr(), cpu.regs().get(15), cpu.pipeline()[0]);
            read_from(&mut cpu, r);
            let after_read = (cpu.regs().cpsr(), cpu.regs().get(15), cpu.pipeline()[0]);
            [after_write, after_read]
                .into_iter()
                .filter(|&state| state != (SVC, ARM_PC + 4, NEXT))
                .map(move |state| format!("{r:?}: {state:x?}"))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn given_the_protection_registers_when_each_is_written_and_read_then_no_abort_is_taken() {
    assert_eq!(mpu_no_abort_problems(), Vec::<String>::new());
}

#[test]
fn given_the_protection_registers_written_when_the_vector_base_is_read_then_it_is_unchanged() {
    let mut cpu = arm9();
    write_to(&mut cpu, reg(6, 7, 0), 0xFFFF_FFFF);
    write_to(&mut cpu, reg(5, 0, 2), 0xFFFF_FFFF);
    assert_eq!(cpu.vector_base(), 0);
}

// ---------------------------------------------------------------- C0 ID registers

/// (opc2, value): the note's "C0,C0,0 / 1 / 2" and the reserved 3..7 reading as the Main ID.
const ID_CASES: [(u32, u32); 8] = [
    (0, 0x4105_9461),
    (1, 0x0F0D_2112),
    (2, 0x0014_0180),
    (3, 0x4105_9461),
    (4, 0x4105_9461),
    (5, 0x4105_9461),
    (6, 0x4105_9461),
    (7, 0x4105_9461),
];

fn id_read_problems() -> Vec<String> {
    let mut cpu = arm9();
    ID_CASES
        .iter()
        .filter(|&&(opc2, expected)| read_from(&mut cpu, reg(0, 0, opc2)) != expected)
        .map(|&(opc2, expected)| format!("C0,C0,{opc2}: want {expected:#x}"))
        .collect()
}

#[test]
fn given_power_on_state_when_mrc_reads_c0_c0_opc2_0_to_7_then_the_ids_and_the_main_id_mirror_return()
 {
    assert_eq!(id_read_problems(), Vec::<String>::new());
}

fn id_write_problems() -> Vec<String> {
    let mut cpu = arm9();
    let before = cpu.cp15().clone();
    ID_CASES
        .iter()
        .filter(|&&(opc2, expected)| {
            write_to(&mut cpu, reg(0, 0, opc2), 0xFFFF_FFFF);
            read_from(&mut cpu, reg(0, 0, opc2)) != expected || *cpu.cp15() != before
        })
        .map(|&(opc2, _)| format!("C0,C0,{opc2} changed"))
        .collect()
}

#[test]
fn given_the_id_registers_when_mcr_writes_all_ones_then_they_read_the_same_and_cp15_is_unchanged() {
    assert_eq!(id_write_problems(), Vec::<String>::new());
}

#[test]
fn given_an_arm9_mrc_p15_c0_when_stepped_then_r0_holds_the_main_id_and_the_core_steps_on() {
    // The word is MRC p15, 0, R0, C0, C0, 0 (the same one `coprocessor_words` builds).
    let mut cpu = arm9();
    exec(&mut cpu, 0xEE10_0F10);
    assert_eq!(
        (cpu.regs().get(0), cpu.regs().cpsr(), cpu.regs().get(15)),
        (0x4105_9461, SVC, ARM_PC + 4)
    );
}

// ---------------------------------------------------------------- ARM7 has no CP15

/// What an ARM7 leaves after stepping a CP15 transfer: mode, fetches, R14, the Undefined SPSR,
/// R1, and whether the CP15 state is the power-on one.
fn arm7_after(opcode: u32) -> (u32, Vec<Fetch>, u32, Option<u32>, u32, bool) {
    let mut cpu = Cpu::<false>::new();
    cpu.regs_mut().set_cpsr(SVC);
    cpu.regs_mut().set(0, CONTROL_VECTORS);
    cpu.regs_mut().set(1, SENTINEL);
    let bus = exec(&mut cpu, opcode);
    (
        cpu.regs().cpsr() & 0x1F,
        bus.fetches,
        cpu.regs().get(14),
        cpu.regs().spsr(Bank::Undefined),
        cpu.regs().get(1),
        cpu.cp15 == Cp15::new(),
    )
}

#[test]
fn given_an_arm7_when_it_executes_mcr_p15_to_the_control_register_then_undefined_is_entered_and_cp15_is_untouched()
 {
    assert_eq!(
        arm7_after(mcr_word(reg(1, 0, 0), 0)),
        (
            0x1B,
            vec![(32, ARM_PC), (32, 4), (32, 8)],
            ARM_PC - 4,
            Some(SVC),
            SENTINEL,
            true
        )
    );
}

#[test]
fn given_an_arm7_when_it_executes_mrc_p15_of_the_main_id_then_undefined_is_entered_and_the_register_is_not_written()
 {
    assert_eq!(
        arm7_after(mrc_word(reg(0, 0, 0), 1)),
        (
            0x1B,
            vec![(32, ARM_PC), (32, 4), (32, 8)],
            ARM_PC - 4,
            Some(SVC),
            SENTINEL,
            true
        )
    );
}

#[test]
fn given_an_arm7_whose_cp15_holds_bit_13_when_the_vector_base_is_read_then_it_is_still_0() {
    let mut cpu = Cpu::<false>::new();
    cpu.cp15.write(1, 0, 0, CONTROL_VECTORS);
    assert_eq!(cpu.vector_base(), 0);
}

// ---------------------------------------------------------------- Rd = R15

// Unverified: these tests pin GBATEK's general coprocessor rules for R15 (an MCR sends the address
// + 12, an MRC loads the flags). GBATEK's CP15 overview says R15 should not be used with CP15, so
// the behavior is a guess (`docs/hw/cp15.md`, "Unverified").

/// (value held in C13,C0,1, CPSR before, CPSR after an MRC into R15): only bits 31-28 move.
const MRC_PC_CASES: [(u32, u32, u32); 4] = [
    (0xA000_0000, 0x5000_0093, 0xA000_0093),
    (0x0FFF_FFFF, 0xF000_0013, 0x0000_0013),
    (0xF000_0000, 0x0000_0093, 0xF000_0093),
    (0x5FFF_FFFF, 0x0800_0013, 0x5800_0013),
];

/// Everything an MRC with Rd = R15 gets wrong: the flags, the other CPSR bits, R15, R0-R14.
fn mrc_pc_problems() -> Vec<String> {
    MRC_PC_CASES
        .iter()
        .filter(|&&(value, before, after)| {
            let mut cpu = arm9();
            write_to(&mut cpu, reg(13, 0, 1), value);
            (0..15).for_each(|i| cpu.regs_mut().set(i, 0x100 + i as u32));
            cpu.regs_mut().set_cpsr(before);
            let regs_before: Vec<u32> = (0..15).map(|i| cpu.regs().get(i)).collect();
            exec(&mut cpu, mrc_word(reg(13, 0, 1), 15));
            let regs_after: Vec<u32> = (0..15).map(|i| cpu.regs().get(i)).collect();
            (cpu.regs().cpsr(), cpu.regs().get(15), regs_after) != (after, ARM_PC + 4, regs_before)
        })
        .map(|&(value, before, _)| format!("value {value:#x}, cpsr {before:#x}"))
        .collect()
}

#[test]
fn given_an_mrc_into_r15_when_stepped_then_only_cpsr_bits_31_to_28_take_the_top_bits_and_no_register_changes()
 {
    assert_eq!(mrc_pc_problems(), Vec::<String>::new());
}

#[test]
fn given_an_mcr_from_r15_when_stepped_then_the_register_holds_the_instruction_address_plus_12() {
    let mut cpu = arm9();
    exec(&mut cpu, mcr_word(reg(13, 0, 1), 15));
    assert_eq!(read_from(&mut cpu, reg(13, 0, 1)), ADDR + 12);
}

#[test]
fn given_an_mcr_from_r15_when_stepped_then_r15_is_not_written() {
    let mut cpu = arm9();
    exec(&mut cpu, mcr_word(reg(13, 0, 1), 15));
    assert_eq!(cpu.regs().get(15), ARM_PC + 4);
}

#[test]
fn given_an_mcr_from_r15_to_a_lockdown_register_when_read_back_then_it_holds_the_address_plus_12() {
    let mut cpu = arm9();
    exec(&mut cpu, mcr_word(reg(9, 0, 0), 15));
    assert_eq!(read_from(&mut cpu, reg(9, 0, 0)), ADDR + 12);
}

// ---------------------------------------------------------------- unmapped registers

/// A core whose every held register has a non-zero value, so an unmapped access that aliased
/// one would show.
fn preloaded() -> Cpu<true> {
    let mut cpu = arm9();
    let held = [
        reg(2, 0, 0),
        reg(2, 0, 1),
        reg(3, 0, 0),
        reg(5, 0, 0),
        reg(5, 0, 1),
        reg(5, 0, 2),
        reg(5, 0, 3),
        reg(6, 0, 0),
        reg(6, 7, 0),
        reg(9, 0, 0),
        reg(9, 0, 1),
        reg(9, 1, 0),
        reg(9, 1, 1),
        reg(13, 0, 1),
    ];
    held.iter()
        .for_each(|&r| write_to(&mut cpu, r, 0x5555_5555));
    write_control(&mut cpu, 0x000A_0004);
    cpu
}

/// Combinations the note's table does not list (or lists with another opc1), the BIST registers
/// (not modeled), and three C7 commands the ARM9 does not list, which the note's C7 row accepts
/// like the others and which behave like an unmapped register.
const UNMAPPED: [Reg; 38] = [
    // The BIST registers (opc1 0, 1 and 2) and three C7 commands the ARM9 does not list.
    (0, 15, 0, 1),
    (1, 15, 0, 1),
    (2, 15, 0, 1),
    (0, 7, 7, 0),
    (0, 7, 10, 4),
    (0, 7, 15, 2),
    (1, 0, 0, 0),
    (1, 0, 0, 1),
    (1, 1, 0, 0),
    (1, 2, 0, 0),
    (1, 6, 0, 0),
    (1, 9, 1, 0),
    (1, 13, 0, 1),
    (1, 15, 0, 0),
    (0, 0, 1, 0),
    (0, 0, 2, 0),
    (0, 1, 0, 1),
    (0, 1, 1, 0),
    (0, 2, 0, 2),
    (0, 3, 0, 1),
    (0, 3, 1, 0),
    (0, 4, 0, 0),
    (0, 5, 0, 4),
    (0, 6, 8, 0),
    (0, 6, 15, 1),
    (0, 8, 0, 0),
    (0, 9, 0, 2),
    (0, 9, 1, 2),
    (0, 9, 2, 0),
    (0, 10, 0, 0),
    (0, 11, 0, 0),
    (0, 12, 0, 0),
    (0, 13, 0, 2),
    (0, 13, 1, 0),
    (0, 13, 2, 1),
    (0, 14, 0, 0),
    (0, 15, 0, 0),
    (0, 15, 1, 0),
];

/// MRC of `r` with an explicit opc1 into R1.
fn read_any(cpu: &mut Cpu<true>, r: Reg) -> u32 {
    cpu.regs_mut().set(1, SENTINEL);
    exec(cpu, mrc_word(r, 1));
    cpu.regs().get(1)
}

fn unmapped_problems() -> Vec<String> {
    UNMAPPED
        .iter()
        .filter(|&&r| {
            let mut cpu = preloaded();
            let before = cpu.cp15().clone();
            let read = read_any(&mut cpu, r);
            cpu.regs_mut().set(0, 0xFFFF_FFFF);
            exec(&mut cpu, mcr_word(r, 0));
            read != 0 || *cpu.cp15() != before || cpu.regs().cpsr() != SVC
        })
        .map(|&r| format!("{r:?} did not read 0 or changed state"))
        .collect()
}

#[test]
fn given_registers_the_note_does_not_map_when_read_and_written_then_they_read_0_and_cp15_is_unchanged()
 {
    assert_eq!(unmapped_problems(), Vec::<String>::new());
}

#[test]
fn given_an_unmapped_register_when_written_then_no_exception_is_taken() {
    let mut cpu = preloaded();
    cpu.regs_mut().set(0, 1);
    exec(&mut cpu, mcr_word((1, 15, 0, 0), 0));
    assert_eq!(
        (cpu.regs().cpsr(), cpu.regs().get(15), cpu.pipeline()[0]),
        (SVC, ARM_PC + 4, NEXT)
    );
}

// ---------------------------------------------------------------- C7 cache commands and halt

/// (CRm, opc2) of the ARM9 cache commands and both wait-for-interrupt forms.
const CACHE_COMMANDS: [(u32, u32); 11] = [
    (0, 4),
    (8, 2),
    (5, 0),
    (5, 1),
    (6, 0),
    (6, 1),
    (10, 1),
    (10, 2),
    (13, 1),
    (14, 1),
    (14, 2),
];

/// Commands after which the CP15 state changed, the core did not step on, or a following
/// instruction did not run.
fn cache_command_problems() -> Vec<String> {
    CACHE_COMMANDS
        .iter()
        .filter(|&&(crm, opc2)| {
            let mut cpu = preloaded();
            let before = cpu.cp15().clone();
            write_to(&mut cpu, reg(7, crm, opc2), 0xFFFF_FFFF);
            let state = (cpu.regs().cpsr(), cpu.regs().get(15), cpu.pipeline()[0]);
            exec(&mut cpu, MOV_R2_5);
            *cpu.cp15() != before || state != (SVC, ARM_PC + 4, NEXT) || cpu.regs().get(2) != 5
        })
        .map(|&(crm, opc2)| format!("C7,C{crm},{opc2}"))
        .collect()
}

#[test]
fn given_cache_commands_and_both_wait_for_interrupt_forms_when_mcr_writes_them_then_cp15_is_unchanged_and_the_core_runs_on()
 {
    // The halt itself is the machine's: `Cpu::step` does not look at it (`hle-swi.md`).
    assert_eq!(cache_command_problems(), Vec::<String>::new());
}

/// The commands after which the core is halted: only the two wait-for-interrupt forms may be.
fn halted_commands() -> Vec<(u32, u32)> {
    CACHE_COMMANDS
        .iter()
        .copied()
        .filter(|&(crm, opc2)| {
            let mut cpu = preloaded();
            write_to(&mut cpu, reg(7, crm, opc2), 0);
            cpu.halted()
        })
        .collect()
}

#[test]
fn given_cache_commands_and_both_wait_for_interrupt_forms_when_mcr_writes_them_then_only_the_wait_forms_halt_the_core()
 {
    assert_eq!(halted_commands(), vec![(0, 4), (8, 2)]);
}

#[test]
fn given_a_wait_for_interrupt_when_mcr_writes_it_then_the_operand_register_is_ignored() {
    let mut cpu = preloaded();
    write_to(&mut cpu, reg(7, 0, 4), 0xDEAD_BEEF);
    assert!(cpu.halted());
}

fn cache_read_problems() -> Vec<String> {
    CACHE_COMMANDS
        .iter()
        .filter(|&&(crm, opc2)| {
            let mut cpu = preloaded();
            read_from(&mut cpu, reg(7, crm, opc2)) != 0
        })
        .map(|&(crm, opc2)| format!("C7,C{crm},{opc2}"))
        .collect()
}

#[test]
fn given_the_write_only_c7_when_mrc_reads_a_command_register_then_it_reads_0() {
    assert_eq!(cache_read_problems(), Vec::<String>::new());
}

// ---------------------------------------------------------------- C13 process ID

#[test]
fn given_the_fcse_process_id_when_all_ones_are_written_then_it_reads_0() {
    let mut cpu = arm9();
    write_to(&mut cpu, reg(13, 0, 0), 0xFFFF_FFFF);
    assert_eq!(read_from(&mut cpu, reg(13, 0, 0)), 0);
}

#[test]
fn given_the_fcse_process_id_when_written_then_the_trace_process_id_is_unchanged() {
    let mut cpu = arm9();
    write_to(&mut cpu, reg(13, 0, 1), 0x1111_2222);
    write_to(&mut cpu, reg(13, 0, 0), 0xFFFF_FFFF);
    assert_eq!(read_from(&mut cpu, reg(13, 0, 1)), 0x1111_2222);
}

#[test]
fn given_the_trace_process_id_written_at_c13_c0_1_when_read_at_c13_c1_1_then_it_is_the_same_value()
{
    let mut cpu = arm9();
    write_to(&mut cpu, reg(13, 0, 1), 0xDEAD_BEEF);
    assert_eq!(read_from(&mut cpu, reg(13, 1, 1)), 0xDEAD_BEEF);
}

#[test]
fn given_the_trace_process_id_written_at_c13_c1_1_when_read_at_c13_c0_1_then_it_is_the_same_value()
{
    let mut cpu = arm9();
    write_to(&mut cpu, reg(13, 1, 1), 0x0BAD_F00D);
    assert_eq!(read_from(&mut cpu, reg(13, 0, 1)), 0x0BAD_F00D);
}

// ---------------------------------------------------------------- state is plain data

/// Encodes with postcard and decodes again.
fn postcard_round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    let bytes = postcard::to_allocvec(value).expect("state is plain data and encodes");
    postcard::from_bytes(&bytes).expect("bytes just encoded decode")
}

#[test]
fn given_a_cp15_holding_non_default_values_when_compared_then_it_differs_from_power_on() {
    assert_ne!(*preloaded().cp15(), Cp15::new());
}

#[test]
fn given_a_cp15_holding_non_default_values_when_postcard_round_tripped_then_it_is_equal() {
    let cp15 = preloaded().cp15().clone();
    assert_eq!(postcard_round_trip(&cp15), cp15);
}

#[test]
fn given_an_arm9_core_holding_non_default_values_when_postcard_round_tripped_then_it_is_equal() {
    let cpu = preloaded();
    assert_eq!(postcard_round_trip(&cpu), cpu);
}

#[test]
fn given_an_arm7_core_when_postcard_round_tripped_then_it_is_equal() {
    let cpu = Cpu::<false>::new();
    assert_eq!(postcard_round_trip(&cpu), cpu);
}
