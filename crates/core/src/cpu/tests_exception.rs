// SPDX-License-Identifier: Apache-2.0
//! Exception entry through `Cpu::step` and the entry API. Expected values come from
//! `docs/hw/arm-exceptions-modes.md`, never from running the implementation: an entry puts the
//! return address in the entered mode's R14, the old CPSR in its SPSR, switches mode, clears T,
//! sets I (and F for Reset and FIQ), and refills the pipeline from the vector.
//!
//! The recording bus answers a fetch with the address it was asked for, so the pipeline contents
//! show which address each entry came from.

use super::exception::Exception;
use super::*;
use proptest::prelude::*;

/// Address of the instruction under test.
const ADDR: u32 = 0x1000;
/// R15 while an ARM instruction executes (address + 8), and between two instructions when the
/// next one is at `ADDR`.
const ARM_PC: u32 = ADDR + 8;
/// R15 while a Thumb instruction executes (address + 4).
const THUMB_PC: u32 = ADDR + 4;
/// The opcode already sitting in `pipeline[1]` before the step.
const NEXT: u32 = 0xAAAA_0001;
const T_BIT: u32 = 0x20;
const I_BIT: u32 = 0x80;
const F_BIT: u32 = 0x40;
/// N, Z, C, V all set.
const FLAGS: u32 = 0xF000_0000;
const HIGH_BASE: u32 = 0xFFFF_0000;

const SWI: u32 = 0xEF00_0123;
/// A word in the undefined space (bits 27–25 = 011, bit 4 = 1).
const UNDEFINED: u32 = 0xE600_0010;
/// `SUBS PC, R14, #4`, the IRQ return.
const SUBS_PC_LR_4: u32 = 0xE25E_F004;
/// `MOVS PC, R14`, the SWI return.
const MOVS_PC_LR: u32 = 0xE1B0_F00E;
/// `LDMIA R13!, {PC}^`.
const LDM_PC_CARET: u32 = 0xE8FD_8000;

/// Every mode, with the bank its SPSR lives in (User and System have none to check).
const MODES: [Mode; 7] = [
    Mode::User,
    Mode::Fiq,
    Mode::Irq,
    Mode::Supervisor,
    Mode::Abort,
    Mode::Undefined,
    Mode::System,
];

/// One code fetch: width in bits and address.
type Fetch = (u8, u32);

struct RecBus {
    fetches: Vec<Fetch>,
    data: u32,
}

impl RecBus {
    fn new() -> Self {
        Self {
            fetches: Vec::new(),
            data: 0,
        }
    }
}

impl Bus for RecBus {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.fetches.push((32, addr));
        addr
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.fetches.push((16, addr));
        addr as u16
    }
    fn read8(&mut self, _addr: u32) -> u8 {
        self.data as u8
    }
    fn read16(&mut self, _addr: u32) -> u16 {
        self.data as u16
    }
    fn read32(&mut self, _addr: u32) -> u32 {
        self.data
    }
    fn write8(&mut self, _addr: u32, _value: u8) {}
    fn write16(&mut self, _addr: u32, _value: u16) {}
    fn write32(&mut self, _addr: u32, _value: u32) {}
}

/// What makes an exception happen in a test.
#[derive(Clone, Copy)]
enum Trigger {
    /// Step an instruction word.
    Step(u32),
    /// The entry API, with R15 as it reads while the instruction executes.
    Irq,
    Fiq,
    Prefetch,
    Data,
    Reset,
}

/// What an ARM-state entry must leave.
struct Expect {
    vector: u32,
    mode: Mode,
    /// F is set by the entry (Reset and FIQ).
    sets_f: bool,
    /// R14 of the entered mode, `None` when the entry leaves it alone (Reset).
    lr: Option<u32>,
}

const fn expect(vector: u32, mode: Mode, sets_f: bool, lr: Option<u32>) -> Expect {
    Expect {
        vector,
        mode,
        sets_f,
        lr,
    }
}

