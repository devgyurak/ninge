// SPDX-License-Identifier: Apache-2.0
//! An in-memory GDB stub: a `Read + Write` that parses the client's `$...#xx` frames, acknowledges
//! them, records every payload, and answers from a scripted CPU. It follows the protocol as the
//! public description gives it: a `-` from the client makes the stub send its last reply again.
//!
//! The stub's own framing is written out here, not borrowed from `crate::rsp`, so a bug in the
//! packet layer does not cancel itself against the fake.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Read, Write};
use std::rc::Rc;

use ninge_core::trace::{Opcode, TraceRecord};

/// What the stub saw, shared with the test that built it.
#[derive(Debug, Default)]
pub struct Log {
    /// Every request payload, in order, as text.
    pub packets: Vec<String>,
    /// Every acknowledgement byte the client sent outside a packet.
    pub acks: Vec<u8>,
}

pub type SharedLog = Rc<RefCell<Log>>;

/// One state of the simulated CPU.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub regs: [u32; 16],
    pub cpsr: u32,
    pub memory: BTreeMap<u32, u8>,
}

impl State {
    /// A state with its pc (r15) at `pc`, ARM mode, nothing in memory.
    pub fn at(pc: u32) -> Self {
        let mut state = Self::default();
        state.regs[15] = pc;
        state.cpsr = 0x0000_001F;
        state
    }

    pub fn reg(mut self, index: usize, value: u32) -> Self {
        self.regs[index] = value;
        self
    }

    /// An ARM opcode (little-endian) at the pc.
    pub fn arm(mut self, opcode: u32) -> Self {
        self.cpsr &= !(1 << 5);
        self.put(self.regs[15], &opcode.to_le_bytes());
        self
    }

    /// A Thumb opcode at the pc, with CPSR bit 5 set.
    pub fn thumb(mut self, opcode: u16) -> Self {
        self.cpsr |= 1 << 5;
        self.put(self.regs[15], &opcode.to_le_bytes());
        self
    }

    /// The state a trace record describes: its registers and CPSR, and its opcode's bytes at the
    /// pc (so the adapter reads the same instruction back).
    pub fn from_record(record: &TraceRecord) -> Self {
        let mut state = Self {
            regs: record.regs,
            cpsr: record.cpsr,
            memory: BTreeMap::new(),
        };
        match record.opcode {
            Opcode::Arm(word) => state.put(record.pc(), &word.to_le_bytes()),
            Opcode::Thumb(half) => state.put(record.pc(), &half.to_le_bytes()),
        }
        state
    }

    pub fn put(&mut self, addr: u32, bytes: &[u8]) {
        bytes.iter().enumerate().for_each(|(offset, byte)| {
            self.memory.insert(addr + offset as u32, *byte);
        });
    }
}

/// How the stub answers requests it has no override for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Answer from the script.
    Simulate,
    /// Answer every request with an empty packet: "not supported".
    Empty,
    /// Answer every request with `E01`.
    Error,
    /// Send nothing at all, as a stub that went away.
    Silent,
}

#[derive(Debug)]
pub struct Fake {
    script: Vec<State>,
    at: usize,
    exited: bool,
    /// The addresses `Z0` set and `z0` has not removed.
    breakpoints: BTreeSet<u32>,
    /// Where the CPSR sits in the `g` block, in words.
    cpsr_word: usize,
    mode: Mode,
    /// The reply data for requests starting with a byte, ahead of the simulation.
    overrides: BTreeMap<u8, Vec<u8>>,
    /// The next this-many requests are answered with `-` and dropped.
    nak_requests: usize,
    /// The next this-many replies carry a wrong checksum (a retransmission is not counted).
    corrupt_replies: usize,
    /// Writes fail with a broken pipe.
    fail_writes: bool,
    pending: Vec<u8>,
    outbox: VecDeque<u8>,
    last_reply: Vec<u8>,
    log: SharedLog,
}

