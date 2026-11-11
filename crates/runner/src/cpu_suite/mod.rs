// SPDX-License-Identifier: Apache-2.0
//! The SingleStepTests/ARM7TDMI `.json.bin` suite: a reader, a replaying bus, and a harness
//! that runs every case through a [`Cpu`] and reports what differs (#7, AGENTS.md §5.1).
//!
//! The suite is fetched by `scripts/fetch_cpu_suite.sh` into `tests/data/` and never
//! vendored. Each file holds many cases; a case is one instruction with the CPU state before
//! and after and the memory accesses in between.
//!
//! # Register convention
//!
//! A state lists the registers as *physical* storage, not as the current mode sees them:
//! `R` holds R0–R15 of the User/System bank, and `R_fiq`, `R_svc`, `R_abt`, `R_irq`, `R_und`
//! hold each exception mode's own registers (R8–R14 for FIQ, R13–R14 for the others), whether
//! or not that mode is active. This was measured on the pinned suite, not assumed: in every
//! case whose CPSR names an exception mode, the `R` slots that mode banks never equal its
//! bank array, and across instructions that do not change the mode the bank slots change
//! about 30 to 90 times as often as those `R` slots (`tests/cpu_suite.rs` pins both facts). So the
//! values a program sees in IRQ mode are `R0–R12` and `R_irq`, and the User R13–R14 stay in
//! `R`. The harness loads and compares in exactly this form, so no bank is ever "unknown".
//!
//! Some final states carry mode bits that name no mode (an instruction wrote an invalid mode
//! with MSR); what the registers mean then is left to the MSR work (#12).
//!
//! # File format
//!
//! Little-endian words: the magic `0xD33DBAE0`, the case count, then the cases. A case is its
//! own size (including that word), an initial state block, a final state block, a
//! transactions block, and an opcode block; every size word counts the whole block. A state
//! block is 168 bytes: size, a marker (1 for the initial state, 2 for the final one), then 40
//! words (R0–R15, R_fiq 7, R_svc 2, R_abt 2, R_irq 2, R_und 2, CPSR, five SPSRs, two pipeline
//! words, access). A transactions block is `12 + 24 * count` bytes: size, the magic 3, the
//! count, then `{kind, size, address, data, cycle, access}` per transaction. The opcode block
//! is 16 bytes: size, the marker 4, the opcode, the base address. The reader checks every one
//! of these and that no byte is left over; the three markers are constant in all 2,250,000
//! cases of the pinned suite.
//!
//! The harness loads the case's `pipeline` and compares R0–R15, every register bank, CPSR, the
//! five SPSRs, the final `pipeline`, and every transaction in order (kind, size, address,
//! data), fetches included. It does not compare `cycle` (the suite marks it experimental) or
//! `access`. Decided in #76 (registers) and #NN (the pipeline and the fetch comparison).

use std::fmt;
use std::path::{Path, PathBuf};

use ninge_core::cpu::{Bank, Bus, Cpu};

/// First word of every `.json.bin` file.
const MAGIC: u32 = 0xD33D_BAE0;
/// Second word of a transactions block.
const TRANSACTIONS_MAGIC: u32 = 3;
/// Registers, banked registers, CPSR, SPSRs, pipeline and access in a state block.
const STATE_WORDS: usize = 40;
/// Bytes of a state block: its size, a marker word, then [`STATE_WORDS`] words.
const STATE_BLOCK_BYTES: usize = 8 + 4 * STATE_WORDS;
/// Marker word of an initial state block, of a final one, and of the opcode block. The
/// converter skips these words; in all 2,250,000 cases of the pinned suite they are constant,
/// so the reader checks them as magic numbers.
const INITIAL_MARKER: u32 = 1;
const FINAL_MARKER: u32 = 2;
const OPCODE_MARKER: u32 = 4;
/// A transactions block is its size, the magic, and a count; the entries follow.
const TRANSACTIONS_HEADER_BYTES: usize = 12;
/// Bytes of one transaction: kind, size, address, data, cycle, access.
const TRANSACTION_BYTES: usize = 24;
/// Bytes of an opcode block: its size, a marker word, the opcode, and the base address.
const OPCODE_BLOCK_BYTES: usize = 16;
/// Bytes of a case's own size word.
const CASE_SIZE_BYTES: usize = 4;
/// Bytes of the file header: the magic and the case count.
const FILE_HEADER_BYTES: usize = 8;