/// The ARM-state cases of the note's "Vector table" and "Return address" tables.
fn arm_expectation(trigger: Trigger) -> Expect {
    match trigger {
        Trigger::Step(op) if op == SWI => expect(0x08, Mode::Supervisor, false, Some(ARM_PC - 4)),
        Trigger::Step(_) => expect(0x04, Mode::Undefined, false, Some(ARM_PC - 4)),
        Trigger::Irq => expect(0x18, Mode::Irq, false, Some(ARM_PC - 4)),
        Trigger::Fiq => expect(0x1C, Mode::Fiq, true, Some(ARM_PC - 4)),
        Trigger::Prefetch => expect(0x0C, Mode::Abort, false, Some(ARM_PC - 4)),
        Trigger::Data => expect(0x10, Mode::Abort, false, Some(ARM_PC)),
        Trigger::Reset => expect(0x00, Mode::Supervisor, true, None),
    }
}

fn mode_bits(mode: Mode) -> u32 {
    match mode {
        Mode::User => 0x10,
        Mode::Fiq => 0x11,
        Mode::Irq => 0x12,
        Mode::Supervisor => 0x13,
        Mode::Abort => 0x17,
        Mode::Undefined => 0x1B,
        Mode::System => 0x1F,
    }
}

/// A CPU in `cpsr`, with R15 and the pipeline as they are while `opcode` executes.
fn cpu_in(cpsr: u32, pc: u32, opcode: u32) -> Cpu<false> {
    let mut cpu = Cpu::<false>::new();
    cpu.regs_mut().set_cpsr(cpsr);
    cpu.regs_mut().set(15, pc);
    cpu.set_pipeline([opcode, NEXT]);
    cpu
}

fn fire<const ARM9: bool>(cpu: &mut Cpu<ARM9>, bus: &mut RecBus, trigger: Trigger) {
    match trigger {
        Trigger::Step(_) => cpu.step(bus),
        Trigger::Irq => assert!(cpu.take_interrupt(bus, true, false)),
        Trigger::Fiq => assert!(cpu.take_interrupt(bus, false, true)),
        Trigger::Prefetch => cpu.prefetch_abort(bus),
        Trigger::Data => cpu.data_abort(bus),
        Trigger::Reset => cpu.reset(bus),
    }
}

fn opcode_of(trigger: Trigger) -> u32 {
    match trigger {
        Trigger::Step(op) => op,
        _ => 0xE1A0_0000,
    }
}

/// Everything wrong with the entry of `trigger` from `cpsr`, in ARM state.
fn entry_problems(trigger: Trigger, cpsr: u32) -> Vec<String> {
    let want = arm_expectation(trigger);
    let mut cpu = cpu_in(cpsr, ARM_PC, opcode_of(trigger));
    // Marks R14 of every bank so a wrong-bank write shows.
    MODES.iter().for_each(|m| {
        cpu.regs_mut()
            .set_banked(m.bank(), 14, 0xB000 + mode_bits(*m));
    });
    let old_lr = cpu.regs().banked(want.mode.bank(), 14);
    let old_spsr = cpu.regs().spsr(want.mode.bank());
    let mut bus = RecBus::new();
    fire(&mut cpu, &mut bus, trigger);
    let new_cpsr = (cpsr & !0xFF) | I_BIT | (cpsr & F_BIT) | mode_bits(want.mode);
    let new_cpsr = new_cpsr | (u32::from(want.sets_f) * F_BIT);
    let checks = [
        ("cpsr", cpu.regs().cpsr() == new_cpsr),
        (
            "lr",
            cpu.regs().banked(want.mode.bank(), 14) == want.lr.unwrap_or(old_lr),
        ),
        (
            "spsr",
            cpu.regs().spsr(want.mode.bank()) == want.lr.map(|_| cpsr).or(old_spsr),
        ),
        ("r15", cpu.regs().get(15) == want.vector + 8),
        ("pipeline", cpu.pipeline() == [want.vector, want.vector + 4]),
    ];
    checks
        .iter()
        .filter(|(_, ok)| !ok)
        .map(|(what, _)| format!("vector {:#x} from {cpsr:#x}: {what}", want.vector))
        .collect()
}

/// Every start mode, with and without the flags, I and F set.
fn start_states() -> Vec<u32> {
    MODES
        .iter()
        .flat_map(|m| {
            [0, FLAGS, I_BIT | F_BIT, FLAGS | I_BIT, FLAGS | F_BIT]
                .iter()
                .map(move |extra| mode_bits(*m) | extra)
        })
        .collect()
}

