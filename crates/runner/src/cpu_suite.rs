// SPDX-License-Identifier: Apache-2.0
//! The SingleStepTests/ARM7TDMI `.json.bin` suite: a reader, a replaying bus, and a harness
//! that runs every case through a [`Cpu`] and reports what differs (#7, AGENTS.md §5.1).
//!
//! The suite is fetched by `scripts/fetch_cpu_suite.sh` into `tests/data/` and never
//! vendored. Each file holds many cases; a case is one instruction with the CPU state before
//! and after and the memory accesses in between.
//!
//! The harness compares R0–R15, every register bank, CPSR, the five SPSRs, and the data
//! transactions (kind, size, address, data, order). It does not compare `cycle` (the suite
//! marks it experimental), `pipeline`, fetch transactions, or `access`: those depend on how the
//! CPU models its prefetch pipeline, which is decided in #11. Decided in #76.

use std::fmt;
use std::path::{Path, PathBuf};

use ninge_core::cpu::{Bank, Bus, Cpu};

/// First word of every `.json.bin` file.
const MAGIC: u32 = 0xD33D_BAE0;
/// Second word of a transactions block.
const TRANSACTIONS_MAGIC: u32 = 3;
/// Registers, banked registers, CPSR, SPSRs, pipeline and access in a state block.
const STATE_WORDS: usize = 40;
/// A state block is its size, one word the converter skips, then [`STATE_WORDS`] words.
const STATE_BLOCK_MIN: usize = 8 + 4 * STATE_WORDS;
/// A transactions block is its size, the magic, and a count; entries follow.
const TRANSACTIONS_BLOCK_MIN: usize = 12;
/// An opcode block is its size, one skipped word, the opcode, and the base address.
const OPCODE_BLOCK_MIN: usize = 16;
/// Bytes of one transaction: kind, size, address, data, cycle, access.
const TRANSACTION_BYTES: usize = 24;

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
    /// R0–R15 as the current mode sees them.
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
    /// The two prefetched opcodes. Not compared; see the module docs.
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
            *slot = self.at(offset + 4 * i)?;
        }
        Ok(out)
    }

    /// The size of the block at `offset`, checked against the least it can hold.
    fn block_size(&self, offset: usize, least: usize) -> Result<usize, SuiteError> {
        let size = usize::try_from(self.at(offset)?).unwrap_or(usize::MAX);
        match size >= least {
            true => Ok(size),
            false => Err(format_error(
                offset,
                "a block is smaller than its fixed part",
            )),
        }
    }
}

fn take<const N: usize>(words: &[u32; STATE_WORDS], start: usize) -> [u32; N] {
    let mut out = [0; N];
    out.copy_from_slice(&words[start..start + N]);
    out
}

fn decode_state(words: &Words, offset: usize) -> Result<(State, usize), SuiteError> {
    let size = words.block_size(offset, STATE_BLOCK_MIN)?;
    let v: [u32; STATE_WORDS] = words.many(offset + 8)?;
    let state = State {
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
    };
    Ok((state, size))
}

fn decode_transactions(
    words: &Words,
    offset: usize,
) -> Result<(Vec<Transaction>, usize), SuiteError> {
    let size = words.block_size(offset, TRANSACTIONS_BLOCK_MIN)?;
    if words.at(offset + 4)? != TRANSACTIONS_MAGIC {
        return Err(format_error(
            offset + 4,
            "a transactions block has the wrong magic",
        ));
    }
    let count = usize::try_from(words.at(offset + 8)?).unwrap_or(usize::MAX);
    let mut list = Vec::new();
    for i in 0..count {
        let at = offset + TRANSACTIONS_BLOCK_MIN + i * TRANSACTION_BYTES;
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

fn decode_case(words: &Words, offset: usize) -> Result<(Case, usize), SuiteError> {
    let size = words.block_size(offset, 4)?;
    let mut at = offset + 4;
    let (initial, n) = decode_state(words, at)?;
    at += n;
    let (expected, n) = decode_state(words, at)?;
    at += n;
    let (transactions, n) = decode_transactions(words, at)?;
    at += n;
    words.block_size(at, OPCODE_BLOCK_MIN)?;
    let (opcode, base_addr) = (words.at(at + 8)?, words.at(at + 12)?);
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
/// Malformed or truncated data is an error, never a panic.
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
    let mut offset = 8;
    for _ in 0..count {
        let (case, size) = decode_case(&words, offset)?;
        cases.push(case);
        offset = offset
            .checked_add(size)
            .ok_or_else(|| format_error(offset, "a case size overflows"))?;
    }
    Ok(cases)
}

/// Where the fetched suite lives; for in-repo tests and the runner binary only.
pub fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/arm7tdmi/v1")
}

/// One data access the CPU made, in the order it made it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Access {
    kind: u32,
    size: u32,
    addr: u32,
    data: u32,
}

