// SPDX-License-Identifier: Apache-2.0
//! The two CPUs' mailbox: IPCSYNC and the two IPC FIFOs (`docs/hw/ipc.md`).
//!
//! Both CPUs see the same four addresses, each with its own point of view, so every accessor is
//! generic over which CPU is asking (`ARM9`, as in `Timing`). The module has no way to reach a
//! CPU's interrupt controller: what it would raise comes back as [`Requests`], and `Memory`
//! hands those to the two controllers.

use crate::irq::{IPCSYNC_IRQ, RECV_NOT_EMPTY_IRQ, SEND_EMPTY_IRQ};
use serde::{Deserialize, Serialize};

/// IPCSYNC, at `0400_0180h` (`ipc.md`, "Registers").
pub const IPCSYNC_ADDR: u32 = 0x0400_0180;
/// IPCFIFOCNT, at `0400_0184h` (`ipc.md`, "Registers").
pub const IPCFIFOCNT_ADDR: u32 = 0x0400_0184;
/// IPCFIFOSEND, at `0400_0188h` (`ipc.md`, "Registers").
pub const IPCFIFOSEND_ADDR: u32 = 0x0400_0188;
/// IPCFIFORECV, at `0410_0000h` (`ipc.md`, "Registers").
pub const IPCFIFORECV_ADDR: u32 = 0x0410_0000;

/// Words a FIFO holds (`ipc.md`, "IPCFIFOSEND and IPCFIFORECV").
const FIFO_CAPACITY: u8 = 16;
/// Ring index mask: the capacity is a power of two.
const FIFO_INDEX_MASK: u8 = FIFO_CAPACITY - 1;

/// IPCSYNC, high byte (bits 8-15): the output nibble, the send strobe and the IRQ enable.
const SYNC_OUT_MASK: u8 = 0x0F; // bits 8-11
const SYNC_SEND_IRQ: u8 = 1 << 5; // bit 13
const SYNC_IRQ_ENABLE: u8 = 1 << 6; // bit 14
/// IPCSYNC bit 14 in the 16-bit register.
const SYNC_IRQ_ENABLE_BIT: u16 = 1 << 14;

/// IPCFIFOCNT bits (`ipc.md`, "IPCFIFOCNT bits").
const CNT_SEND_EMPTY: u16 = 1 << 0;
const CNT_SEND_FULL: u16 = 1 << 1;
const CNT_SEND_IRQ_ENABLE: u8 = 1 << 2; // low byte
const CNT_SEND_CLEAR: u8 = 1 << 3; // low byte
const CNT_RECV_EMPTY: u16 = 1 << 8;
const CNT_RECV_FULL: u16 = 1 << 9;
const CNT_RECV_IRQ_ENABLE: u8 = 1 << 2; // bit 10, in the high byte
const CNT_ERROR: u8 = 1 << 6; // bit 14, in the high byte
const CNT_ENABLE: u8 = 1 << 7; // bit 15, in the high byte
const CNT_SEND_IRQ_ENABLE_BIT: u16 = 1 << 2;
const CNT_RECV_IRQ_ENABLE_BIT: u16 = 1 << 10;
const CNT_ERROR_BIT: u16 = 1 << 14;
const CNT_ENABLE_BIT: u16 = 1 << 15;

/// The IF bits an IPC event asks each CPU's controller to latch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Requests {
    pub(crate) irq9: u32,
    pub(crate) irq7: u32,
}

impl Requests {
    fn merge(self, other: Self) -> Self {
        Self {
            irq9: self.irq9 | other.irq9,
            irq7: self.irq7 | other.irq7,
        }
    }
}

/// One direction's FIFO: a ring of 16 words. Plain data (AGENTS.md §2 rule 5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Fifo {
    words: [u32; FIFO_CAPACITY as usize],
    head: u8,
    len: u8,
    // The most recently received word: what an empty read returns. 0 after a flush
    // (`ipc.md`, "Behavior").
    last: u32,
}

