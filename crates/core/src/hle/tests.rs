// SPDX-License-Identifier: Apache-2.0
//! Direct boot through `Machine::load_rom`. Expected values come from
//! `docs/hw/direct-boot.md` (and `cart-header.md`, `arm9-memory-map.md`,
//! `arm-branch-pipeline.md`), never from running the code.
//!
//! Scenarios live in helpers that return what they observed; a test body only asserts.

use crate::Machine;
use crate::cart::test_image::{Image, arm7_byte, arm9_byte, header_byte, words};
use crate::cart::{BinarySpec, HEADER_LEN, HeaderError};
use crate::cpu::Bank;

const RAM: u32 = 0x0200_0000;
const RAM_END: u32 = 0x0240_0000;
const HEADER_COPY: u32 = 0x027F_FE00;
/// `add r0, r0, #1`.
const ADD_R0_1: u32 = 0xE280_0001;
/// `b` to the instruction before it (offset -12 from the branch plus pipeline 8 = -3 words).
const B_BACK_ONE: u32 = 0xEAFF_FFFD;

fn booted(image: &Image) -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(image.build()).expect("valid image");
    machine
}

/// An image whose ARM9 binary is a counting loop at `entry`.
fn loop_image() -> Image {
    let mut image = Image::valid();
    image.arm9_code = words(&[ADD_R0_1, B_BACK_ONE]);
    image
}

/// An image with ARM9 RAM address, entry and offset all different, so mix-ups show.
fn distinct_image() -> Image {
    let mut image = Image::valid();
    image.arm9 = BinarySpec {
        rom_offset: 0x5000,
        entry: 0x0201_0040,
        ram_address: 0x0201_0000,
        size: 0x100,
    };
    image.arm9_code = words(&[0xE1A0_0000, 0xE1A0_1001, 0xE1A0_2002]);
    image
}

/// Byte offsets (in `0..len`) where main RAM at `base` differs from `expected(offset)`.
fn byte_mismatches(
    m: &Machine,
    base: u32,
    len: usize,
    expected: impl Fn(usize) -> u8,
) -> Vec<usize> {
    (0..len)
        .filter(|&i| m.inspect().arm9_peek8(base + i as u32) != expected(i))
        .collect()
}

/// Word-aligned main-RAM addresses outside `allowed` (start, len) ranges that hold nonzero data.
fn stray_nonzero_words(m: &Machine, allowed: &[(u32, u32)]) -> Vec<u32> {
    (RAM..RAM_END)
        .step_by(4)
        .filter(|&a| !allowed.iter().any(|&(s, l)| a >= s && a < s + l))
        .filter(|&a| m.inspect().arm9_peek32(a) != 0)
        .collect()
}

fn nonzero_core_registers(m: &Machine) -> Vec<(usize, u32)> {
    (0..=11)
        .map(|i| (i, m.inspect().arm9().regs().get(i)))
        .filter(|&(_, v)| v != 0)
        .collect()
}

/// Header-copy words that differ from the image's first 0x170 bytes.
fn header_copy_mismatches(m: &Machine, image: &[u8]) -> Vec<usize> {
    (0..HEADER_LEN / 4)
        .filter(|&w| {
            let b = &image[w * 4..w * 4 + 4];
            m.inspect().arm9_peek32(HEADER_COPY + (w * 4) as u32)
                != u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        })
        .collect()
}

// ---- Placement ----

#[test]
fn given_valid_image_when_loading_then_header_is_parsed_and_kept() {
    let image = Image::valid().build();
    let mut machine = Machine::new();
    machine.load_rom(image.clone()).expect("valid");
    assert_eq!(
        machine.header().copied(),
        Some(crate::cart::parse(&image).expect("valid"))
    );
}

#[test]
fn given_valid_image_when_loading_then_the_first_0x170_bytes_are_copied_to_027ffe00() {
    let image = Image::valid().build();
    let machine = booted(&Image::valid());
    assert_eq!(
        header_copy_mismatches(&machine, &image),
        Vec::<usize>::new()
    );
}