/// Whether `trigger` can fire from `cpsr`: an interrupt needs its mask bit clear.
fn can_fire(trigger: Trigger, cpsr: u32) -> bool {
    match trigger {
        Trigger::Irq => cpsr & I_BIT == 0,
        Trigger::Fiq => cpsr & F_BIT == 0,
        _ => true,
    }
}

fn all_start_problems(trigger: Trigger) -> Vec<String> {
    start_states()
        .into_iter()
        .filter(|&cpsr| can_fire(trigger, cpsr))
        .flat_map(|cpsr| entry_problems(trigger, cpsr))
        .collect()
}

#[test]
fn given_every_start_mode_when_a_swi_executes_then_supervisor_is_entered_through_vector_8() {
    assert_eq!(all_start_problems(Trigger::Step(SWI)), Vec::<String>::new());
}

#[test]
fn given_every_start_mode_when_an_undefined_word_executes_then_undefined_is_entered_through_vector_4()
 {
    assert_eq!(
        all_start_problems(Trigger::Step(UNDEFINED)),
        Vec::<String>::new()
    );
}

#[test]
fn given_every_start_mode_when_an_irq_is_taken_then_irq_is_entered_through_vector_18() {
    assert_eq!(all_start_problems(Trigger::Irq), Vec::<String>::new());
}

#[test]
fn given_every_start_mode_when_a_fiq_is_taken_then_fiq_is_entered_with_f_set_through_vector_1c() {
    assert_eq!(all_start_problems(Trigger::Fiq), Vec::<String>::new());
}

#[test]
fn given_every_start_mode_when_a_prefetch_abort_is_taken_then_abort_is_entered_through_vector_c() {
    assert_eq!(all_start_problems(Trigger::Prefetch), Vec::<String>::new());
}

#[test]
fn given_every_start_mode_when_a_data_abort_is_taken_then_abort_is_entered_through_vector_10() {
    assert_eq!(all_start_problems(Trigger::Data), Vec::<String>::new());
}

#[test]
fn given_every_start_mode_when_the_core_resets_then_supervisor_is_entered_with_i_and_f_set_through_vector_0()
 {
    assert_eq!(all_start_problems(Trigger::Reset), Vec::<String>::new());
}

// ---------------------------------------------------------------- Thumb state

/// R14 after `exception` is taken in Thumb state with R15 = address + 4 (the note's "Return
/// address" table), and whether T ended clear.
fn thumb_entry(exception: Exception) -> (u32, bool) {
    let mut cpu = cpu_in(0x13 | T_BIT, THUMB_PC, 0);
    cpu.take_exception(exception);
    (cpu.regs().get(14), cpu.regs().cpsr() & T_BIT == 0)
}

fn thumb_lr_problems() -> Vec<String> {
    let cases = [
        (Exception::SoftwareInterrupt, THUMB_PC - 2),
        (Exception::Undefined, THUMB_PC - 2),
        (Exception::PrefetchAbort, THUMB_PC),
        (Exception::DataAbort, THUMB_PC + 4),
        (Exception::Irq, THUMB_PC),
        (Exception::Fiq, THUMB_PC),
    ];
    cases
        .iter()
        .filter(|&&(e, lr)| thumb_entry(e) != (lr, true))
        .map(|&(e, lr)| format!("{e:?}: want lr {lr:#x}, got {:?}", thumb_entry(e)))
        .collect()
}

#[test]
fn given_thumb_state_when_each_exception_is_taken_then_r14_follows_the_thumb_column_and_t_is_cleared()
 {
    assert_eq!(thumb_lr_problems(), Vec::<String>::new());
}

// ------------------------------------------------------------ failed conditions

#[test]
fn given_a_swi_whose_condition_fails_when_stepped_then_it_is_a_plain_step() {
    // SWINE with Z set.
    let mut cpu = cpu_in(0x13 | 0x4000_0000, ARM_PC, 0x1F00_0000);
    let before = cpu.regs().cpsr();
    cpu.step(&mut RecBus::new());
    assert_eq!(
        (cpu.regs().cpsr(), cpu.regs().get(15), cpu.pipeline()),
        (before, ARM_PC + 4, [NEXT, ARM_PC])
    );
}