/// Transaction kind of a code fetch.
const KIND_FETCH: u32 = 0;
/// Transaction kind of a data read.
const KIND_READ: u32 = 1;
/// Transaction kind of a data write.
const KIND_WRITE: u32 = 2;

/// Why a suite file could not be read.
#[derive(Debug)]
pub enum SuiteError {
    /// The file or directory could not be read.
    Io {
        /// What failed.
        path: PathBuf,
        /// The underlying IO error.
        source: std::io::Error,
    },
    /// The bytes are not a well-formed suite file.
    Format {
        /// Byte offset where decoding stopped.
        offset: usize,
        /// What is wrong.
        reason: String,
    },
}

impl fmt::Display for SuiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Format { offset, reason } => {
                write!(f, "bad suite data at byte {offset}: {reason}")
            }
        }
    }
}

impl std::error::Error for SuiteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Format { .. } => None,
        }
    }
}

/// CPU state as the suite stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// R0–R15 of the User/System bank: physical storage, not the active mode's view (see the
    /// register convention in the module docs).
    pub r: [u32; 16],
    /// FIQ-banked R8–R14.
    pub r_fiq: [u32; 7],
    /// Supervisor-banked R13–R14.
    pub r_svc: [u32; 2],
    /// Abort-banked R13–R14.
    pub r_abt: [u32; 2],
    /// IRQ-banked R13–R14.
    pub r_irq: [u32; 2],
    /// Undefined-banked R13–R14.
    pub r_und: [u32; 2],
    /// CPSR.
    pub cpsr: u32,
    /// SPSR of FIQ, Supervisor, Abort, IRQ, Undefined, in that order.
    pub spsr: [u32; 5],
    /// The two prefetched opcodes: `[0]` executes next (`docs/hw/arm-branch-pipeline.md`).
    pub pipeline: [u32; 2],
    /// Access flags. Not compared.
    pub access: u32,
}

/// One bus transaction of a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transaction {
    /// 0 fetch, 1 data read, 2 data write.
    pub kind: u32,
    /// Bytes transferred: 1, 2 or 4.
    pub size: u32,
    /// Address.
    pub addr: u32,
    /// Value read or written.
    pub data: u32,
    /// Cycle number. Not compared.
    pub cycle: u32,
    /// Access flags. Not compared.
    pub access: u32,
}

/// One test: run `opcode` from `initial` and expect `expected` and `transactions`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    /// State before the instruction; R15 is already ahead of the instruction.
    pub initial: State,
    /// State after the instruction.
    pub expected: State,
    /// Every bus transaction, fetches included.
    pub transactions: Vec<Transaction>,
    /// The instruction.
    pub opcode: u32,
    /// Address the instruction was fetched from.
    pub base_addr: u32,
}

fn format_error(offset: usize, reason: &str) -> SuiteError {
    SuiteError::Format {
        offset,
        reason: reason.to_string(),
    }
}

/// `a + b` for offsets and sizes taken from the file; a sum that overflows `usize` (possible
/// on a 32-bit host with a crafted file) is a format error, not a panic.
fn add(a: usize, b: usize) -> Result<usize, SuiteError> {
    a.checked_add(b)
        .ok_or_else(|| format_error(a, "an offset overflows"))
}

/// Reads little-endian words out of a byte slice without ever panicking.
struct Words<'a> {
    bytes: &'a [u8],
}

impl Words<'_> {
    fn at(&self, offset: usize) -> Result<u32, SuiteError> {
        offset
            .checked_add(4)
            .and_then(|end| self.bytes.get(offset..end))
            .and_then(|word| <[u8; 4]>::try_from(word).ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| format_error(offset, "the data ends inside a word"))
    }

    fn many<const N: usize>(&self, offset: usize) -> Result<[u32; N], SuiteError> {
        let mut out = [0; N];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.at(add(offset, 4 * i)?)?;
        }
        Ok(out)
    }

    /// The size word at `offset` as a `usize`; a value that does not fit stays out of range.
    fn size_at(&self, offset: usize) -> Result<usize, SuiteError> {
        Ok(usize::try_from(self.at(offset)?).unwrap_or(usize::MAX))
    }
}

fn take<const N: usize>(words: &[u32; STATE_WORDS], start: usize) -> [u32; N] {
    let mut out = [0; N];
    out.copy_from_slice(&words[start..start + N]);
    out
}