impl Fifo {
    fn new() -> Self {
        Self {
            words: [0; FIFO_CAPACITY as usize],
            head: 0,
            len: 0,
            last: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn is_full(&self) -> bool {
        self.len >= FIFO_CAPACITY
    }

    /// The slot `offset` words after the head. The mask keeps a hand-made state in range.
    fn slot(&self, offset: u8) -> usize {
        usize::from(self.head.wrapping_add(offset) & FIFO_INDEX_MASK)
    }

    /// Appends a word; the caller has checked the FIFO is not full.
    fn push(&mut self, word: u32) {
        self.words[self.slot(self.len)] = word;
        self.len += 1;
    }

    /// Removes and returns the oldest word; the caller has checked the FIFO is not empty.
    fn pop(&mut self) -> u32 {
        let word = self.words[self.slot(0)];
        self.head = self.head.wrapping_add(1) & FIFO_INDEX_MASK;
        self.len -= 1;
        self.last = word;
        word
    }

    /// What a read would return without taking anything: the oldest word, or the last one
    /// received when empty.
    fn front(&self) -> u32 {
        match self.is_empty() {
            true => self.last,
            false => self.words[self.slot(0)],
        }
    }

    /// Empties the FIFO and forgets the last word (`ipc.md`, "Clear").
    fn clear(&mut self) {
        *self = Self::new();
    }

    fn is_well_formed(&self) -> bool {
        self.len <= FIFO_CAPACITY && self.head < FIFO_CAPACITY
    }
}

/// One CPU's own half of the registers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Port {
    // IPCSYNC bits 8-11, which the other CPU reads in bits 0-3.
    sync_out: u8,
    // IPCSYNC bit 14.
    sync_irq: bool,
    // IPCFIFOCNT bits 2, 10, 14, 15.
    send_irq: bool,
    recv_irq: bool,
    error: bool,
    enabled: bool,
    // The two IRQ conditions as they were at the last change, for edge detection
    // (`ipc.md`, "Interrupts"). Always equal to the current conditions once an operation ends.
    send_edge: bool,
    recv_edge: bool,
}

impl Port {
    fn new() -> Self {
        Self {
            sync_out: 0,
            sync_irq: false,
            send_irq: false,
            recv_irq: false,
            error: false,
            enabled: false,
            send_edge: false,
            recv_edge: false,
        }
    }

    /// IF.17's condition: `IPCFIFOCNT.2 AND IPCFIFOCNT.0`.
    fn send_condition(&self, send: &Fifo) -> bool {
        self.send_irq && send.is_empty()
    }

    /// IF.18's condition: `IPCFIFOCNT.10 AND NOT IPCFIFOCNT.8`.
    fn recv_condition(&self, recv: &Fifo) -> bool {
        self.recv_irq && !recv.is_empty()
    }

    /// The IF bits for conditions that rose since the last call (`ipc.md`, "Interrupts").
    // TODO(hw-unverified): the edge detector watches the whole condition, enable bit included,
    // so enabling an IRQ while its condition holds raises the flag (`ipc.md`, "Unverified").
    fn edges(&mut self, send: &Fifo, recv: &Fifo) -> u32 {
        let (send_now, recv_now) = (self.send_condition(send), self.recv_condition(recv));
        let mask = match (send_now && !self.send_edge, recv_now && !self.recv_edge) {
            (true, true) => SEND_EMPTY_IRQ | RECV_NOT_EMPTY_IRQ,
            (true, false) => SEND_EMPTY_IRQ,
            (false, true) => RECV_NOT_EMPTY_IRQ,
            (false, false) => 0,
        };
        self.send_edge = send_now;
        self.recv_edge = recv_now;
        mask
    }

    /// IPCFIFOCNT as this CPU reads it.
    fn control(&self, send: &Fifo, recv: &Fifo) -> u16 {
        [
            (send.is_empty(), CNT_SEND_EMPTY),
            (send.is_full(), CNT_SEND_FULL),
            (self.send_irq, CNT_SEND_IRQ_ENABLE_BIT),
            (recv.is_empty(), CNT_RECV_EMPTY),
            (recv.is_full(), CNT_RECV_FULL),
            (self.recv_irq, CNT_RECV_IRQ_ENABLE_BIT),
            (self.error, CNT_ERROR_BIT),
            (self.enabled, CNT_ENABLE_BIT),
        ]
        .into_iter()
        .filter(|&(set, _)| set)
        .fold(0, |value, (_, bit)| value | bit)
    }
}

/// IPCSYNC and the FIFOs of both CPUs. Plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipc {
    port9: Port,
    port7: Port,
    // The ARM9's send FIFO, which is the ARM7's receive FIFO.
    fifo9: Fifo,
    // The ARM7's send FIFO, which is the ARM9's receive FIFO.
    fifo7: Fifo,
}

impl Ipc {
    /// The power-on state: both FIFOs empty and disabled, IPCSYNC 0.
    // TODO(hw-unverified): no reset values are stated (`ipc.md`, "Unverified").
    pub fn new() -> Self {
        Self {
            port9: Port::new(),
            port7: Port::new(),
            fifo9: Fifo::new(),
            fifo7: Fifo::new(),
        }
    }

    /// This CPU's port, its send FIFO and its receive FIFO.
    fn own<const ARM9: bool>(&self) -> (&Port, &Fifo, &Fifo) {
        match ARM9 {
            true => (&self.port9, &self.fifo9, &self.fifo7),
            false => (&self.port7, &self.fifo7, &self.fifo9),
        }
    }

