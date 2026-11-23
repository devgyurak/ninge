# Cartridge save chips: the AUXSPI bus

Sources: GBATEK "DS Cartridge Backup" (SPI Bus Backup Memory: the chip list, the command
sets, the status register, the detection table); GBATEK "DS Cartridge I/O Ports" (AUXSPICNT,
AUXSPIDATA); GBATEK "DS Firmware Serial Flash Memory" (the FLASH instruction set and the chip ID
table, which the backup FLASH shares); GBATEK "DS Memory Control - Cartridges and Main RAM"
(EXMEMCNT, the slot owner); GBATEK "DS Technical Data" (the bus clock).
Used by: `crates/core/src/cart/save/`, `crates/core/src/io/cart/`   Phase: 5 (#38)

The cartridge slot carries a second, serial bus next to the parallel ROM bus: AUXSPI. It talks to
the save chip on the card, one byte per write to AUXSPIDATA. The same registers and the same slot
owner as the ROM side (`cart-protocol.md`) apply; this note covers what is new: the SPI side of
AUXSPICNT, AUXSPIDATA, and the chips behind them.

## Scope

Implemented: the three chip families a card uses for saves (EEPROM, FLASH, FRAM) at the sizes
GBATEK lists, their command sets, status registers and memory, the SPI session (chip select, hold,
busy), and the choice of chip type (see "Model (not hardware)").

Not implemented, and not planned for this feature:

- the infrared-sharing cards, where FLASH access takes a `00h` prefix byte and a slower clock;
- NAND saves (a writable part of the ROM chip) and everything specific to the DSi;
- the exact behavior of FLASH deep power-down (the commands are accepted, see "FLASH").

## Registers

Both CPUs see these; only the owner of the slot (EXMEMCNT bit 11) reaches them. For the other CPU
they read zero and writes are ignored, the same rule as the ROM side (`cart-protocol.md`,
"Ownership").

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_01A0 | AUXSPICNT | 16 | R/W (bit 7 read-only) | 0 |
| 0x0400_01A2 | AUXSPIDATA | 16 | R/W | 0 (our choice, see "Unverified") |

### AUXSPICNT bits (the SPI side)

Bits 13-15 are shared with the ROM side and keep the meaning in `cart-protocol.md`.

| Bits | Meaning |
|---|---|
| 0-1 | SPI baud rate: 0 = 4 MHz, 1 = 2 MHz, 2 = 1 MHz, 3 = 512 kHz |
| 2-5 | Not used, zero |
| 6 | Hold chip select: 0 = deselect after this transfer, 1 = keep selected |
| 7 | SPI busy: 1 while a byte is being exchanged. Read-only: a write does not change it |
| 8-12 | Not used, zero |
| 13 | Slot mode: 0 = ROM (parallel), 1 = SPI backup (serial) |
| 14 | Transfer-ready IRQ. For the ROM side only; AUXSPI has no interrupt |
| 15 | Slot enable (both sides) |

### AUXSPIDATA bits

| Bits | Meaning |
|---|---|
| 0-7 | Write: the byte to send, which starts a transfer. Read: the byte received by the last transfer |
| 8-15 | Not used, read as zero |

## Behavior

### A transfer

A write to AUXSPIDATA while the slot mode is SPI (bit 13 = 1) and the slot is enabled (bit 15 = 1)
exchanges one byte with the chip: the written byte goes out, and a byte comes back at the same
time. Software that only wants to read still has to write a dummy byte (zero) to clock the byte in.
The received byte is then what AUXSPIDATA reads.

While the byte is on the wire, AUXSPICNT bit 7 reads 1. The byte takes 8 bits, and a bit takes
8, 16, 32 or 64 bus clocks for baud rates 0-3 (GBATEK gives the rates as 4 MHz, 2 MHz, 1 MHz and
512 kHz, which are the 33.51 MHz bus clock divided by 8, 16, 32 and 64). In master cycles (two
per bus clock, `scheduler.md`) a byte takes **128, 256, 512 or 1024 master cycles**. The busy bit
is computed from the clock when it is read: it is 1 until the byte's finishing time and 0
after. Nothing in this feature is an event: there is no interrupt and no DMA on this bus.

### Chip select and hold

The chip is selected for as long as a *session* lasts. The first transfer after the chip was
deselected starts a session, and its byte is the chip's command. Every later transfer of the
session is a parameter or data byte.

The hold bit (bit 6), as it stands **when the transfer starts**, decides whether the session
continues. With hold = 1 the chip stays selected, and the next transfer belongs to the same
session. With hold = 0 the chip is deselected when this transfer completes, and the session ends.
That is why software clears hold *before* the last byte of a command (GBATEK, AUXSPICNT).

A command's effect on the chip's registers (WEL, the write-protect bits) happens when the session
ends. A byte of data is written to memory as soon as it arrives.

### The command bytes

All values are most significant byte first. The "address" of a command is 1, 2 or 3 bytes, by
chip (below). "Page" is the chip's page size. A write command needs the write-enable latch (WEL,
set by WREN): without it, the command's data is ignored.