/// A state block: exactly [`STATE_BLOCK_BYTES`] long, with `marker` after its size.
fn decode_state(words: &Words, offset: usize, marker: u32) -> Result<State, SuiteError> {
    if words.size_at(offset)? != STATE_BLOCK_BYTES {
        return Err(format_error(offset, "a state block is not 168 bytes"));
    }
    if words.at(add(offset, 4)?)? != marker {
        return Err(format_error(
            add(offset, 4)?,
            "a state block has the wrong marker",
        ));
    }
    let v: [u32; STATE_WORDS] = words.many(add(offset, 8)?)?;
    Ok(State {
        r: take(&v, 0),
        r_fiq: take(&v, 16),
        r_svc: take(&v, 23),
        r_abt: take(&v, 25),
        r_irq: take(&v, 27),
        r_und: take(&v, 29),
        cpsr: v[31],
        spsr: take(&v, 32),
        pipeline: take(&v, 37),
        access: v[39],
    })
}

/// A transactions block: `12 + 24 * count` bytes, with the magic after its size.
fn decode_transactions(
    words: &Words,
    offset: usize,
) -> Result<(Vec<Transaction>, usize), SuiteError> {
    let size = words.size_at(offset)?;
    if words.at(add(offset, 4)?)? != TRANSACTIONS_MAGIC {
        return Err(format_error(
            add(offset, 4)?,
            "a transactions block has the wrong magic",
        ));
    }
    let count = usize::try_from(words.at(add(offset, 8)?)?).unwrap_or(usize::MAX);
    let expected = count
        .checked_mul(TRANSACTION_BYTES)
        .and_then(|entries| entries.checked_add(TRANSACTIONS_HEADER_BYTES));
    if expected != Some(size) {
        return Err(format_error(
            offset,
            "a transactions block size does not match its count",
        ));
    }
    let mut list = Vec::new();
    for i in 0..count {
        let at = add(
            add(offset, TRANSACTIONS_HEADER_BYTES)?,
            i * TRANSACTION_BYTES,
        )?;
        let [kind, size, addr, data, cycle, access] = words.many(at)?;
        list.push(Transaction {
            kind,
            size,
            addr,
            data,
            cycle,
            access,
        });
    }
    Ok((list, size))
}

/// One case: its size word, then the four blocks, whose sizes must add up to it exactly.
fn decode_case(words: &Words, offset: usize) -> Result<(Case, usize), SuiteError> {
    let size = words.size_at(offset)?;
    let mut at = add(offset, CASE_SIZE_BYTES)?;
    let initial = decode_state(words, at, INITIAL_MARKER)?;
    at = add(at, STATE_BLOCK_BYTES)?;
    let expected = decode_state(words, at, FINAL_MARKER)?;
    at = add(at, STATE_BLOCK_BYTES)?;
    let (transactions, n) = decode_transactions(words, at)?;
    at = add(at, n)?;
    if words.size_at(at)? != OPCODE_BLOCK_BYTES {
        return Err(format_error(at, "an opcode block is not 16 bytes"));
    }
    if words.at(add(at, 4)?)? != OPCODE_MARKER {
        return Err(format_error(
            add(at, 4)?,
            "an opcode block has the wrong marker",
        ));
    }
    let (opcode, base_addr) = (words.at(add(at, 8)?)?, words.at(add(at, 12)?)?);
    if add(at, OPCODE_BLOCK_BYTES)? != add(offset, size)? {
        return Err(format_error(
            offset,
            "a case size does not match its blocks",
        ));
    }
    let case = Case {
        initial,
        expected,
        transactions,
        opcode,
        base_addr,
    };
    Ok((case, size))
}

/// Decodes the bytes of one `.json.bin` file.
///
/// The reader accepts exactly the layout the pinned suite has: every block has its fixed
/// size, every case size equals the sum of its blocks, and the last case ends at the last
/// byte. Malformed or truncated data is an error, never a panic.
pub fn decode(bytes: &[u8]) -> Result<Vec<Case>, SuiteError> {
    let words = Words { bytes };
    if words.at(0)? != MAGIC {
        return Err(format_error(
            0,
            "the file does not start with the suite magic",
        ));
    }
    let count = usize::try_from(words.at(4)?).unwrap_or(usize::MAX);
    let mut cases = Vec::new();
    let mut offset = FILE_HEADER_BYTES;
    for _ in 0..count {
        let (case, size) = decode_case(&words, offset)?;
        cases.push(case);
        offset = add(offset, size)?;
    }
    if offset != bytes.len() {
        return Err(format_error(offset, "bytes follow the last case"));
    }
    Ok(cases)
}