#[test]
fn given_an_undefined_word_whose_condition_fails_when_stepped_then_it_is_a_plain_step() {
    // The undefined space with condition NE and Z set.
    let mut cpu = cpu_in(0x13 | 0x4000_0000, ARM_PC, 0x1600_0010);
    let before = cpu.regs().cpsr();
    cpu.step(&mut RecBus::new());
    assert_eq!(
        (cpu.regs().cpsr(), cpu.regs().get(15), cpu.pipeline()),
        (before, ARM_PC + 4, [NEXT, ARM_PC])
    );
}

// --------------------------------------------------------- same-mode nesting

#[test]
fn given_a_swi_taken_in_supervisor_mode_when_it_enters_then_r14_and_the_spsr_are_overwritten() {
    let mut cpu = cpu_in(0x13 | FLAGS, ARM_PC, SWI);
    cpu.regs_mut().set(14, 0x1111_1111);
    cpu.regs_mut().set_spsr(Bank::Supervisor, 0x2222_2222);
    cpu.step(&mut RecBus::new());
    assert_eq!(
        (cpu.regs().get(14), cpu.regs().spsr(Bank::Supervisor)),
        (ARM_PC - 4, Some(0x13 | FLAGS))
    );
}

#[test]
fn given_a_fiq_from_user_mode_when_it_enters_then_r8_to_r12_switch_to_the_fiq_copies() {
    let mut cpu = cpu_in(0x10, ARM_PC, 0);
    cpu.regs_mut().set(8, 1);
    cpu.regs_mut().set_banked(Bank::Fiq, 8, 2);
    cpu.take_interrupt(&mut RecBus::new(), false, true);
    assert_eq!(
        (cpu.regs().get(8), cpu.regs().banked(Bank::UserSystem, 8)),
        (2, 1)
    );
}

// --------------------------------------------------------------- interrupts

/// What `take_interrupt` returns and which mode the CPU ends in, for the lines and masks.
fn interrupt_outcome(cpsr: u32, irq: bool, fiq: bool) -> (bool, u32) {
    let mut cpu = cpu_in(cpsr, ARM_PC, 0);
    let taken = cpu.take_interrupt(&mut RecBus::new(), irq, fiq);
    (taken, cpu.regs().cpsr() & 0x1F)
}

fn interrupt_mask_problems() -> Vec<String> {
    // (cpsr control byte, irq line, fiq line, taken, mode afterwards)
    let cases = [
        (0x13, true, false, true, 0x12),
        (0x13 | I_BIT, true, false, false, 0x13),
        (0x13, false, true, true, 0x11),
        (0x13 | F_BIT, false, true, false, 0x13),
        (0x13, true, true, true, 0x11),
        (0x13 | F_BIT, true, true, true, 0x12),
        (0x13 | I_BIT, true, true, true, 0x11),
        (0x13 | I_BIT | F_BIT, true, true, false, 0x13),
        (0x13, false, false, false, 0x13),
    ];
    cases
        .iter()
        .filter(|&&(cpsr, irq, fiq, taken, mode)| {
            interrupt_outcome(cpsr, irq, fiq) != (taken, mode)
        })
        .map(|&(cpsr, irq, fiq, ..)| format!("cpsr {cpsr:#x} irq {irq} fiq {fiq}"))
        .collect()
}

#[test]
fn given_masks_and_pending_lines_when_an_interrupt_is_offered_then_only_unmasked_lines_enter_and_fiq_wins()
 {
    assert_eq!(interrupt_mask_problems(), Vec::<String>::new());
}

#[test]
fn given_i_set_when_an_irq_is_offered_then_nothing_changes() {
    let mut cpu = cpu_in(0x13 | I_BIT, ARM_PC, 0);
    let before = cpu.clone();
    cpu.take_interrupt(&mut RecBus::new(), true, false);
    assert_eq!(cpu, before);
}

// ------------------------------------------------------------- vector base