/// A bus that replays one case: reads come from the case's recorded reads, fetches from its
/// opcode, and every data access is logged for comparison.
struct ReplayBus {
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
    /// suite's README prescribes.
    fn fetch(&self, addr: u32) -> u32 {
        match addr == self.base_addr {
            true => self.opcode,
            false => addr,
        }
    }
}

impl Bus for ReplayBus {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.fetch(addr)
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.fetch(addr) as u16 // truncation intended: a Thumb opcode is the low halfword
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

/// Puts the CPU into `state`.
fn load<const ARM9: bool>(cpu: &mut Cpu<ARM9>, state: &State) {
    let regs = cpu.regs_mut();
    regs.set_cpsr(state.cpsr);
    for (bank, _, first, count) in BANKED {
        for (i, &value) in bank_values(state, bank).iter().enumerate().take(count) {
            regs.set_banked(bank, first + i, value);
        }
    }
    // The visible registers are the truth for the active bank, so they go in last.
    for (i, &value) in state.r.iter().enumerate() {
        regs.set(i, value);
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
        push_if_different(&mut out, format!("R{i}"), value, regs.get(i));
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

/// Lists how the data accesses the CPU made differ from the case's data transactions.
fn transaction_diffs(case: &Case, log: &[Access]) -> Vec<FieldDiff> {
    let want: Vec<Access> = case
        .transactions
        .iter()
        .filter(|t| t.kind == KIND_READ || t.kind == KIND_WRITE)
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

/// Runs one case: loads the initial state, executes one instruction, and lists every
/// difference from the expectation. An empty list means the case passed.
pub fn run_case<const ARM9: bool>(case: &Case) -> Vec<FieldDiff> {
    let mut cpu = Cpu::<ARM9>::new();
    load(&mut cpu, &case.initial);
    let mut bus = ReplayBus::new(case);
    cpu.step(&mut bus);
    let mut diffs = register_diffs(&cpu, &case.expected);
    diffs.extend(transaction_diffs(case, &bus.log));
    diffs
}

/// Runs every case in decoded `cases`; `file` only labels the diffs.
pub fn run_cases<const ARM9: bool>(file: &str, cases: &[Case]) -> Vec<CaseDiff> {
    cases
        .iter()
        .enumerate()
        .filter_map(|(index, case)| {
            let fields = run_case::<ARM9>(case);
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
    let bytes = std::fs::read(path).map_err(|source| SuiteError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Ok(run_cases::<ARM9>(&file, &decode(&bytes)?))
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
            failed: run_cases::<false>("", &cases).len(),
        });
    }
    Ok(out)
}

/// The text the `cpu-suite` command prints: one line per file and a total.
pub fn report_dir(dir: &Path) -> Result<String, SuiteError> {
    let files = summarize_dir(dir)?;
    let cases: usize = files.iter().map(|f| f.cases).sum();
    let failed: usize = files.iter().map(|f| f.failed).sum();
    let mut lines: Vec<String> = files.iter().map(ToString::to_string).collect();
    lines.push(format!("total: {cases} cases, {failed} failed"));
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests;