/// One bus access the CPU made, in the order it made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Access {
    kind: u32,
    size: u32,
    addr: u32,
    data: u32,
}

/// The bus [`run_case_with`] hands to its step closure; it carries no public state.
///
/// It replays one case: reads come from the case's recorded reads, fetches from its opcode, and
/// every access, fetches included, is logged for comparison.
pub struct ReplayBus {
    opcode: u32,
    base_addr: u32,
    reads: Vec<(Transaction, bool)>,
    log: Vec<Access>,
}

impl ReplayBus {
    fn new(case: &Case) -> Self {
        Self {
            opcode: case.opcode,
            base_addr: case.base_addr,
            reads: case
                .transactions
                .iter()
                .filter(|t| t.kind == KIND_READ)
                .map(|&t| (t, false))
                .collect(),
            log: Vec::new(),
        }
    }

    /// The first recorded read of this size at this address that no earlier read used.
    fn read(&mut self, addr: u32, size: u32) -> u32 {
        let found = self
            .reads
            .iter_mut()
            .find(|(t, used)| !*used && t.addr == addr && t.size == size);
        let data = match found {
            Some((t, used)) => {
                *used = true;
                t.data
            }
            None => 0,
        };
        self.log.push(Access {
            kind: KIND_READ,
            size,
            addr,
            data,
        });
        data
    }

    fn write(&mut self, addr: u32, size: u32, data: u32) {
        self.log.push(Access {
            kind: KIND_WRITE,
            size,
            addr,
            data,
        });
    }

    /// The opcode at its own address; any other address yields the address itself, as the
    /// suite's README prescribes. The fetch is logged with its width.
    fn fetch(&mut self, addr: u32, size: u32) -> u32 {
        let data = match addr == self.base_addr {
            true => self.opcode,
            false => addr,
        };
        // A halfword fetch carries the low halfword of that word.
        let data = match size {
            2 => data & 0xFFFF,
            _ => data,
        };
        self.log.push(Access {
            kind: KIND_FETCH,
            size,
            addr,
            data,
        });
        data
    }
}

impl Bus for ReplayBus {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.fetch(addr, 4)
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.fetch(addr, 2) as u16 // truncation intended: a Thumb opcode is the low halfword
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.read(addr, 1) as u8 // truncation intended: the transaction holds a byte
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.read(addr, 2) as u16 // truncation intended: the transaction holds a halfword
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.read(addr, 4)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.write(addr, 1, u32::from(value));
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.write(addr, 2, u32::from(value));
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.write(addr, 4, value);
    }
}

/// One thing that differs between a case's expectation and the CPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDiff {
    /// Which value: `R3`, `R_fiq[2]`, `CPSR`, `SPSR[1]`, `transaction[0]`, `transactions`.
    pub name: String,
    /// What the suite expects.
    pub expected: String,
    /// What the CPU produced.
    pub actual: String,
}

/// A case that failed, with everything that differed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseDiff {
    /// The suite file the case came from.
    pub file: String,
    /// Position of the case within the file.
    pub index: usize,
    /// The instruction.
    pub opcode: u32,
    /// Every difference found.
    pub fields: Vec<FieldDiff>,
}

/// The banks other than User/System with their registers in the suite's order.
const BANKED: [(Bank, &str, usize, usize); 5] = [
    (Bank::Fiq, "R_fiq", 8, 7),
    (Bank::Supervisor, "R_svc", 13, 2),
    (Bank::Abort, "R_abt", 13, 2),
    (Bank::Irq, "R_irq", 13, 2),
    (Bank::Undefined, "R_und", 13, 2),
];

fn bank_values(state: &State, bank: Bank) -> &[u32] {
    match bank {
        Bank::Fiq => &state.r_fiq,
        Bank::Supervisor => &state.r_svc,
        Bank::Abort => &state.r_abt,
        Bank::Irq => &state.r_irq,
        Bank::Undefined => &state.r_und,
        Bank::UserSystem => &[],
    }
}