#[test]
fn given_valid_image_when_loading_then_header_copy_stops_at_0x170() {
    let machine = booted(&Image::valid());
    // Image bytes 0x170.. are 0xEE reserved filler; they must not be copied.
    assert_eq!(
        machine
            .inspect()
            .arm9_peek32(HEADER_COPY + HEADER_LEN as u32),
        0
    );
}

#[test]
fn given_valid_image_when_loading_then_header_copy_is_also_visible_at_the_main_ram_mirror() {
    let machine = booted(&Image::valid());
    let expected = u32::from_le_bytes([
        header_byte(0),
        header_byte(1),
        header_byte(2),
        header_byte(3),
    ]);
    assert_eq!(machine.inspect().arm9_peek32(0x023F_FE00), expected);
}

#[test]
fn given_arm9_ram_address_differs_from_rom_offset_when_loading_then_binary_is_at_the_ram_address() {
    let machine = booted(&distinct_image());
    let code = words(&[0xE1A0_0000, 0xE1A0_1001, 0xE1A0_2002]);
    assert_eq!(
        byte_mismatches(&machine, 0x0201_0000, 0x100, |j| code
            .get(j)
            .copied()
            .unwrap_or_else(|| arm9_byte(j))),
        Vec::<usize>::new()
    );
}

#[test]
fn given_valid_image_when_loading_then_arm9_binary_copy_does_not_run_past_its_size() {
    let machine = booted(&Image::valid());
    assert_eq!(machine.inspect().arm9_peek32(RAM + 0x100), 0);
}

#[test]
fn given_arm7_in_main_ram_when_loading_then_binary_is_copied_to_its_ram_address() {
    let machine = booted(&Image::valid());
    assert_eq!(
        byte_mismatches(&machine, 0x0238_0000, 0x80, arm7_byte),
        Vec::<usize>::new()
    );
}

#[test]
fn given_arm7_binary_in_main_ram_when_loading_then_the_byte_after_it_is_zero() {
    let machine = booted(&Image::valid());
    assert_eq!(machine.inspect().arm9_peek32(0x0238_0080), 0);
}

#[test]
fn given_valid_image_when_loading_then_all_other_main_ram_is_zero() {
    let machine = booted(&Image::valid());
    let allowed = [
        (RAM, 0x100),
        (0x0238_0000, 0x80),
        (HEADER_COPY & 0x023F_FFFF, HEADER_LEN as u32),
    ];
    assert_eq!(stray_nonzero_words(&machine, &allowed), Vec::<u32>::new());
}

#[test]
fn given_arm7_in_the_wram_range_when_loading_then_it_is_accepted() {
    let mut image = Image::valid();
    image.arm7.ram_address = 0x037F_8000;
    image.arm7.entry = 0x037F_8000;
    assert_eq!(Machine::new().load_rom(image.build()), Ok(()));
}

#[test]
fn given_arm7_in_the_wram_range_when_loading_then_nothing_else_in_main_ram_changes() {
    let mut image = Image::valid();
    image.arm7.ram_address = 0x037F_8000;
    image.arm7.entry = 0x037F_8000;
    let machine = booted(&image);
    let allowed = [(RAM, 0x100), (HEADER_COPY & 0x023F_FFFF, HEADER_LEN as u32)];
    assert_eq!(stray_nonzero_words(&machine, &allowed), Vec::<u32>::new());
}

#[test]
fn given_overlapping_binaries_when_loading_then_the_arm7_bytes_win() {
    let mut image = Image::valid();
    image.arm7.ram_address = RAM + 0x80;
    image.arm7.entry = RAM + 0x80;
    let machine = booted(&image);
    // ARM9 first, then ARM7: from 0x80 on, RAM holds the ARM7 bytes.
    assert_eq!(
        byte_mismatches(&machine, RAM + 0x80, 0x80, arm7_byte),
        Vec::<usize>::new()
    );
}

