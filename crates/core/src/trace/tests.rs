// SPDX-License-Identifier: Apache-2.0
//! The trace text form: encode, parse, and their errors. Expected values come from the format in
//! `docs/contributing/trace-oracle.md` ("The trace format"), never from running the code: one
//! space between columns, lowercase hex of a fixed width, a 4-digit opcode in Thumb state (CPSR
//! bit 5), and r15 equal to the pc column.

use super::*;
use proptest::prelude::*;

/// CPSR of the line below: System mode, ARM state, I and F set.
const ARM_CPSR: u32 = 0x0000_00DF;
/// The same with CPSR bit 5 set.
const THUMB_CPSR: u32 = 0x0000_00FF;

/// An ARM9 line at step 0 of `mov r0, #0x55` at 0x02000000, every register 0 but r15.
const ARM_LINE: &str = "9 00000000 02000000 e3a00055 00000000 00000000 00000000 00000000 \
00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
00000000 02000000 000000df";

/// An ARM7 Thumb line at step 0x2a: `movs r1, #0x66` at 0x0238000a, r0 = 0x55.
const THUMB_LINE: &str = "7 0000002a 0238000a 2166 00000055 00000000 00000000 00000000 \
00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
00000000 0238000a 000000ff";

fn arm_record() -> TraceRecord {
    let mut regs = [0; 16];
    regs[15] = 0x0200_0000;
    TraceRecord {
        cpu: TraceCpu::Arm9,
        step: 0,
        opcode: Opcode::Arm(0xE3A0_0055),
        regs,
        cpsr: ARM_CPSR,
    }
}

fn thumb_record() -> TraceRecord {
    let mut regs = [0; 16];
    regs[0] = 0x55;
    regs[15] = 0x0238_000A;
    TraceRecord {
        cpu: TraceCpu::Arm7,
        step: 0x2A,
        opcode: Opcode::Thumb(0x2166),
        regs,
        cpsr: THUMB_CPSR,
    }
}

/// `ARM_LINE` with the column at `index` (0 = cpu) replaced.
fn arm_line_with(index: usize, token: &str) -> String {
    let mut tokens: Vec<&str> = ARM_LINE.split(' ').collect();
    tokens[index] = token;
    tokens.join(" ")
}

/// `THUMB_LINE` with the column at `index` replaced.
fn thumb_line_with(index: usize, token: &str) -> String {
    let mut tokens: Vec<&str> = THUMB_LINE.split(' ').collect();
    tokens[index] = token;
    tokens.join(" ")
}

/// Column indices in a line.
const COL_CPU: usize = 0;
const COL_STEP: usize = 1;
const COL_PC: usize = 2;
const COL_OPCODE: usize = 3;
const COL_R0: usize = 4;
const COL_CPSR: usize = 20;

/// Every malformed line paired with the error it must give; returns the cases that differ.
fn line_error_failures() -> Vec<String> {
    let short = ARM_LINE
        .rsplit_once(' ')
        .map(|(head, _)| head.to_string())
        .unwrap_or_default();
    let long = format!("{ARM_LINE} 00000000");
    let cases: Vec<(&str, String, LineError)> = vec![
        ("empty line", String::new(), LineError::Columns),
        ("one column missing", short, LineError::Columns),
        ("one column extra", long, LineError::Columns),
        ("cpu 8", arm_line_with(COL_CPU, "8"), LineError::Cpu),
        (
            "cpu two digits",
            arm_line_with(COL_CPU, "09"),
            LineError::Cpu,
        ),
        (
            "uppercase opcode",
            arm_line_with(COL_OPCODE, "E3A00055"),
            LineError::Number("opcode"),
        ),
        (
            "uppercase step",
            arm_line_with(COL_STEP, "0000000A"),
            LineError::Number("step"),
        ),
        (
            "uppercase register",
            arm_line_with(COL_R0, "0000000A"),
            LineError::Number("register"),
        ),
        (
            "uppercase cpsr",
            arm_line_with(COL_CPSR, "000000DF"),
            LineError::Number("cpsr"),
        ),
        (
            "short step",
            arm_line_with(COL_STEP, "0"),
            LineError::Number("step"),
        ),
        (
            "short register",
            arm_line_with(COL_R0, "0000000"),
            LineError::Number("register"),
        ),
        (
            "long cpsr",
            arm_line_with(COL_CPSR, "0000000df"),
            LineError::Number("cpsr"),
        ),
        (
            "non-hex pc",
            arm_line_with(COL_PC, "0200000g"),
            LineError::Number("pc"),
        ),
        (
            "0x prefix",
            arm_line_with(COL_R0, "0x000000"),
            LineError::Number("register"),
        ),
        (
            "pc differs from r15",
            arm_line_with(COL_PC, "02000004"),
            LineError::Pc,
        ),
        (
            "4-digit opcode in ARM state",
            arm_line_with(COL_OPCODE, "0055"),
            LineError::State,
        ),
        (
            "8-digit opcode in Thumb state",
            thumb_line_with(COL_OPCODE, "00002166"),
            LineError::State,
        ),
        (
            "6-digit opcode in ARM state",
            arm_line_with(COL_OPCODE, "e3a000"),
            LineError::State,
        ),
    ];
    cases
        .into_iter()
        .filter_map(|(name, line, expected)| {
            let got = parse_line(&line);
            (got != Err(expected))
                .then(|| format!("{name}: expected Err({expected:?}), got {got:?}"))
        })
        .collect()
}