/// Puts the CPU into `state` (see the register convention in the module docs).
fn load<const ARM9: bool>(cpu: &mut Cpu<ARM9>, state: &State) {
    cpu.set_pipeline(state.pipeline);
    let regs = cpu.regs_mut();
    // The mode first, so the banks are switched before any of them is written.
    regs.set_cpsr(state.cpsr);
    for i in (0..8).chain([15]) {
        regs.set(i, state.r[i]);
    }
    for i in 8..=14 {
        regs.set_banked(Bank::UserSystem, i, state.r[i]);
    }
    for (bank, _, first, count) in BANKED {
        for (i, &value) in bank_values(state, bank).iter().enumerate().take(count) {
            regs.set_banked(bank, first + i, value);
        }
    }
    for (bank, slot) in [
        Bank::Fiq,
        Bank::Supervisor,
        Bank::Abort,
        Bank::Irq,
        Bank::Undefined,
    ]
    .into_iter()
    .zip(state.spsr)
    {
        regs.set_spsr(bank, slot);
    }
}

fn hex(value: u32) -> String {
    format!("{value:#010x}")
}

fn push_if_different(out: &mut Vec<FieldDiff>, name: String, expected: u32, actual: u32) {
    if expected != actual {
        out.push(FieldDiff {
            name,
            expected: hex(expected),
            actual: hex(actual),
        });
    }
}

/// Lists how the CPU's registers differ from `want`.
fn register_diffs<const ARM9: bool>(cpu: &Cpu<ARM9>, want: &State) -> Vec<FieldDiff> {
    let regs = cpu.regs();
    let mut out = Vec::new();
    for (i, &value) in want.r.iter().enumerate() {
        push_if_different(
            &mut out,
            format!("R{i}"),
            value,
            regs.banked(Bank::UserSystem, i),
        );
    }
    for (bank, name, first, count) in BANKED {
        for (i, &value) in bank_values(want, bank).iter().enumerate().take(count) {
            push_if_different(
                &mut out,
                format!("{name}[{i}]"),
                value,
                regs.banked(bank, first + i),
            );
        }
    }
    push_if_different(&mut out, "CPSR".to_string(), want.cpsr, regs.cpsr());
    for (i, (&expected, actual)) in want.pipeline.iter().zip(cpu.pipeline()).enumerate() {
        push_if_different(&mut out, format!("pipeline[{i}]"), expected, actual);
    }
    for (i, (bank, &value)) in [
        Bank::Fiq,
        Bank::Supervisor,
        Bank::Abort,
        Bank::Irq,
        Bank::Undefined,
    ]
    .into_iter()
    .zip(&want.spsr)
    .enumerate()
    {
        push_if_different(
            &mut out,
            format!("SPSR[{i}]"),
            value,
            regs.spsr(bank).unwrap_or(0),
        );
    }
    out
}

fn describe(a: &Access) -> String {
    let kind = match a.kind {
        KIND_FETCH => "fetch",
        KIND_READ => "read",
        _ => "write",
    };
    format!(
        "{kind} size={} addr={} data={}",
        a.size,
        hex(a.addr),
        hex(a.data)
    )
}

/// Lists how the accesses the CPU made, fetches included, differ from the case's transactions.
fn transaction_diffs(case: &Case, log: &[Access]) -> Vec<FieldDiff> {
    let want: Vec<Access> = case
        .transactions
        .iter()
        .map(|t| Access {
            kind: t.kind,
            size: t.size,
            addr: t.addr,
            data: t.data,
        })
        .collect();
    let mut out = Vec::new();
    for i in 0..want.len().max(log.len()) {
        let (expected, actual) = (want.get(i), log.get(i));
        if expected != actual {
            out.push(FieldDiff {
                name: format!("transaction[{i}]"),
                expected: expected.map_or_else(|| "none".to_string(), describe),
                actual: actual.map_or_else(|| "none".to_string(), describe),
            });
        }
    }
    out
}

/// Runs one case through `step`: loads the initial state, lets `step` act on the CPU and the
/// replay bus, and lists every difference from the expectation. An empty list means the case
/// passed. Harness tests pass their own closure, so they do not depend on what
/// [`Cpu::step`] executes.
pub fn run_case_with<const ARM9: bool>(
    case: &Case,
    step: impl FnOnce(&mut Cpu<ARM9>, &mut ReplayBus),
) -> Vec<FieldDiff> {
    let mut cpu = Cpu::<ARM9>::new();
    load(&mut cpu, &case.initial);
    let mut bus = ReplayBus::new(case);
    step(&mut cpu, &mut bus);
    let mut diffs = register_diffs(&cpu, &case.expected);
    diffs.extend(transaction_diffs(case, &bus.log));
    diffs
}