    /// The other CPU's port.
    fn other_port<const ARM9: bool>(&self) -> &Port {
        match ARM9 {
            true => &self.port7,
            false => &self.port9,
        }
    }

    /// The same as `own`, mutable.
    fn own_mut<const ARM9: bool>(&mut self) -> (&mut Port, &mut Fifo, &mut Fifo) {
        match ARM9 {
            true => (&mut self.port9, &mut self.fifo9, &mut self.fifo7),
            false => (&mut self.port7, &mut self.fifo7, &mut self.fifo9),
        }
    }

    /// Latches the IRQ conditions of both CPUs and returns what rose. Run after every change
    /// to a FIFO or an enable bit, so a change by one CPU shows on the other's conditions.
    fn settle(&mut self) -> Requests {
        Requests {
            irq9: self.port9.edges(&self.fifo9, &self.fifo7),
            irq7: self.port7.edges(&self.fifo7, &self.fifo9),
        }
    }

    /// IPCSYNC as this CPU reads it: the other's output in bits 0-3, its own in bits 8-11 and
    /// its IRQ enable in bit 14 (`ipc.md`, "IPCSYNC bits").
    pub fn sync<const ARM9: bool>(&self) -> u16 {
        let (own, ..) = self.own::<ARM9>();
        let other = self.other_port::<ARM9>();
        let enable = match own.sync_irq {
            true => SYNC_IRQ_ENABLE_BIT,
            false => 0,
        };
        u16::from(other.sync_out) | u16::from(own.sync_out) << 8 | enable
    }

    /// IPCFIFOCNT as this CPU reads it (`ipc.md`, "IPCFIFOCNT bits").
    pub fn control<const ARM9: bool>(&self) -> u16 {
        let (port, send, recv) = self.own::<ARM9>();
        port.control(send, recv)
    }