#### Status register

| Bit | Meaning |
|---|---|
| 0 | WIP, write in progress. Always 0 here (see "Model (not hardware)") |
| 1 | WEL, write enable latch. Set by WREN, cleared by WRDI and when a session that wrote or erased ends |
| 2-3 | WP, write protect: 0 = none, 1 = upper quarter, 2 = upper half, 3 = all memory (EEPROM, FRAM) |
| 4-7 | 0.5K EEPROM: always 1. 8K-128K EEPROM and FRAM: bits 4-6 zero, bit 7 SRWD (stored; it only has an effect while the chip's write-protect pin is low, and the card wires that pin high). FLASH: zero |

WRSR writes only the WP bits and SRWD; it is a no-op without WEL, and clears WEL.

#### EEPROM and FRAM

| Command | Name | Bytes after it |
|---|---|---|
| 06h | WREN | none |
| 04h | WRDI | none |
| 05h | RDSR | status, repeated for every transfer |
| 01h | WRSR | one byte |
| 9Fh | RDID | `FFh` bytes (EEPROM and FRAM do not answer an ID read) |
| 03h | READ (RDLO on 0.5K) | address, then data to read |
| 02h | WRITE (WRLO on 0.5K) | address, then bytes to write |
| 0Bh | RDHI (0.5K only) | address, then data to read: the upper 256 bytes |
| 0Ah | WRHI (0.5K only) | address, then bytes to write: the upper 256 bytes |

Address bytes by chip: 0.5K EEPROM takes **one** byte (the ninth address bit is in the command:
`03h`/`02h` for the lower half, `0Bh`/`0Ah` for the upper); 8K and 64K EEPROM and both FRAMs take
**two**; 128K EEPROM takes **three** (GBATEK writes "As above, but with 24bit addr ?", see
"Unverified").

Sizes and pages, by chip:

| Chip | Bytes | Page | Address bytes |
|---|---|---|---|
| EEPROM "0.5K" | 512 | 16 | 1 |
| EEPROM "8K" | 8192 | 32 | 2 |
| EEPROM "64K" | 65536 | 128 | 2 |
| EEPROM "128K" | 131072 | not given | 3 |
| FRAM "8K" | 8192 | no limit | 2 |
| FRAM "32K" | 32768 | no limit | 2 |

GBATEK's names count bytes ("8K bytes", "64K bytes", "128K bytes"; the 0.5K part is 512 bytes), so
the table gives the byte counts the code uses.

Reads continue through memory, the address counter advancing by one per byte; a counter that runs
past the end of the chip wraps to 0. A write wraps inside its **page**: the address counter
advances, but the bits above the page stay fixed, so the byte after the last one of a page goes to
the page's first byte. FRAM has no page: its writes advance through memory like reads.

An address past the chip's size is **mirrored**: the high bits are dropped (the sizes are all
powers of two).

If WP covers an address, a write to it is ignored: 1 protects the upper quarter of the memory, 2
the upper half, 3 everything. The address counter still advances.

#### FLASH