#[test]
fn given_a_known_arm_line_when_parsed_then_it_is_the_expected_record() {
    assert_eq!(parse_line(ARM_LINE), Ok(arm_record()));
}

#[test]
fn given_a_known_thumb_line_when_parsed_then_the_opcode_is_a_halfword_record() {
    assert_eq!(parse_line(THUMB_LINE), Ok(thumb_record()));
}

#[test]
fn given_a_known_arm_record_when_printed_then_it_is_the_fixed_width_line() {
    assert_eq!(arm_record().to_string(), ARM_LINE);
}

#[test]
fn given_a_known_thumb_record_when_printed_then_the_opcode_has_four_digits() {
    assert_eq!(thumb_record().to_string(), THUMB_LINE);
}

#[test]
fn given_a_record_when_asked_for_pc_then_it_is_r15() {
    assert_eq!(thumb_record().pc(), 0x0238_000A);
}

#[test]
fn given_each_malformed_line_when_parsed_then_each_gives_its_error() {
    assert_eq!(line_error_failures(), Vec::<String>::new());
}

#[test]
fn given_a_header_other_than_the_format_when_parsed_then_it_is_refused() {
    let texts = [
        "ninge-trace 2\n",
        "ninge-trace 1 \n",
        "Ninge-trace 1\n",
        "# c\nninge-trace 1\n",
        "",
    ];
    let results: Vec<_> = texts.iter().map(|t| parse_trace(t)).collect();
    assert_eq!(results, vec![Err(TraceError::Header); texts.len()]);
}

#[test]
fn given_only_the_header_when_parsed_then_the_trace_is_empty() {
    assert_eq!(parse_trace("ninge-trace 1\n"), Ok(vec![]));
}

#[test]
fn given_comments_and_blank_lines_when_parsed_then_only_records_remain() {
    let text = format!("ninge-trace 1\n# a comment\n\n{ARM_LINE}\n#another\n\n{THUMB_LINE}\n");
    assert_eq!(parse_trace(&text), Ok(vec![arm_record(), thumb_record()]));
}

#[test]
fn given_a_bad_record_after_comments_when_parsed_then_the_error_names_its_1_based_line() {
    // Line 1 header, 2 comment, 3 blank, 4 valid, 5 malformed.
    let text = format!(
        "ninge-trace 1\n# c\n\n{ARM_LINE}\n{}\n",
        arm_line_with(COL_CPU, "8")
    );
    assert_eq!(
        parse_trace(&text),
        Err(TraceError::Line {
            line: 5,
            error: LineError::Cpu
        })
    );
}

#[test]
fn given_a_bad_first_record_when_parsed_then_the_error_is_line_2() {
    let text = format!("ninge-trace 1\n{}\n", arm_line_with(COL_PC, "02000004"));
    assert_eq!(
        parse_trace(&text),
        Err(TraceError::Line {
            line: 2,
            error: LineError::Pc
        })
    );
}

/// A record whose opcode width follows the Thumb bit and whose pc is r15.
fn record_strategy(thumb: bool) -> impl Strategy<Value = TraceRecord> {
    (
        prop::bool::ANY,
        any::<u32>(),
        any::<[u32; 16]>(),
        any::<u32>(),
        any::<u32>(),
    )
        .prop_map(move |(nine, step, regs, cpsr, word)| TraceRecord {
            cpu: if nine { TraceCpu::Arm9 } else { TraceCpu::Arm7 },
            step,
            opcode: if thumb {
                Opcode::Thumb(word as u16)
            } else {
                Opcode::Arm(word)
            },
            regs,
            cpsr: if thumb { cpsr | FLAG_T } else { cpsr & !FLAG_T },
        })
}