    /// The words in this CPU's send FIFO, oldest first. For the debugger.
    pub fn send_words<const ARM9: bool>(&self) -> impl Iterator<Item = u32> + '_ {
        let (_, send, _) = self.own::<ARM9>();
        (0..send.len.min(FIFO_CAPACITY)).map(move |i| send.words[send.slot(i)])
    }

    /// The byte of IPCSYNC or IPCFIFOCNT at `addr`, if one holds it.
    pub(crate) fn read_byte<const ARM9: bool>(&self, addr: u32) -> Option<u8> {
        let (value, shift) = match addr {
            IPCSYNC_ADDR | 0x0400_0181 => (self.sync::<ARM9>(), (addr - IPCSYNC_ADDR) * 8),
            IPCFIFOCNT_ADDR | 0x0400_0185 => (self.control::<ARM9>(), (addr - IPCFIFOCNT_ADDR) * 8),
            _ => return None,
        };
        Some((value >> shift) as u8)
    }

    /// Writes a byte of IPCSYNC or IPCFIFOCNT; any other address is ignored. Returns what the
    /// write asks the controllers to latch.
    ///
    /// The fields of both registers sit in single bytes, so a wider write is the same as its
    /// bytes in turn: the IRQ conditions can only fall between them, never rise and fall.
    pub(crate) fn write_byte<const ARM9: bool>(&mut self, addr: u32, value: u8) -> Requests {
        let sent = match addr {
            0x0400_0181 => self.write_sync_high::<ARM9>(value),
            IPCFIFOCNT_ADDR => {
                self.write_cnt_low::<ARM9>(value);
                Requests::default()
            }
            0x0400_0185 => {
                self.write_cnt_high::<ARM9>(value);
                Requests::default()
            }
            // IPCSYNC bits 0-7 hold nothing writable.
            _ => return Requests::default(),
        };
        sent.merge(self.settle())
    }

    /// IPCSYNC bits 8-15. Bit 13 sends the IRQ.
    // TODO(hw-unverified): an IRQ sent while the other CPU's bit 14 is 0 is lost, not latched
    // for later (`ipc.md`, "Unverified").
    fn write_sync_high<const ARM9: bool>(&mut self, value: u8) -> Requests {
        let (port, ..) = self.own_mut::<ARM9>();
        port.sync_out = value & SYNC_OUT_MASK;
        port.sync_irq = value & SYNC_IRQ_ENABLE != 0;
        let remote_listens = self.other_port::<ARM9>().sync_irq;
        match (value & SYNC_SEND_IRQ != 0 && remote_listens, ARM9) {
            (true, true) => Requests {
                irq9: 0,
                irq7: IPCSYNC_IRQ,
            },
            (true, false) => Requests {
                irq9: IPCSYNC_IRQ,
                irq7: 0,
            },
            (false, _) => Requests::default(),
        }
    }

    /// IPCFIFOCNT bits 0-7: the send-empty IRQ enable and the clear strobe.
    // TODO(hw-unverified): bit 3 is a write-only strobe and reads 0 (`ipc.md`, "Unverified").
    fn write_cnt_low<const ARM9: bool>(&mut self, value: u8) {
        let (port, send, _) = self.own_mut::<ARM9>();
        port.send_irq = value & CNT_SEND_IRQ_ENABLE != 0;
        // The strobe empties the send FIFO, which is the other CPU's receive FIFO.
        if value & CNT_SEND_CLEAR != 0 {
            send.clear();
        }
    }

    /// IPCFIFOCNT bits 8-15: the receive IRQ enable, the error acknowledge and the enable.
    // TODO(hw-unverified): bit 14 acknowledges on a 1 and a 0 leaves it (`ipc.md`, "Unverified").
    fn write_cnt_high<const ARM9: bool>(&mut self, value: u8) {
        let (port, ..) = self.own_mut::<ARM9>();
        port.recv_irq = value & CNT_RECV_IRQ_ENABLE != 0;
        if value & CNT_ERROR != 0 {
            port.error = false;
        }
        port.enabled = value & CNT_ENABLE != 0;
    }

    /// The word a read of IPCFIFORECV would return, without taking it or raising an error.
    pub(crate) fn peek_recv<const ARM9: bool>(&self) -> u32 {
        self.own::<ARM9>().2.front()
    }

    /// A CPU's read of IPCFIFORECV: returns the word and what the read asks the controllers to
    /// latch (the sender's FIFO may just have become empty).
    // TODO(hw-unverified): a disabled FIFO that is empty answers with the last word and no error
    // (`ipc.md`, "Unverified").
    pub(crate) fn receive<const ARM9: bool>(&mut self) -> (u32, Requests) {
        let (port, _, recv) = self.own_mut::<ARM9>();
        if !port.enabled {
            // A disabled FIFO shows its oldest word and keeps it (`ipc.md`, "Disabled FIFO").
            return (recv.front(), Requests::default());
        }
        if recv.is_empty() {
            port.error = true;
            return (recv.last, Requests::default());
        }
        let word = recv.pop();
        (word, self.settle())
    }

    /// A CPU's write to IPCFIFOSEND; returns what it asks the controllers to latch.
    // TODO(hw-unverified): a send is visible to the other CPU at once, with no transfer delay
    // (`ipc.md`, "Unverified").
    // TODO(hw-unverified): a word sent to a full FIFO is dropped (`ipc.md`, "Unverified").
    pub(crate) fn send<const ARM9: bool>(&mut self, word: u32) -> Requests {
        let (port, send, _) = self.own_mut::<ARM9>();
        if !port.enabled {
            return Requests::default();
        }
        if send.is_full() {
            port.error = true;
            return Requests::default();
        }
        send.push(word);
        self.settle()
    }

    /// Whether the state holds only what the hardware can: FIFO lengths and heads in range, a
    /// 4-bit output nibble, and edge memory that matches the conditions. A savestate that decodes
    /// to anything else is corrupt (`savestate`).
    pub(crate) fn is_well_formed(&self) -> bool {
        let ports = [
            (&self.port9, &self.fifo9, &self.fifo7),
            (&self.port7, &self.fifo7, &self.fifo9),
        ];
        self.fifo9.is_well_formed()
            && self.fifo7.is_well_formed()
            && ports.into_iter().all(|(port, send, recv)| {
                port.sync_out <= SYNC_OUT_MASK
                    && port.send_edge == port.send_condition(send)
                    && port.recv_edge == port.recv_condition(recv)
            })
    }
}

#[cfg(test)]
impl Ipc {
    /// An IPC whose ARM9 send FIFO claims `len` words, to build a corrupt state in a test.
    pub(crate) fn with_corrupt_fifo_len(len: u8) -> Self {
        let mut ipc = Self::new();
        ipc.fifo9.len = len;
        ipc
    }

    /// An IPC whose ARM9 send FIFO head is `head`.
    pub(crate) fn with_corrupt_fifo_head(head: u8) -> Self {
        let mut ipc = Self::new();
        ipc.fifo9.head = head;
        ipc
    }

    /// An IPC whose ARM9 output nibble is `out`, unmasked.
    pub(crate) fn with_corrupt_sync_out(out: u8) -> Self {
        let mut ipc = Self::new();
        ipc.port9.sync_out = out;
        ipc
    }

    /// An IPC whose ARM9 remembers an IF.17 condition that its empty FIFO and disabled IRQ
    /// do not give.
    pub(crate) fn with_corrupt_edge_memory() -> Self {
        let mut ipc = Self::new();
        ipc.port9.send_edge = true;
        ipc
    }
}

impl Default for Ipc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