The FLASH set is the firmware chip's (GBATEK, "DS Firmware Serial Flash Memory"): three address
bytes, a page of 256 bytes, and a sector of 65536 bytes.

| Command | Name | Bytes after it |
|---|---|---|
| 06h / 04h | WREN / WRDI | none |
| 05h | RDSR | status (only WIP and WEL; repeated) |
| 9Fh | RDID | up to three ID bytes (below) |
| 03h | READ | address, then data to read |
| 0Bh | FAST READ | address, one dummy byte, then data to read |
| 0Ah | PW, page write | address, then 1..256 bytes: the page is erased and then written |
| 02h | PP, page program | address, then 1..256 bytes: bits can only go from 1 to 0 |
| DBh | PE, page erase | address: the 256-byte page becomes `FFh` |
| D8h | SE, sector erase | address: the 65536-byte sector becomes `FFh` |
| B9h / ABh | DP / RDP, deep power-down and its release | none |

A PW changes only the bytes sent (GBATEK: it "reads unchanged data, erases the page, then writes new & unchanged data"): it reads the page, erases it, and writes back the bytes that were
not sent next to the new ones, so the bytes the program did not name keep their value. A PP ANDs the
new bytes into memory. A write wraps inside its page, like an EEPROM write. PE and SE take effect
when the session ends. The write commands each clear WEL when their session ends, as the status
register's description says. An address past the chip's size is mirrored.

FLASH sizes and the three-byte ID that RDID returns (the first byte is the maker, the second the
type, the third the capacity). The ID is a chip specification, not data read from a dump:

| Chip | Bytes | ID |
|---|---|---|
| FLASH "256K" | 262144 | `20h 40h 12h` |
| FLASH "512K" | 524288 | `20h 40h 13h` |
| FLASH "1M" | 1048576 | `20h 40h 14h` |
| FLASH "8M" | 8388608 | `C2h 20h 17h` |

RDID returns the three bytes in order. Bytes past the third read `FFh` (see "Unverified"). RDSR on a
FLASH shows WEL in bit 1 and zero elsewhere.

### Detecting a chip from the outside

GBATEK lists the answers a program can see. With RDSR and RDID read in turn:

| RDSR | RDID | Chip |
|---|---|---|
| `FFh` | `FFh FFh FFh` | none |
| `F0h` | `FFh FFh FFh` | EEPROM, one address byte |
| `00h` | `FFh FFh FFh` | EEPROM or FRAM, two address bytes |
| `00h` | other bytes | FLASH |

The model gives exactly these answers for the chips it has (the model's choice, not a measurement); no chip (`none`) reads `FFh` for every
byte, RDSR included.

### Edge cases

- An unknown command byte: the session ignores everything that follows and answers `FFh`.
- During the command byte, address bytes and write data, the chip answers `FFh` (the output line
  floats high); only read bytes and status carry data.
- A write command's session that is cut short (the session ends before the address is complete)
  changes nothing.
- AUXSPIDATA written while the slot mode is not SPI, or the slot is not enabled, does nothing: no
  transfer, no busy.
- AUXSPIDATA written while busy is ignored.
- Reading AUXSPIDATA gives the byte received by the last transfer, busy or not, and bits 8-15 read
  zero.
- A byte write to AUXSPIDATA's low byte starts a transfer; a byte write to its high byte
  (`0400_01A3h`) does nothing. A halfword or word write that covers AUXSPIDATA uses its low 8 bits.

## Model (not hardware)

These are choices of the emulator. They do not describe a part.

### Writes finish at once

The hardware takes time to finish a write, program or erase, and WIP says so. The model finishes
the operation inside the transfer that completes it: **WIP is always 0**. Software that polls WIP
sees a ready chip.

### Chip type: how it is chosen

The emulator, unlike a cart, does not come with a chip, so it has to be told or to guess. The
type, in order of priority:

1. the host's `--save-type` (`none`, `eeprom-512`, `eeprom-8k`, `eeprom-64k`, `eeprom-128k`,
   `flash-256k`, `flash-512k`, `flash-1m`, `flash-8m`, `fram-8k`, `fram-32k`; here the number is
   the **size**: `eeprom-512` is 512 bytes, `eeprom-8k` is 8 KiB, and so on);