#[test]
fn given_the_high_vector_flag_when_an_arm9_takes_a_swi_then_it_enters_at_ffff0008() {
    let mut cpu = Cpu::<true>::new();
    cpu.set_high_vectors(true);
    cpu.regs_mut().set(15, ARM_PC);
    cpu.set_pipeline([SWI, NEXT]);
    let mut bus = RecBus::new();
    cpu.step(&mut bus);
    assert_eq!(
        (cpu.regs().get(15), cpu.pipeline(), bus.fetches),
        (
            HIGH_BASE + 8 + 8,
            [HIGH_BASE + 8, HIGH_BASE + 12],
            vec![(32, ARM_PC), (32, HIGH_BASE + 8), (32, HIGH_BASE + 12)]
        )
    );
}

#[test]
fn given_the_high_vector_flag_when_an_arm7_takes_a_swi_then_it_still_enters_at_8() {
    let mut cpu = Cpu::<false>::new();
    cpu.set_high_vectors(true);
    cpu.regs_mut().set(15, ARM_PC);
    cpu.set_pipeline([SWI, NEXT]);
    cpu.step(&mut RecBus::new());
    assert_eq!(cpu.pipeline(), [8, 12]);
}

#[test]
fn given_power_on_state_when_reading_the_vector_base_then_both_cores_use_the_low_vectors() {
    assert_eq!(
        (
            Cpu::<false>::new().vector_base(),
            Cpu::<true>::new().vector_base()
        ),
        (0, 0)
    );
}

#[test]
fn given_the_high_vector_flag_when_reading_the_vector_base_then_only_the_arm9_moves() {
    let mut arm7 = Cpu::<false>::new();
    let mut arm9 = Cpu::<true>::new();
    arm7.set_high_vectors(true);
    arm9.set_high_vectors(true);
    assert_eq!((arm7.vector_base(), arm9.vector_base()), (0, HIGH_BASE));
}

#[test]
fn given_the_high_vector_flag_set_and_cleared_when_an_arm9_reads_the_base_then_it_is_low_again() {
    let mut arm9 = Cpu::<true>::new();
    arm9.set_high_vectors(true);
    arm9.set_high_vectors(false);
    assert_eq!(arm9.vector_base(), 0);
}

// ------------------------------------------------------------- coprocessors

/// CDP, MCR, MRC, LDC and STC words (condition AL) for coprocessor `cp`, all with R0 and
/// offset 0. MCRR is the `1100010x` corner of the LDC/STC space.
fn coprocessor_words(cp: u32) -> [u32; 6] {
    [
        0xEE00_0000 | cp << 8,
        0xEE00_0010 | cp << 8,
        0xEE10_0010 | cp << 8,
        0xED90_0000 | cp << 8,
        0xED80_0000 | cp << 8,
        0xEC40_0000 | cp << 8,
    ]
}

/// Whether stepping `word` on an ARM7 entered Undefined: the mode, the vector fetches, and
/// R14 = address + 4.
fn arm7_traps(word: u32) -> bool {
    let mut cpu = cpu_in(0x13, ARM_PC, word);
    let mut bus = RecBus::new();
    cpu.step(&mut bus);
    cpu.regs().cpsr() & 0x1F == 0x1B
        && bus.fetches == [(32, ARM_PC), (32, 4), (32, 8)]
        && cpu.regs().get(14) == ARM_PC - 4
}

fn arm7_coprocessor_problems() -> Vec<String> {
    (0..16)
        .flat_map(coprocessor_words)
        .filter(|&word| !arm7_traps(word))
        .map(|word| format!("{word:#010x} did not trap"))
        .collect()
}

#[test]
fn given_every_coprocessor_number_and_form_when_an_arm7_executes_it_then_undefined_is_entered() {
    assert_eq!(arm7_coprocessor_problems(), Vec::<String>::new());
}

/// An ARM9 stepping `word`: whether it entered Undefined, and whether it only stepped on.
fn arm9_outcome(word: u32) -> (bool, bool) {
    let mut cpu = Cpu::<true>::new();
    cpu.regs_mut().set_cpsr(0x13);
    cpu.regs_mut().set(15, ARM_PC);
    cpu.set_pipeline([word, NEXT]);
    cpu.step(&mut RecBus::new());
    (
        cpu.regs().cpsr() & 0x1F == 0x1B,
        cpu.regs().cpsr() == 0x13 && cpu.regs().get(15) == ARM_PC + 4,
    )
}