impl Fake {
    /// A stub over `script`, and the log it writes.
    pub fn new(script: Vec<State>) -> (Self, SharedLog) {
        let log = SharedLog::default();
        let fake = Self {
            script,
            at: 0,
            exited: false,
            breakpoints: BTreeSet::new(),
            cpsr_word: 16,
            mode: Mode::Simulate,
            overrides: BTreeMap::new(),
            nak_requests: 0,
            corrupt_replies: 0,
            fail_writes: false,
            pending: Vec::new(),
            outbox: VecDeque::new(),
            last_reply: Vec::new(),
            log: Rc::clone(&log),
        };
        (fake, log)
    }

    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    pub fn cpsr_word(mut self, word: usize) -> Self {
        self.cpsr_word = word;
        self
    }

    /// Answers requests starting with `command` with `reply` (raw wire data, not escaped).
    pub fn reply(mut self, command: u8, reply: &[u8]) -> Self {
        self.overrides.insert(command, reply.to_vec());
        self
    }

    pub fn nak_requests(mut self, count: usize) -> Self {
        self.nak_requests = count;
        self
    }

    pub fn corrupt_replies(mut self, count: usize) -> Self {
        self.corrupt_replies = count;
        self
    }

    pub fn fail_writes(mut self) -> Self {
        self.fail_writes = true;
        self
    }

    fn current(&self) -> State {
        self.script.get(self.at).cloned().unwrap_or_default()
    }

    /// The `g` reply: r0..r15, zero padding, the CPSR at its word, little-endian hex.
    fn registers(&self) -> Vec<u8> {
        let state = self.current();
        let mut words = vec![0u32; self.cpsr_word.max(15) + 1];
        words[..16].copy_from_slice(&state.regs);
        words[self.cpsr_word] = state.cpsr;
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        hex(&bytes).into_bytes()
    }

    /// The `m<addr>,<len>` reply; `E14` for a byte nothing is mapped at.
    fn memory(&self, args: &str) -> Vec<u8> {
        let parsed = args.split_once(',').and_then(|(addr, len)| {
            Some((
                u32::from_str_radix(addr, 16).ok()?,
                u32::from_str_radix(len, 16).ok()?,
            ))
        });
        let Some((addr, len)) = parsed else {
            return b"E01".to_vec();
        };
        let state = self.current();
        let bytes: Option<Vec<u8>> = (0..len)
            .map(|offset| state.memory.get(&(addr + offset)).copied())
            .collect();
        match bytes {
            Some(bytes) => hex(&bytes).into_bytes(),
            None => b"E14".to_vec(),
        }
    }

    fn step(&mut self) -> Vec<u8> {
        match self.at + 1 < self.script.len() {
            true => {
                self.at += 1;
                b"S05".to_vec()
            }
            false => {
                self.exited = true;
                b"W00".to_vec()
            }
        }
    }

    /// Runs on to the next state after the current one whose r15 is a breakpoint address, or to
    /// the last state of the script when there is none.
    fn resume(&mut self) -> Vec<u8> {
        let last = self.script.len().saturating_sub(1);
        self.at = (self.at + 1..self.script.len())
            .find(|&index| self.breakpoints.contains(&self.script[index].regs[15]))
            .unwrap_or(last);
        b"S05".to_vec()
    }

    /// `Z0,<addr>,<kind>` and `z0,<addr>,<kind>`: records or removes the address.
    fn breakpoint(&mut self, set: bool, args: &str) -> Vec<u8> {
        let addr = args
            .strip_prefix("0,")
            .and_then(|rest| rest.split(',').next())
            .and_then(|addr| u32::from_str_radix(addr, 16).ok());
        match addr {
            Some(addr) => {
                match set {
                    true => self.breakpoints.insert(addr),
                    false => self.breakpoints.remove(&addr),
                };
                b"OK".to_vec()
            }
            None => b"E01".to_vec(),
        }
    }