proptest! {
    #[test]
    fn given_any_arm_record_when_printed_and_parsed_then_it_is_unchanged(r in record_strategy(false)) {
        prop_assert_eq!(parse_line(&r.to_string()), Ok(r));
    }

    #[test]
    fn given_any_thumb_record_when_printed_and_parsed_then_it_is_unchanged(r in record_strategy(true)) {
        prop_assert_eq!(parse_line(&r.to_string()), Ok(r));
    }

    #[test]
    fn given_any_record_when_printed_then_the_line_has_21_lowercase_columns(
        r in prop_oneof![record_strategy(false), record_strategy(true)],
    ) {
        let line = r.to_string();
        let columns = line.split(' ').count();
        let lowercase = line == line.to_lowercase();
        prop_assert_eq!((columns, lowercase), (21, true));
    }

    #[test]
    fn given_any_records_when_written_as_a_trace_and_parsed_then_they_are_unchanged(
        rs in prop::collection::vec(prop_oneof![record_strategy(false), record_strategy(true)], 0..6),
    ) {
        let body: String = rs.iter().map(|r| format!("{r}\n")).collect();
        prop_assert_eq!(parse_trace(&format!("{HEADER}\n{body}")), Ok(rs));
    }
}

// ---- The BIOS area (`docs/hw/hle-swi.md`, "The BIOS area"; `arm9-memory-map.md`) ----

/// Addresses at and around both edges of both areas, so an off-by-one on either side shows.
const EDGE_PCS: [u32; 12] = [
    0x0000_0000,
    0x0000_0001,
    0x0000_3FFF,
    0x0000_4000,
    0x0200_0000,
    0xFFFE_FFFF,
    0xFFFF_0000,
    0xFFFF_0018,
    0xFFFF_7FFF,
    0xFFFF_8000,
    0xFFFF_FFFF,
    0x0000_4001,
];

/// The edge addresses inside the ARM9 BIOS (32 KiB at `FFFF0000h`), written by hand from the
/// memory map and not derived from the code under test.
const ARM9_EDGES_INSIDE: [u32; 3] = [0xFFFF_0000, 0xFFFF_0018, 0xFFFF_7FFF];

/// The edge addresses inside the ARM7 BIOS (16 KiB at 0).
const ARM7_EDGES_INSIDE: [u32; 3] = [0x0000_0000, 0x0000_0001, 0x0000_3FFF];

/// The edge addresses where `inside` is the hand-written list of members, for which `contains`
/// gives a different answer. Empty when the two agree on every edge.
fn membership_disagreements(inside: &[u32], contains: impl Fn(u32) -> bool) -> Vec<u32> {
    EDGE_PCS
        .iter()
        .copied()
        .filter(|pc| contains(*pc) != inside.contains(pc))
        .collect()
}

#[test]
fn given_the_arm9_when_asked_for_its_bios_area_then_it_is_32_kib_at_ffff0000() {
    assert_eq!(TraceCpu::Arm9.bios_area(), 0xFFFF_0000..0xFFFF_8000);
}

#[test]
fn given_the_arm7_when_asked_for_its_bios_area_then_it_is_16_kib_at_zero() {
    assert_eq!(TraceCpu::Arm7.bios_area(), 0..0x4000);
}

#[test]
fn given_the_edge_addresses_when_the_arm9_area_is_asked_then_only_the_hand_listed_ones_are_inside()
{
    let area = TraceCpu::Arm9.bios_area();
    let wrong = membership_disagreements(&ARM9_EDGES_INSIDE, |pc| area.contains(&pc));
    assert_eq!(wrong, Vec::<u32>::new());
}

#[test]
fn given_the_edge_addresses_when_the_arm7_area_is_asked_then_only_the_hand_listed_ones_are_inside()
{
    let area = TraceCpu::Arm7.bios_area();
    let wrong = membership_disagreements(&ARM7_EDGES_INSIDE, |pc| area.contains(&pc));
    assert_eq!(wrong, Vec::<u32>::new());
}

#[test]
fn given_the_edge_addresses_when_the_traced_arm9_filter_runs_then_it_agrees_with_the_hand_list() {
    let wrong = membership_disagreements(&ARM9_EDGES_INSIDE, in_bios::<true>);
    assert_eq!(wrong, Vec::<u32>::new());
}

#[test]
fn given_the_edge_addresses_when_the_traced_arm7_filter_runs_then_it_agrees_with_the_hand_list() {
    let wrong = membership_disagreements(&ARM7_EDGES_INSIDE, in_bios::<false>);
    assert_eq!(wrong, Vec::<u32>::new());
}

#[test]
fn given_the_edge_addresses_when_the_oracle_area_and_the_traced_filter_are_compared_then_they_agree()
 {
    let area9 = TraceCpu::Arm9.bios_area();
    let area7 = TraceCpu::Arm7.bios_area();
    let differ: Vec<u32> = EDGE_PCS
        .iter()
        .copied()
        .filter(|pc| {
            area9.contains(pc) != in_bios::<true>(*pc)
                || area7.contains(pc) != in_bios::<false>(*pc)
        })
        .collect();
    assert_eq!(differ, Vec::<u32>::new());
}