#[test]
fn given_overlapping_binaries_when_loading_then_the_arm9_bytes_before_the_overlap_stay() {
    let mut image = Image::valid();
    image.arm7.ram_address = RAM + 0x80;
    image.arm7.entry = RAM + 0x80;
    let machine = booted(&image);
    assert_eq!(
        byte_mismatches(&machine, RAM, 0x80, arm9_byte),
        Vec::<usize>::new()
    );
}

// ---- ARM9 state at entry ----

#[test]
fn given_valid_image_when_loading_then_r0_to_r11_are_zero() {
    let machine = booted(&distinct_image());
    assert_eq!(nonzero_core_registers(&machine), Vec::<(usize, u32)>::new());
}

#[test]
fn given_entry_differs_from_ram_address_when_loading_then_r12_is_the_entry() {
    let machine = booted(&distinct_image());
    assert_eq!(machine.inspect().arm9().regs().get(12), 0x0201_0040);
}

#[test]
fn given_entry_differs_from_ram_address_when_loading_then_r14_is_the_entry() {
    let machine = booted(&distinct_image());
    assert_eq!(machine.inspect().arm9().regs().get(14), 0x0201_0040);
}

#[test]
fn given_valid_image_when_loading_then_running_mode_r13_is_03002f7c() {
    let machine = booted(&Image::valid());
    assert_eq!(machine.inspect().arm9().regs().get(13), 0x0300_2F7C);
}

#[test]
fn given_valid_image_when_loading_then_irq_stack_pointer_is_03003f80() {
    let machine = booted(&Image::valid());
    assert_eq!(
        machine.inspect().arm9().regs().banked(Bank::Irq, 13),
        0x0300_3F80
    );
}

#[test]
fn given_valid_image_when_loading_then_supervisor_stack_pointer_is_03003fc0() {
    let machine = booted(&Image::valid());
    assert_eq!(
        machine.inspect().arm9().regs().banked(Bank::Supervisor, 13),
        0x0300_3FC0
    );
}

#[test]
fn given_valid_image_when_loading_then_r14_irq_and_r14_svc_are_zero() {
    let machine = booted(&Image::valid());
    let regs = machine.inspect().arm9().regs();
    assert_eq!(
        (
            regs.banked(Bank::Irq, 14),
            regs.banked(Bank::Supervisor, 14)
        ),
        (0, 0)
    );
}

#[test]
fn given_valid_image_when_loading_then_spsr_irq_and_spsr_svc_are_zero() {
    let machine = booted(&Image::valid());
    let regs = machine.inspect().arm9().regs();
    assert_eq!(
        (regs.spsr(Bank::Irq), regs.spsr(Bank::Supervisor)),
        (Some(0), Some(0))
    );
}

#[test]
fn given_valid_image_when_loading_then_cpsr_is_system_arm_with_i_and_f_set() {
    let machine = booted(&Image::valid());
    assert_eq!(machine.inspect().arm9().regs().cpsr(), 0x0000_00DF);
}

#[test]
fn given_valid_image_when_loading_then_the_register_bank_in_use_is_user_system() {
    let machine = booted(&Image::valid());
    assert_eq!(machine.inspect().arm9().regs().bank(), Bank::UserSystem);
}

// ---- Pipeline ----

#[test]
fn given_code_at_the_entry_when_loading_then_pipeline_holds_the_first_two_opcodes() {
    // The entry is 0x40 bytes into the binary, so the opcodes sit after a 0x40-byte lead-in.
    let mut image = distinct_image();
    image.arm9_code = [vec![0; 0x40], words(&[0xE1A0_0000, 0xE1A0_1001])].concat();
    let machine = booted(&image);
    assert_eq!(
        machine.inspect().arm9().pipeline(),
        [0xE1A0_0000, 0xE1A0_1001]
    );
}

#[test]
fn given_entry_differs_from_ram_address_when_loading_then_r15_is_entry_plus_8() {
    let machine = booted(&distinct_image());
    assert_eq!(machine.inspect().arm9().regs().get(15), 0x0201_0048);
}

// ---- CP15 ----