2. the size of an existing save file, when it matches one of the sizes above (the host decides;
   8 KiB can be an 8K EEPROM or an 8K FRAM, see "Save files");
3. **auto-detection** in the core, from the first writing session, described next.

There is no game database. There is no re-detection on every transfer.

### Auto-detection

Until a type is found, the chip behaves as an empty one: every read returns `FFh`. RDSR returns WEL
in bit 1 and zeros elsewhere (so a WREN is visible), and RDID returns `FFh` bytes. The decision
comes from the first session that writes: the command byte and *n*, the number of bytes the
session sent after the command (address and data together).

A byte count cannot say how many of the bytes are address. The model uses the chips' page sizes: a
session's data is never longer than the page of the chip it is for, so it takes the **narrowest
address that fits** the count.

| First writing session | Decided type |
|---|---|
| `02h` or `0Ah`, 2 <= *n* <= 17 | EEPROM, one address byte (512 bytes) |
| `02h`, 18 <= *n* <= 130 | EEPROM, two address bytes (64K bytes) |
| `02h`, 131 <= *n* <= 259 | FLASH (three address bytes) |
| `02h`, *n* > 259 | FRAM, two address bytes |
| `0Ah`, 18 <= *n* <= 259 | FLASH |
| `DBh` or `D8h`, *n* = 3 | FLASH |
| anything else | no decision; the session is ignored |

The rows overlap on purpose, in the order given. Cases the table settles:

- `0Ah` is the upper-half write of the 0.5K EEPROM and the page write of FLASH; `0Bh` is its
  upper-half read and the FLASH fast read. The byte count tells them apart: a short session is the
  EEPROM's.
- `02h` with three address bytes is both 128K EEPROM's write-and-erase and FLASH's page program. On
  an empty chip they look the same, so the model decides **FLASH**; 128K EEPROM is available only by
  naming it.
- A first session that only reads decides nothing: an empty chip would answer `FFh` anyway.

Default size per class (a larger chip is safe: an address that fits the real chip also fits a larger
one, whereas a smaller chip would mirror the program's data over itself):

| Decided class | Size the model uses |
|---|---|
| EEPROM, one address byte | 512 bytes (the only one) |
| EEPROM, two address bytes | 64K EEPROM, 65536 bytes (page 128) |
| FLASH | 1M, 1048576 bytes (RDID `20h 40h 14h`) |
| FRAM | 32K, 32768 bytes |

The detector keeps the bytes of the session it is watching, up to 1024 after the command, so it
can replay them. Once the type is decided, the writing session that decided it **is applied to the chip** (it is not
lost), and the type stays for the rest of the run. A savestate keeps the decision.

### Deep power-down and the power-up lockout

GBATEK says a FLASH in deep power-down ignores every instruction except the release, and that
writes, programs and erases are rejected for the first 1 to 10 ms after power-up. The model does
neither: DP and RDP are accepted and have no effect, and a write is never refused for being too
early (time since power-up is not tracked). Both are the emulator's simplification.

### Wrong detection and the way out

A short first write to an 8K or 64K EEPROM or to a FLASH (17 bytes or fewer after the command) is
decided as the 512-byte EEPROM; from then on a second address byte is stored as data and accesses
land at the wrong address. The 512-byte file the host writes then names that type by its size on
every later run, and `--save-type` of another size refuses it. The way out: delete the save file,
or run once with `--save-type` of the right chip and a save file that does not exist yet. The
hosts print the type that was decided, so a wrong guess is visible.

### Save files

A save file is the chip's memory as raw bytes: the file's size is the chip's size, with no header.
The host supplies the file's bytes at power-on and takes the memory back. A savestate **includes the
save memory**, so loading one puts the save back to the state it had then; the host sees that as a
change of the memory and writes it to the file at its next flush. The core counts changes in a
*generation* number so the host knows when to write; that counter is bookkeeping for the host,
not hardware, and is not a part of the savestate bytes.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware, or a trace against the reference emulator used as an oracle.

- **The SPI clock divisor** for "4 MHz" etc. is read here as the bus clock divided by 8, 16, 32,
  64 (4.19 MHz for baud 0); GBATEK gives round numbers. `crates/core/src/io/cart/` (the busy
  time). Verify by timing a transfer.
- **AUXSPIDATA written while busy** is ignored here; the real chip may corrupt the byte or queue
  it. `io/cart/`.
- **A write to AUXSPIDATA with the slot mode or enable off**, and a change of mode or enable during
  a session, are not described; the model ignores the first and leaves a session running through
  the second. `io/cart/`.
- **The 128K EEPROM**: its page size (the model uses 256), its address width (GBATEK has a "?"),
  and whether its command set is the 16-bit one with one more address byte. `cart/save/`.
- **Writing into a write-protected region** (WP bits): the model ignores the bytes and still clears
  WEL at the end of the session; the chip's own handling of WEL there is not described (GBATEK asks
  the same question about FLASH). The WP bits are not modeled for FLASH. `cart/save/`.