    fn answer(&mut self, data: &[u8]) -> Vec<u8> {
        let command = data.first().copied().unwrap_or(0);
        if let Some(reply) = self.overrides.get(&command) {
            return reply.clone();
        }
        match self.mode {
            Mode::Empty | Mode::Silent => return Vec::new(),
            Mode::Error => return b"E01".to_vec(),
            Mode::Simulate => {}
        }
        let args = String::from_utf8_lossy(&data[1..]).into_owned();
        match command {
            b'g' => self.registers(),
            b'm' => self.memory(&args),
            b's' => self.step(),
            b'c' => self.resume(),
            b'Z' | b'z' => self.breakpoint(command == b'Z', &args),
            _ => Vec::new(),
        }
    }

    /// Puts `reply` on the wire, with a broken checksum while corruption is scheduled.
    fn emit(&mut self, reply: &[u8]) {
        let mut wire = wire(reply);
        if self.corrupt_replies > 0 {
            self.corrupt_replies -= 1;
            // Break the checksum: flip the last digit.
            let last = wire.len() - 1;
            wire[last] = if wire[last] == b'0' { b'1' } else { b'0' };
        }
        self.outbox.extend(wire);
    }

    fn send(&mut self, reply: Vec<u8>) {
        self.emit(&reply);
        self.last_reply = reply;
    }

    fn handle_packet(&mut self, data: &[u8]) {
        self.log
            .borrow_mut()
            .packets
            .push(String::from_utf8_lossy(data).into_owned());
        if self.mode == Mode::Silent {
            return;
        }
        if self.nak_requests > 0 {
            self.nak_requests -= 1;
            self.outbox.push_back(b'-');
            return;
        }
        self.outbox.push_back(b'+');
        let reply = self.answer(data);
        self.send(reply);
    }

    /// Consumes whole items from `pending`: acks and complete frames.
    fn parse(&mut self) {
        loop {
            match self.pending.first().copied() {
                None => return,
                Some(b'+') => {
                    self.pending.remove(0);
                    self.log.borrow_mut().acks.push(b'+');
                }
                Some(b'-') => {
                    self.pending.remove(0);
                    self.log.borrow_mut().acks.push(b'-');
                    // The protocol: the stub sends its last reply again.
                    let again = self.last_reply.clone();
                    self.emit(&again);
                }
                Some(b'$') => {
                    let Some(hash) = self.pending.iter().position(|byte| *byte == b'#') else {
                        return;
                    };
                    if self.pending.len() < hash + 3 {
                        return;
                    }
                    let data: Vec<u8> = self.pending[1..hash].to_vec();
                    self.pending.drain(..hash + 3);
                    self.handle_packet(&data);
                }
                Some(_) => {
                    self.pending.remove(0);
                }
            }
        }
    }
}

impl Write for Fake {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.fail_writes {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.pending.extend_from_slice(buf);
        self.parse();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Fake {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = buf.len().min(self.outbox.len());
        buf[..count]
            .iter_mut()
            .zip(self.outbox.drain(..count))
            .for_each(|(slot, byte)| *slot = byte);
        Ok(count)
    }
}

/// `data` framed with a correct checksum, verbatim (no escaping: tests that want an escape put
/// it in the data themselves).
fn wire(data: &[u8]) -> Vec<u8> {
    let sum = data.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    let mut out = vec![b'$'];
    out.extend_from_slice(data);
    out.extend(format!("#{sum:02x}").bytes());
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The scripted states of a run: one per record, in order.
pub fn script_of(records: &[TraceRecord]) -> Vec<State> {
    records.iter().map(State::from_record).collect()
}

/// The request payloads that start with `command`.
pub fn requests(log: &SharedLog, command: char) -> Vec<String> {
    log.borrow()
        .packets
        .iter()
        .filter(|packet| packet.starts_with(command))
        .cloned()
        .collect()
}

/// The addresses of every `m` request the stub saw.
pub fn read_addresses(log: &SharedLog) -> Vec<u32> {
    requests(log, 'm')
        .iter()
        .filter_map(|packet| packet[1..].split(',').next())
        .filter_map(|addr| u32::from_str_radix(addr, 16).ok())
        .collect()
}