#[test]
fn given_valid_image_when_loading_then_both_tcms_are_disabled() {
    let machine = booted(&Image::valid());
    let cp15 = machine.inspect().arm9().cp15();
    assert_eq!((cp15.itcm().enabled, cp15.dtcm().enabled), (false, false));
}

#[test]
fn given_valid_image_when_loading_then_cp15_control_is_the_power_on_value_0x78() {
    let machine = booted(&Image::valid());
    assert_eq!(machine.inspect().arm9().cp15().control(), 0x78);
}

// ---- Refusal ----

#[test]
fn given_refused_header_when_loading_then_the_error_names_the_problem() {
    let mut image = Image::valid();
    image.arm9.size = 0;
    assert_eq!(
        Machine::new().load_rom(image.build()),
        Err(HeaderError::ZeroSize {
            binary: crate::cart::Binary::Arm9
        })
    );
}

#[test]
fn given_refused_header_when_loading_then_machine_is_in_power_on_state() {
    let mut image = Image::valid();
    image.arm9.size = 0;
    let mut machine = Machine::new();
    machine.load_rom(image.build()).ok();
    assert_eq!(machine, Machine::new());
}

#[test]
fn given_refused_header_when_loading_then_there_is_no_header_and_rom_is_empty() {
    let mut image = Image::valid();
    image.arm9.size = 0;
    let mut machine = Machine::new();
    machine.load_rom(image.build()).ok();
    assert_eq!((machine.header().copied(), machine.rom().len()), (None, 0));
}

#[test]
fn given_booted_machine_when_a_refused_rom_is_loaded_then_the_earlier_program_is_gone() {
    let mut machine = booted(&Image::valid());
    machine.load_rom(vec![1, 2, 3]).ok();
    assert_eq!(machine, Machine::new());
}

#[test]
fn given_refused_header_when_running_a_frame_then_only_the_frame_count_advances() {
    let mut machine = Machine::new();
    machine.load_rom(vec![0; 16]).ok();
    machine.run_frame();
    let mut expected = Machine::new();
    expected.run_frame();
    assert_eq!(machine, expected);
}

#[test]
fn given_refused_header_when_running_a_frame_then_the_arm9_is_not_stepped() {
    let mut machine = Machine::new();
    machine.load_rom(vec![0; 16]).ok();
    machine.run_frame();
    assert_eq!(machine.inspect().arm9(), Machine::new().inspect().arm9());
}

// ---- Second load ----

#[test]
fn given_a_booted_program_when_loading_another_then_the_first_programs_ram_is_zero() {
    let mut machine = booted(&distinct_image());
    machine.load_rom(Image::valid().build()).expect("valid");
    assert_eq!(machine.inspect().arm9_peek32(0x0201_0000), 0);
}

#[test]
fn given_a_booted_program_when_loading_another_then_registers_are_the_new_programs() {
    let mut machine = booted(&distinct_image());
    machine.load_rom(Image::valid().build()).expect("valid");
    assert_eq!(machine.inspect().arm9().regs().get(12), 0x0200_0000);
}

// ---- Running ----

#[test]
fn given_a_counting_loop_when_running_one_frame_then_r0_is_560190() {
    let mut machine = booted(&loop_image());
    machine.run_frame();
    // 263 * 355 * 6 * 2 = 1,120,380 steps; the loop is two instructions per count.
    assert_eq!(machine.inspect().arm9().regs().get(0), 560_190);
}

#[test]
fn given_a_counting_loop_when_running_two_frames_then_r0_is_1120380() {
    let mut machine = booted(&loop_image());
    machine.run_frame();
    machine.run_frame();
    assert_eq!(machine.inspect().arm9().regs().get(0), 1_120_380);
}

#[test]
fn given_a_booted_program_when_running_a_frame_then_frame_count_is_one() {
    let mut machine = booted(&loop_image());
    machine.run_frame();
    assert_eq!(machine.frame_count(), 1);
}