/// Runs one case with [`Cpu::step`] as the step.
pub fn run_case<const ARM9: bool>(case: &Case) -> Vec<FieldDiff> {
    run_case_with::<ARM9>(case, |cpu, bus| {
        let _ = cpu.step(bus);
    })
}

/// What one case does between loading and comparing; [`Cpu::step`] outside of tests.
type StepFn<const ARM9: bool> = fn(&mut Cpu<ARM9>, &mut ReplayBus);

fn step_cpu<const ARM9: bool>(cpu: &mut Cpu<ARM9>, bus: &mut ReplayBus) {
    let _ = cpu.step(bus);
}

/// Runs every case in decoded `cases`; `file` only labels the diffs.
pub fn run_cases<const ARM9: bool>(file: &str, cases: &[Case]) -> Vec<CaseDiff> {
    run_cases_with::<ARM9>(file, cases, step_cpu::<ARM9>)
}

/// [`run_cases`] with the step chosen by the caller, so tests of the file and directory
/// plumbing do not depend on what [`Cpu::step`] executes.
pub(crate) fn run_cases_with<const ARM9: bool>(
    file: &str,
    cases: &[Case],
    step: StepFn<ARM9>,
) -> Vec<CaseDiff> {
    cases
        .iter()
        .enumerate()
        .filter_map(|(index, case)| {
            let fields = run_case_with::<ARM9>(case, step);
            (!fields.is_empty()).then(|| CaseDiff {
                file: file.to_string(),
                index,
                opcode: case.opcode,
                fields,
            })
        })
        .collect()
}

/// Reads one suite file and returns the cases that differ.
pub fn run_json_suite<const ARM9: bool>(path: &Path) -> Result<Vec<CaseDiff>, SuiteError> {
    run_json_suite_with::<ARM9>(path, step_cpu::<ARM9>)
}

pub(crate) fn run_json_suite_with<const ARM9: bool>(
    path: &Path,
    step: StepFn<ARM9>,
) -> Result<Vec<CaseDiff>, SuiteError> {
    let bytes = std::fs::read(path).map_err(|source| SuiteError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Ok(run_cases_with::<ARM9>(&file, &decode(&bytes)?, step))
}

/// Pass and fail counts for one suite file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSummary {
    /// File name.
    pub file: String,
    /// Cases in the file.
    pub cases: usize,
    /// Cases that differed.
    pub failed: usize,
}

impl fmt::Display for FileSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} cases, {} failed",
            self.file, self.cases, self.failed
        )
    }
}

/// Runs every `*.json.bin` file in `dir`, in file-name order, as the ARM7TDMI.
pub fn summarize_dir(dir: &Path) -> Result<Vec<FileSummary>, SuiteError> {
    summarize_dir_with(dir, step_cpu::<false>)
}

pub(crate) fn summarize_dir_with(
    dir: &Path,
    step: StepFn<false>,
) -> Result<Vec<FileSummary>, SuiteError> {
    let io = |source| SuiteError::Io {
        path: dir.to_path_buf(),
        source,
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(io)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".json.bin"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let bytes = std::fs::read(&path).map_err(|source| SuiteError::Io {
            path: path.clone(),
            source,
        })?;
        let cases = decode(&bytes)?;
        out.push(FileSummary {
            file: path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            cases: cases.len(),
            failed: run_cases_with::<false>("", &cases, step).len(),
        });
    }
    Ok(out)
}

/// The text the `cpu-suite` command prints: one line per file and a total.
pub fn report_dir(dir: &Path) -> Result<String, SuiteError> {
    report_dir_with(dir, step_cpu::<false>)
}

pub(crate) fn report_dir_with(dir: &Path, step: StepFn<false>) -> Result<String, SuiteError> {
    let files = summarize_dir_with(dir, step)?;
    let cases: usize = files.iter().map(|f| f.cases).sum();
    let failed: usize = files.iter().map(|f| f.failed).sum();
    let mut lines: Vec<String> = files.iter().map(ToString::to_string).collect();
    lines.push(format!("total: {cases} cases, {failed} failed"));
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests;