- **WRSR on FLASH** is not in the FLASH set, so the model ignores it. `cart/save/`.
- **What a chip answers** during the command, address and write bytes, and for an unknown command:
  `FFh` here. `cart/save/`.
- **The address counter** of a 0.5K EEPROM read past its 256-byte half (the model keeps counting
  through all 512 bytes), and of a write past its page (it wraps inside the page; GBATEK says only
  that the bytes may not exceed the page size). `cart/save/`.
- **A PW with no data byte**: nothing is erased here. **PE and SE with bytes after the address**:
  the extra bytes are ignored. `cart/save/`.
- **RDID bytes after the third** read `FFh`. **A FLASH from another maker**: the model answers with
  the ST codes in the table; whether a program checks the ID, or expects another maker's, is not
  known. **The 8M part's ID** is from the Macronix entry in GBATEK's table. `cart/save/`.
- **Auto-detection**: all of it is the emulator's heuristic. It decides small writes as the
  smallest address that fits (a one-byte write to an 8K EEPROM looks like a 0.5K write), it cannot
  see a FRAM write of 131-259 bytes (it reads as FLASH), and it has none of GBATEK's two special
  cases: carts that try one chip and use another (the workaround there is to detect again on each
  transfer, which the model does not do), and a cart that needs a type forced by its game code.
  `cart/save/` (the detector). Verify by the type a given cart really has; the host flag is the
  way around a wrong guess.
- **Class default sizes** are the larger of what a class holds; the real chip may be smaller, so a
  save file made by auto-detection can be larger than the cart's own. `cart/save/`.
- **Whether `00h`-prefixed FLASH access** (infrared carts) matters is out of scope.
- **Parts of the session model that GBATEK does not state**: the hold bit being read when a transfer
  starts; a command's WEL and WP effects landing when the session ends; WRSR writing only WP and
  SRWD, needing WEL and clearing it; a session cut short before its address is complete changing
  nothing; an unknown command ignoring the rest of its session; the read address counter wrapping
  to 0 at the end of the chip; address mirroring past the chip size (GBATEK only says reads can show
  mirrors); FRAM writes advancing through memory with no page. `cart/save/`, `io/cart/`.
- **AUXSPIDATA's reset value** (0), its read while busy (the last received byte), and its byte and
  wider write widths (only a write to the low byte starts a transfer; wider writes use the low
  8 bits). `io/cart/`.
- **Details outside GBATEK**: WRSR on a FLASH is ignored; a page write with no data byte erases nothing; bytes after a page or sector erase address are ignored; a read on the 512-byte EEPROM keeps counting past its 256-byte half. `cart/save/`.
- **A session past 1024 bytes** is replayed cut short when it decides the type, so the chip misses
  the bytes after the first 1024. `cart/save/` (the detector's record).
- **A chip of unknown type** answers RDSR with only the latch and RDID with `FFh` here. `cart/save/`.