#[test]
fn given_a_booted_program_when_cloned_after_one_frame_then_both_copies_agree_after_the_next() {
    let mut straight = booted(&loop_image());
    straight.run_frame();
    straight.run_frame();
    let mut resumed = booted(&loop_image());
    resumed.run_frame();
    let mut copy = resumed.clone();
    copy.run_frame();
    assert_eq!(
        (straight == copy, copy.inspect().arm9().regs().get(0)),
        (true, 1_120_380)
    );
}

// ------------------------------------------------ an ARM7 binary in WRAM (`wramcnt.md`, #22)

/// An image whose ARM7 binary is `size` bytes placed at `ram_address` in the WRAM range.
fn arm7_in_wram(ram_address: u32, size: u32) -> Image {
    let mut image = Image::valid();
    image.arm7.ram_address = ram_address;
    image.arm7.entry = ram_address;
    image.arm7.size = size;
    image
}

/// Byte offsets (in `0..len`) where the ARM7's view at `base` differs from `expected(offset)`.
fn arm7_byte_mismatches(
    m: &Machine,
    base: u32,
    len: usize,
    expected: impl Fn(usize) -> u8,
) -> Vec<usize> {
    (0..len)
        .filter(|&i| m.inspect().arm7_peek8(0, base + i as u32) != expected(i))
        .collect()
}

#[test]
fn given_an_arm7_binary_at_037f_8000_when_loading_then_the_arm7_reads_it_there() {
    // Direct boot sets WRAMCNT = 3, so 037F_8000h is the start of the shared WRAM, all the
    // ARM7's (`wramcnt.md`, "The 037F_8000h mirror" and "Unverified").
    let machine = booted(&arm7_in_wram(0x037F_8000, 0x80));
    assert_eq!(
        arm7_byte_mismatches(&machine, 0x037F_8000, 0x80, arm7_byte),
        Vec::<usize>::new()
    );
}

#[test]
fn given_an_arm7_binary_at_037f_8000_when_loading_then_0380_8000_is_plain_arm7_wram_not_a_repeat() {
    // With WRAMCNT = 3 the binary is in the shared WRAM; 0380_8000h is the ARM7's own WRAM, which
    // boot leaves zero. A 64 KiB repetition would show the binary there (`wramcnt.md`, "Unverified").
    let machine = booted(&arm7_in_wram(0x037F_8000, 0x80));
    assert_eq!(
        arm7_byte_mismatches(&machine, 0x0380_8000, 0x80, |_| 0),
        Vec::<usize>::new()
    );
}

#[test]
fn given_an_arm7_binary_that_crosses_0380_0000_when_loading_then_it_continues_in_the_next_range() {
    // 0x40 bytes end at 037F_FFFFh and the rest start at 0380_0000h; both are the ARM7's WRAM.
    let machine = booted(&arm7_in_wram(0x037F_FFC0, 0x100));
    let tail = |i: usize| arm7_byte(0x40 + i);
    assert_eq!(
        (
            arm7_byte_mismatches(&machine, 0x037F_FFC0, 0x40, arm7_byte),
            arm7_byte_mismatches(&machine, 0x0380_0000, 0xC0, tail),
        ),
        (Vec::<usize>::new(), Vec::<usize>::new())
    );
}

#[test]
fn given_an_arm7_binary_at_037f_8000_when_loading_then_the_byte_after_it_is_zero() {
    let machine = booted(&arm7_in_wram(0x037F_8000, 0x80));
    assert_eq!(machine.inspect().arm7_peek8(0, 0x037F_8080), 0);
}

#[test]
fn given_an_arm7_binary_at_037f_8000_when_loading_then_the_arm9_owns_no_shared_wram() {
    // Mode 3 hands all of the shared WRAM to the ARM7, so the ARM9 window is empty and reads 0
    // although the binary sits in the shared WRAM (`wramcnt.md`, "Behavior" and "Unverified").
    let machine = booted(&arm7_in_wram(0x037F_8000, 0x80));
    assert_eq!(
        (
            machine.inspect().arm9_peek32(0x037F_8000),
            machine.inspect().arm9_peek32(0x0300_0000),
            machine.inspect().wramcnt().mode
        ),
        (0, 0, 3)
    );
}

