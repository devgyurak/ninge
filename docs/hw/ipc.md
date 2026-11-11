# Inter-process communication (IPCSYNC, IPC FIFO)

Sources: GBATEK "DS Inter Process Communication (IPC)" (IPCSYNC, IPCFIFOCNT, IPCFIFOSEND,
IPCFIFORECV, "IPCFIFO Notes"); GBATEK "DS Interrupts" (the IF bit table, bits 16-18).
Used by: `crates/core/src/io/ipc/mod.rs`, `crates/core/src/io/mod.rs`, `crates/core/src/io/irq/mod.rs`   Phase: 3 (#24)

The two CPUs talk through three things that both can reach at the same addresses: a 4-bit
mailbox each way (IPCSYNC) and two 16-word FIFOs, one for each direction. Each CPU sees the same
registers, but they show that CPU's own point of view: what the ARM9 reads as its "send FIFO" is
what the ARM7 reads as its "receive FIFO", and the other way round. The sources of IF bits 16-18
live here; the controller that latches them is in `interrupts.md`.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0180h` | IPCSYNC | 16 | R/W | not stated, 0 here |
| `0400_0184h` | IPCFIFOCNT | 16 | R/W | not stated, see "Unverified" |
| `0400_0188h` | IPCFIFOSEND | 32 | W | no state |
| `0410_0000h` | IPCFIFORECV | 32 | R | no state |

All four exist on both CPUs. The state behind them is per CPU (its own half of IPCSYNC, its own
control bits) or per direction (a FIFO).

### IPCSYNC bits

| Bits | Access | Meaning |
|---|---|---|
| 0-3 | R | Data input: bits 8-11 of the other CPU's IPCSYNC |
| 4-7 | - | Not used |
| 8-11 | R/W | Data output: shows up in bits 0-3 of the other CPU's IPCSYNC |
| 12 | - | Not used |
| 13 | W | Send IRQ to the other CPU (0 = nothing, 1 = send) |
| 14 | R/W | Enable IRQ from the other CPU (0 = disable, 1 = enable) |
| 15-31 | - | Not used |

### IPCFIFOCNT bits

| Bits | Access | Meaning |
|---|---|---|
| 0 | R | Send FIFO empty (1 = empty) |
| 1 | R | Send FIFO full (1 = full) |
| 2 | R/W | Send FIFO empty IRQ enable |
| 3 | W | Send FIFO clear (1 = flush the send FIFO) |
| 4-7 | - | Not used |
| 8 | R | Receive FIFO empty (1 = empty) |
| 9 | R | Receive FIFO full (1 = full) |
| 10 | R/W | Receive FIFO not-empty IRQ enable |
| 11-13 | - | Not used |
| 14 | R/W | Error: a read of an empty receive FIFO or a send to a full send FIFO (1 = error; write 1 to acknowledge) |
| 15 | R/W | Enable send/receive FIFO |
| 16-31 | - | Not used |

### IPCFIFOSEND and IPCFIFORECV

A 32-bit word each. Each FIFO holds at most 16 words (64 bytes). A write to IPCFIFOSEND appends
a word to the writer's send FIFO; a read of IPCFIFORECV takes the oldest word out of the
reader's receive FIFO. The ARM9's send FIFO is the ARM7's receive FIFO, and the ARM7's send
FIFO is the ARM9's receive FIFO.

## Behavior

**IPCSYNC.** A CPU's write to bits 8-11 stores a 4-bit value that the other CPU reads in bits
0-3 of its own IPCSYNC. A CPU reads its own output in bits 8-11, the other's output in bits 0-3,
its own IRQ enable in bit 14, and 0 in bit 13 (write-only). A write of 1 to bit 13 sends an IRQ
to the other CPU: bit 16 of that CPU's IF is set, provided that CPU has bit 14 of its own IPCSYNC
set (see "Unverified" for what happens when it does not). A write of 0 to bit 13 does nothing.
The two halves are separate state: one CPU's write never changes the other CPU's output nibble
or enable.

**Sending.** With the FIFO enabled (IPCFIFOCNT bit 15 of the sender) and room in the send FIFO,
a word written to IPCFIFOSEND goes to the end of the send FIFO. When the send FIFO already holds
16 words, the write sets the sender's error bit (IPCFIFOCNT bit 14); GBATEK does not say what
happens to the word, which this model drops (see "Unverified").

**Receiving.** With the FIFO enabled and the receive FIFO not empty, a read of IPCFIFORECV
returns the oldest word and removes it. When the receive FIFO is empty, the read returns the
most recently received word (0 if nothing was ever received, or the FIFO was flushed with
IPCFIFOCNT bit 3), does not change the FIFO, and sets the reader's error bit.

**Clear.** Writing 1 to IPCFIFOCNT bit 3 empties the writer's send FIFO, which is the other
CPU's receive FIFO, and forgets the "most recently received word" of that FIFO (it reads as 0
from then on). Bit 3 reads as 0.

**Error.** The error bit is set by the two conditions above and stays set until the CPU writes 1
to IPCFIFOCNT bit 14. A write of 0 to bit 14 leaves it unchanged. It is per CPU: the ARM7's
overflow sets the ARM7's bit.

**Disabled FIFO** (IPCFIFOCNT bit 15 of the accessing CPU = 0). A write to IPCFIFOSEND is
ignored: no word is stored and no error is set. A read of IPCFIFORECV returns the oldest word as
usual but does not remove it. The empty/full flags still show the FIFO's real state.

**Status flags.** Bit 0 and bit 1 describe the CPU's send FIFO (0 words / 16 words), bits 8 and
9 its receive FIFO. They are derived from the FIFO, not stored.

**Interrupts.** IF bit 16 is the IPCSYNC IRQ (above). IF bit 17 ("send FIFO empty") and IF bit
18 ("receive FIFO not empty") are **edge triggered** on the CPU's own conditions:

- IF.17 is set when `(IPCFIFOCNT.2 AND IPCFIFOCNT.0)` changes from 0 to 1;
- IF.18 is set when `(IPCFIFOCNT.10 AND NOT IPCFIFOCNT.8)` changes from 0 to 1.

Each condition is the CPU's own: it moves when the FIFO changes (a send, a receive, a clear by
either CPU) or when the CPU changes its own enable bit. A condition that is already 1 does not
set the flag again; the flag can be acknowledged (a 1 written to the IF bit) while the condition
stays true, and it is not set again until the condition has gone 0 and come back. Reading the
formula as written, switching an enable bit on while the FIFO already is in the matching state
is a 0-to-1 change and sets the flag.

**Wider and narrower accesses.** IPCSYNC and IPCFIFOCNT are 16-bit registers: an 8-bit access
sees one byte of the value (little-endian) and an 8-bit write acts on only the bits in that
byte. A 32-bit access at the register's address also reaches the neighbouring byte addresses,
which no register holds (they read 0 and ignore writes). IPCFIFOSEND and IPCFIFORECV are 32-bit
and are used as a whole: one word write sends exactly one word, and one word read receives
exactly one word.

## Edge cases

- Send to a full FIFO: error bit set, FIFO unchanged (16 words).
- Receive from an empty FIFO: error bit set, the last received word (or 0) comes back, FIFO
  unchanged.
- The FIFOs are independent: a send by the ARM9 never fills the ARM9's own receive FIFO.
- A flush by the sender empties the receiver's FIFO, including the words the receiver has not
  read yet.
- Both CPUs write IPCSYNC bit 13 in the same instant: each sends its own IRQ; there is no
  conflict.
- IF.16-18 are latched in IF whatever IME and IE say (`interrupts.md`); only IPCSYNC bit 14
  gates IF.16, and the two IPCFIFOCNT enable bits gate IF.17 and IF.18 through their conditions.

## Unverified

Each item carries `TODO(hw-unverified)` at the named site; verify with a test ROM on hardware,
or a trace against the reference oracle (AGENTS.md §6).

- A send to a full FIFO drops the word — `crates/core/src/io/ipc/mod.rs`, the send path. GBATEK names
  the error and says nothing about the data.
- IPCSYNC bit 13 sent while the other CPU's bit 14 is 0: nothing is latched, the request is
  lost — `crates/core/src/io/ipc/mod.rs`, the IPCSYNC write. GBATEK only says "Enable IRQ from remote
  CPU"; the alternatives are latching it for when the enable comes, or latching it in IF
  regardless of the enable.
- Whether switching an IRQ enable (IPCFIFOCNT bit 2 or bit 10) on while the condition already
  holds raises the flag: the model follows GBATEK's formula literally and raises it —
  `crates/core/src/io/ipc/mod.rs`, the edge detection.
- The edge detector is a model of "changes from 0 to 1": it watches the whole condition,
  including the enable bit; a condition that goes 0, 1, 0 and 1 within one write is taken once —
  `crates/core/src/io/ipc/mod.rs`, the edge detection.
- Reset values: IPCSYNC = 0, IPCFIFOCNT = `0101h` (both FIFOs empty, enables 0, no error, FIFO
  disabled) — GBATEK states no reset values; `crates/core/src/io/ipc/mod.rs`, `Ipc::new`.
- 8-bit and 16-bit accesses to IPCFIFOSEND and IPCFIFORECV: the model ignores such a write
  (no word, no error) and answers such a read like a word read (one word received, the low
  bytes of it selected by the address) — `crates/core/src/io/mod.rs`, the IPC slow path.
- A read of IPCFIFORECV with the FIFO disabled and nothing in it: the model returns the last
  received word without setting the error bit — `crates/core/src/io/ipc/mod.rs`, the receive path.
- Whether IPCFIFORECV is mirrored over `0410_0000h`-`041F_FFFFh`: the model has it at
  `0410_0000h` only — `crates/core/src/io/mod.rs`.
- IPCFIFOCNT bit 3 is a write-only strobe and reads 0; bit 14 acknowledges on a 1 — taken from
  the access column of GBATEK's table, whose "R/W" for bit 14 is ambiguous —
  `crates/core/src/io/ipc/mod.rs`, the IPCFIFOCNT write.
- Timing: a send is visible to the other CPU at once, with no transfer delay —
  `crates/core/src/io/ipc/mod.rs`; approximate timing is the goal (AGENTS.md §2 rule 7).