fn arm9_coprocessor_problems() -> Vec<String> {
    // MCR p15 and MRC p15 step on (the CP15 arrives in #15); MCRR/MRRC are ARMv5TE and stay
    // unhandled (#14); everything else traps.
    let cases: Vec<(u32, (bool, bool))> = (0..16)
        .flat_map(|cp| {
            coprocessor_words(cp)
                .into_iter()
                .enumerate()
                .map(move |(form, word)| {
                    let steps_on = cp == 15 && (form == 1 || form == 2) || form == 5;
                    (word, (!steps_on, steps_on))
                })
        })
        .collect();
    cases
        .iter()
        .filter(|&&(word, want)| arm9_outcome(word) != want)
        .map(|&(word, _)| format!("{word:#010x}"))
        .collect()
}

#[test]
fn given_every_coprocessor_number_and_form_when_an_arm9_executes_it_then_only_mcr_mrc_p15_and_mcrr_step_on()
 {
    assert_eq!(arm9_coprocessor_problems(), Vec::<String>::new());
}

// -------------------------------------------------------------- return trips

/// Enters `trigger` from `start`, runs `return_op` at the vector, and returns
/// (R15, CPSR) afterwards. `data` is what the return instruction's data reads give.
fn round_trip(trigger: Trigger, start: u32, return_op: u32, data: u32) -> (u32, u32, [u32; 2]) {
    let mut cpu = cpu_in(start, ARM_PC, opcode_of(trigger));
    let mut bus = RecBus::new();
    fire(&mut cpu, &mut bus, trigger);
    cpu.set_pipeline([return_op, 0]);
    // The LDM form pops the return address from the handler's stack.
    cpu.regs_mut().set(13, 0x4000);
    bus.data = data;
    bus.fetches.clear();
    cpu.step(&mut bus);
    (cpu.regs().get(15), cpu.regs().cpsr(), cpu.pipeline())
}

#[test]
fn given_a_swi_when_the_handler_runs_movs_pc_lr_then_execution_resumes_after_the_swi_with_the_old_cpsr()
 {
    let start = 0x10 | FLAGS;
    assert_eq!(
        round_trip(Trigger::Step(SWI), start, MOVS_PC_LR, 0),
        (ADDR + 4 + 8, start, [ADDR + 4, ADDR + 8])
    );
}

#[test]
fn given_an_irq_when_the_handler_runs_subs_pc_lr_4_then_execution_resumes_at_the_interrupted_instruction()
 {
    let start = 0x1F | FLAGS;
    assert_eq!(
        round_trip(Trigger::Irq, start, SUBS_PC_LR_4, 0),
        (ADDR + 8, start, [ADDR, ADDR + 4])
    );
}

#[test]
fn given_an_irq_when_the_handler_runs_ldm_pc_caret_then_the_popped_address_and_the_old_cpsr_return()
{
    let start = 0x10 | FLAGS;
    assert_eq!(
        round_trip(Trigger::Irq, start, LDM_PC_CARET, 0x2000),
        (0x2000 + 8, start, [0x2000, 0x2004])
    );
}

#[test]
fn given_a_data_abort_when_the_handler_runs_subs_pc_lr_8_then_execution_resumes_at_the_aborted_instruction()
 {
    // SUBS PC, R14, #8
    let start = 0x1F;
    assert_eq!(
        round_trip(Trigger::Data, start, 0xE25E_F008, 0),
        (ADDR + 8, start, [ADDR, ADDR + 4])
    );
}

// ----------------------------------------------------------------- proptest

proptest! {
    #[test]
    fn given_any_flags_and_f_bit_when_a_swi_executes_then_bits_31_to_8_and_f_are_kept(
        top in any::<u32>(),
        f in any::<bool>(),
        mode in 0usize..7,
    ) {
        let cpsr = (top & !0xFF) | mode_bits(MODES[mode]) | (u32::from(f) * F_BIT);
        let mut cpu = cpu_in(cpsr, ARM_PC, SWI);
        cpu.step(&mut RecBus::new());
        prop_assert_eq!(
            cpu.regs().cpsr(),
            (cpsr & !0xFF) | I_BIT | (cpsr & F_BIT) | 0x13
        );
    }
}