#[test]
fn given_direct_boot_of_any_image_when_loading_then_wramcnt_mode_is_3() {
    let machine = booted(&Image::valid());
    assert_eq!(machine.inspect().wramcnt().mode, 3);
}

#[test]
fn given_an_arm7_binary_larger_than_7d80_bytes_at_037f_8000_when_loading_then_every_region_reads_back_without_aliasing()
 {
    // 0x8100 bytes: 0x8000 fill the shared WRAM (037F_8000h..037F_FFFFh), the last 0x100 start
    // the ARM7's own WRAM at 0380_0000h. The old stack address 0380_FD80h is a different byte
    // from 037F_FD80h (binary offset 0x7D80, nonzero), so placing did not alias it.
    let machine = booted(&arm7_in_wram(0x037F_8000, 0x8100));
    let seam = |i: usize| arm7_byte(0x8000 + i);
    assert_eq!(
        (
            arm7_byte_mismatches(&machine, 0x037F_8000, 0x100, arm7_byte),
            arm7_byte_mismatches(&machine, 0x037F_FF80, 0x80, |i| arm7_byte(0x7F80 + i)),
            arm7_byte_mismatches(&machine, 0x0380_0000, 0x100, seam),
            machine.inspect().arm7_peek8(0, 0x037F_FD80),
            machine.inspect().arm7_peek8(0, 0x0380_FD80),
        ),
        (
            Vec::<usize>::new(),
            Vec::<usize>::new(),
            Vec::<usize>::new(),
            arm7_byte(0x7D80),
            0
        )
    );
}

#[test]
fn given_an_arm7_binary_at_037f_8000_when_loading_then_the_arm7_starts_on_its_first_opcode() {
    // The ARM7's pipeline is filled through its own view at the entry point, so r15 reads
    // entry + 8 (two opcodes ahead) and the first opcode is the one the binary holds.
    let mut image = arm7_in_wram(0x037F_8000, 0x80);
    image.arm7_code = words(&[ADD_R0_1, ADD_R0_1, B_BACK_ONE]);
    let mut machine = booted(&image);
    machine.step_arm7_instruction();
    assert_eq!(machine.inspect().arm7().regs().get(0), 1);
}

// ------------------------------------------------------------------ the BIOS trap addresses

use crate::hle::bios::{Trap, trap_at};

/// The R15 values at which each CPU's machine takes over: the vector + 8 (`hle-swi.md`, "How the
/// machine takes the BIOS's place"). Rows: (R15, ARM9's trap, ARM7's trap).
fn trap_table_failures() -> Vec<u32> {
    let table = [
        (0x0000_0010, None, Some(Trap::Swi)),
        (0x0000_0020, None, Some(Trap::Irq)),
        (0x0000_0028, None, Some(Trap::IrqReturn)),
        (0x0000_0014, None, None),
        (0x0000_0018, None, None),
        (0x0000_002C, None, Some(Trap::IntrWaitLoop)),
        (0x0000_0030, None, None),
        (0x0200_0010, None, None),
        (0xFFFF_0010, Some(Trap::Swi), None),
        (0xFFFF_0020, Some(Trap::Irq), None),
        (0xFFFF_0028, Some(Trap::IrqReturn), None),
        (0xFFFF_002C, Some(Trap::IntrWaitLoop), None),
        (0xFFFF_0024, None, None),
        (0xFFFE_0010, None, None),
    ];
    table
        .iter()
        .filter(|&&(pc, nine, seven)| (trap_at::<true>(pc), trap_at::<false>(pc)) != (nine, seven))
        .map(|&(pc, ..)| pc)
        .collect()
}

#[test]
fn given_the_vector_addresses_plus_eight_when_looked_up_then_each_cpu_traps_only_in_its_own_bios() {
    assert_eq!(trap_table_failures(), Vec::<u32>::new());
}
