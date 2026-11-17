# Cartridge protocol: reading the ROM

Sources: GBATEK "DS Cartridge Protocol"; GBATEK "DS Cartridge I/O Ports"; GBATEK "DS Memory
Control - Cartridges and Main RAM" (EXMEMCNT); GBATEK "DS DMA Transfers"; GBATEK "DS Interrupts";
GBATEK "DS Cartridge Header" (the fields that set the bus speed and the chip size).
Used by: `crates/core/src/cart/`, `crates/core/src/io/`   Phase: 5 (#37)

The cartridge slot ("slot 1") is a serial bus: the CPU sends an 8-byte command, then receives a
stream of data bytes. This note covers reading the ROM. The save chip on the same connector is
`TODO(#38)`; the key-based encryption that real boot code uses is out of scope here (see "Scope").

## Scope

Implemented: the **main data mode** commands `B7h` (read data) and `B8h` (chip ID), through the
registers below, with the transfer's timing, its interrupt and its DMA start.

Not implemented, and not planned for this feature:

- the plain commands used right after a reset (`9Fh`, `00h`, `90h`), and the KEY1-encrypted
  commands that lead from them to the main data mode;
- KEY1 and KEY2 encryption. The card is simply *in main data mode with encryption off*: data
  comes out as plain bytes. The ROMCTRL bits and seed registers that control encryption are
  stored where GBATEK says they are readable, and have no effect. Direct boot (`direct-boot.md`)
  never runs the plain and KEY1 phases, so it starts the card in main data mode.

## Registers

All of these are seen by both CPUs, but only the CPU that owns the slot reaches them (see
"Ownership").

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_0204 | EXMEMCNT (ARM9) / EXMEMSTAT (ARM7) | 16 | R/W (see below) | 0 |
| 0x0400_01A0 | AUXSPICNT | 16 | R/W | 0 |
| 0x0400_01A2 | AUXSPIDATA | 16 | R/W | `TODO(#38)` |
| 0x0400_01A4 | ROMCTRL | 32 | R/W | 0 (power-on; direct boot sets it, see "The card after direct boot") |
| 0x0400_01A8 | Command out (8 bytes) | 64 | W | 0 |
| 0x0410_0010 | Data in (4 bytes) | 32 | R | 0 |

The encryption seed registers (`0400_01B0h`-`0400_01BAh`) belong to the encryption feature and
are not modeled. AUXSPIDATA is the save chip's (#38).

### EXMEMCNT bits

GBATEK names the register EXMEMCNT on the ARM9 and EXMEMSTAT on the ARM7. Each CPU has its own
copy of bits 0-6; bits 7-15 are one setting that only the ARM9 can change and both CPUs read.

| Bits | Meaning |
|---|---|
| 0-6 | GBA slot timing and clock (per CPU; no effect here) |
| 7 | GBA slot access rights: 0 = ARM9, 1 = ARM7 (no effect here) |
| 8-10 | Not used, read as zero |
| 11 | **NDS slot access rights: 0 = ARM9, 1 = ARM7** |
| 12 | Not used, read as zero |
| 13-15 | Main-memory interface bits (stored, no effect here) |

Only bit 11 changes anything in this feature. The other bits are stored so a read gives back what
the ARM9 wrote; their meaning belongs to the memory-timing work (Phase 8).

An ARM7 write changes its own bits 0-6 and nothing else. An ARM9 write changes bits 0-6 of the
ARM9's copy and the shared bits 7-15.

### AUXSPICNT bits

| Bits | Meaning |
|---|---|
| 0-1 | SPI baud rate (`TODO(#38)`: stored, no effect) |
| 2-5 | Not used, zero |
| 6 | SPI hold chip select (`TODO(#38)`: stored, no effect) |
| 7 | SPI busy (`TODO(#38)`: reads 0 until the save chip exists) |
| 8-12 | Not used, zero |
| 13 | Slot mode: 0 = ROM (parallel), 1 = SPI backup (serial) |
| 14 | Transfer-ready IRQ: 0 = off, 1 = on. For the ROM side only |
| 15 | Slot enable |

Writable bits are 0-1, 6 and 13-15 (`E043h`); the rest read zero.

### ROMCTRL bits

| Bits | Meaning |
|---|---|
| 0-12 | KEY1 gap 1 length, in bus clocks: the leading gap, between the command and the first data |
| 13 | KEY2 encrypt data (stored, no effect) |
| 14 | "SE", unknown (stored, no effect) |
| 15 | KEY2 apply seed. Write-only; reads 0; no effect |
| 16-21 | KEY1 gap 2 length, in bus clocks: the gap after each 200h-byte part of the data |
| 22 | KEY2 encrypt command (stored, no effect) |
| 23 | Data word status: 0 = busy, 1 = a word is ready to be read. Read-only |
| 24-26 | Data block size, see below |
| 27 | Transfer clock: 0 = 6.7 MHz (33.51 MHz / 5), 1 = 4.2 MHz (33.51 MHz / 8) |
| 28 | KEY1 gap clocks: 0 = clock held high during gaps, 1 = dummy pulses (stored, no effect) |
| 29 | RESB, release reset: 0 = reset, 1 = released. Once set it cannot be cleared |
| 30 | Data direction: 0 = read, 1 = write (stored; see "Unverified") |
| 31 | Block start / busy: writing 1 starts a transfer; reads 1 until the transfer is done |

**Data block size** (bits 24-26): 0 = no data; 1 to 6 = `100h << n` bytes (200h, 400h, 800h,
1000h, 2000h, 4000h); 7 = 4 bytes.

GBATEK says the console supports transfers of nothing, four bytes, and 200h to 4000h bytes.

### Command out

Eight bytes at `0400_01A8h`-`0400_01AFh`. **The first command byte is at the lowest address**, so
`B7h` goes in `0400_01A8h` and the address's most significant byte in `0400_01A9h`. This is the
reverse of the CPU's byte order. A word written to `0400_01A8h` holds bytes 0-3 of the command
with its lowest byte first: the word's bits 0-7 are the first command byte (`B7h`) and bits 24-31 are
the fourth. A word at `0400_01ACh` holds bytes 4-7 the same way.

For `B7aaaaaaaa000000h` the bytes are, in order: `B7`, address bits 31-24, 23-16, 15-8, 7-0,
`00`, `00`, `00`.

### Data in

A 32-bit register at `0410_0010h`. The first received byte is bits 0-7, the second bits 8-15,
and so on. It is read by the CPU (when ROMCTRL bit 23 says a word is ready) or by DMA.

## Commands

### Main data mode

The card is in this mode after direct boot. Only two commands are defined here.

**`B7aaaaaaaa000000h` - read data.** Returns the bytes of the ROM starting at the address, as
many as the data block size says. The address is the ROM offset, most significant byte first. It
needs no alignment.

Rules for the address, from GBATEK:

1. **Wrap inside a 4 KiB block.** If the data runs across a 1000h boundary, the stream wraps to
   the start of the block it began in: the byte after offset `xxxxFFFh` is offset `xxxx000h` of the
   *same* block, not the next one.
2. **Low addresses are redirected.** The command reads only offsets `8000h` and up. An address
   below `8000h` is silently replaced by `8000h + (address & 1FFh)`.
3. **Addresses past the end of the ROM mirror** into the valid range, including the
   non-readable low region: such an address ends up redirected like rule 2.

The order our model applies is: mirror the address into the chip, then redirect, then read with the
4 KiB wrap.

**`B800000000000000h` - chip ID.** Returns the 4-byte chip ID, repeated every 4 bytes for as many
bytes as the block size says.

### The chip ID

The chip ID is four bytes (GBATEK, "1st Get ROM Chip ID"):

| Byte | Meaning |
|---|---|
| 1 | Manufacturer |
| 2 | Chip size: `00h` to `7Fh` means (N+1) MiB |
| 3 | Flags (bit 0: infrared on the SPI bus; others zero or unknown) |
| 4 | Flags: bit 3 NAND, bit 4 3DS, bit 5 unknown, bit 6 DSi, bit 7 protocol variant (0 = older, smaller ROM, 1 = newer, 64 MB or more) |

The model computes it from the ROM image, never from a dump:

- **Size byte:** the image length is rounded up to a power of two of MiB, at least 1, and the byte
  is that count minus one (GBATEK's known IDs: 8 MiB `07h`, 16 MiB `0Fh`, 32 MiB `1Fh`, 64 MiB
  `3Fh`, 128 MiB `7Fh`). Images above 128 MiB use `7Fh`; the GBATEK encoding for bigger chips is
  inconsistent in its own table.
- **Byte 3** is 0 (no infrared). **Byte 4** is 0, except bit 7, which is set when the chip is
  larger than 64 MiB (GBATEK: the variant 1 chips are "64MB or more"; the known 64 MiB parts
  differ, see "Unverified").
- **Manufacturer** is `C2h`, a value from GBATEK's list of known IDs. GBATEK does not fix the
  byte; see "Unverified".

The ID, read as the data word, has byte 1 in bits 0-7.

### Anything else

Any other command in main data mode: the model returns `FFFF_FFFFh` for every word. GBATEK says
the real answer is an endless stream of encrypted zero bytes and that the card may then stop
answering; with no encryption to apply we cannot show the same bytes. See "Unverified".

## Ownership

EXMEMCNT bit 11 gives the slot to one CPU at a time: 0 is the ARM9, 1 is the ARM7.

- The **owning CPU** reaches AUXSPICNT, ROMCTRL, the command registers and the data register.
- For the **other CPU** these addresses read zero and writes are ignored (GBATEK says this of the
  GBA slot; for the NDS slot it is silent, see "Unverified").
- **IF bit 19** is requested on the owner.
- **DMA** start modes for the slot (ARM9 mode 5, ARM7 mode 2) fire only the owner's channels.
- EXMEMCNT itself is readable by both CPUs.

Changing bit 11 while a transfer runs does not stop it. The interrupt and the DMA requests go to
whichever CPU owns the slot at the time they happen.

## Behavior

### Starting a transfer

A write to ROMCTRL that sets bit 31 while no transfer is running starts one. The transfer reads
the command registers as they are at that moment, and the other ROMCTRL bits as written by the
same write.

1. Bit 31 reads 1 and bit 23 reads 0.
2. After the command and the leading gap, the first word is ready: bit 23 reads 1 and the data
   register holds the word.
3. A **read of the data register** takes the word: bit 23 reads 0 until the next word is ready.
4. When the last word has been taken, the transfer is done: bit 31 reads 0, and if AUXSPICNT
   bit 14 is set, IF bit 19 is requested on the owner.

A block size of 0 has no words: the transfer is done as soon as the command and the leading gap
are over.

A write that sets bit 31 while a transfer runs changes the other bits and does not restart it. A
write that clears bit 31 while a transfer runs does not stop it (the model has no way to abort).
See "Unverified".

### Reading the data register

- When a word is ready, a 32-bit read returns it and takes it (see step 3).
- Narrower reads return the matching bytes of the word. The word is taken by a read that
  includes the register's byte 3, and by no other: see "Unverified".
- When no word is ready, a read returns the last word taken (zero before any) and takes
  nothing.

### Words

A word is 4 bytes. A transfer of *n* bytes has *n / 4* words, and a block of 4 bytes has one.

### Timing

Transfers follow the event scheduler (`scheduler.md`), in master cycles (ARM9 cycles, twice the
bus clock). The bus clock of the card is the 33.51 MHz bus clock divided by 5 (ROMCTRL bit 27 = 0)
or by 8 (bit 27 = 1): so one card clock is **10** or **16** master cycles. One byte takes one card
clock (GBATEK: 6.7 MHz gives about 6.7 MB/s), so a word takes four.

| Step | Cost |
|---|---|
| Sending the command | 8 card clocks |
| Leading gap | gap 1 card clocks (ROMCTRL bits 0-12) |
| One word | 4 card clocks |
| Gap after each 200h bytes of data (not after the last) | gap 2 card clocks (bits 16-21) |

So the first word is ready `(8 + gap1 + 4)` card clocks after the start; the word after it comes
four card clocks after the first is **taken**, plus gap 2 when the words already received make a
multiple of 200h bytes. A block of size 0 is done `8 + gap1` card clocks after the start.

GBATEK's figures fit this. The usual header value `00586000h` (gap 1 = 0, gap 2 = 18h, 6.7 MHz)
is the "faster MROM" setting; a 200h-byte block takes `8 + 0 + 512` card clocks, so the rate is
close to the 6.7 MB/s the clock allows. The slower value `00416657h` (gap 1 = 657h, gap 2 = 1,
6.7 MHz) takes `8 + 1623 + 512 = 2143` (657h is 1623) card clocks for 200h bytes, which is 1.6 MB/s: the figure
GBATEK gives for 200h-byte blocks on those carts.

The gaps are counted in the same clocks as the data: GBATEK's `910h` dummy bytes of the KEY1
commands equal gap 1 + gap 2 at their usual values (`8F8h + 18h`).

### Interrupt

IF bit 19, "card slot transfer complete", is requested on the CPU that owns the slot when a
transfer ends, if AUXSPICNT bit 14 is set at that moment. It is raised through the interrupt
controller like any other source (`interrupts.md`). Bit 20 (IREQ_MC) is not modeled.

### DMA start

DMA start mode 5 (ARM9) and mode 2 (ARM7) mean "DS cartridge slot" (`dma.md`). GBATEK's recipe is
source = `0410_0010h` fixed, length 1, 32-bit units, repeat on. Each time a word becomes ready, a
**request** goes to the owner's channels that are waiting in that mode; each request moves one unit.
The channel keeps its internal state between requests until its count is used up; then the end
of the transfer rules of `dma.md` apply.

Which channel goes first when several wait, and the other CPU's channels: see "Unverified".

### The card after direct boot

Direct boot never runs the card's reset or KEY phases (`direct-boot.md`). The model starts the card
as it would be after them:

- the card is in main data mode;
- EXMEMCNT is 0, so the ARM9 owns the slot;
- AUXSPICNT is `0000h` (slot not enabled, IRQ off);
- ROMCTRL is `2000_0000h`: RESB released, nothing else set, no transfer running.

All three values are our choice (see "Unverified").

Software that wants the slot sets AUXSPICNT and ROMCTRL itself.

### Savestate

The cartridge device's state (the registers, the command bytes, the current transfer: command,
address, bytes left, next word, whether a word is ready) is saved with the machine. The ROM image
is not saved: `Machine::load_rom` supplies it again, like the rest of the ROM-owned data. A word
that is "in flight" is rebuilt as an event on load (`scheduler.md`, "Savestate checks").

## Model (not hardware)

- **A word-by-word model.** A word-ready event is scheduled for each word, at the time above,
  from the moment the previous word is taken. Real hardware streams bytes; the word is the unit
  the software and DMA see, so it is the unit modeled.
- **The transfer waits for the CPU.** The next word is scheduled when the previous is taken,
  so a slow reader slows the transfer. Hardware may instead overrun; see "Unverified".
- **Cart events sort before DMA events** at the same master time, so a DMA channel started by the
  word that just became ready finds the word ready. This is our choice of tie order;
  `scheduler.md` records it when the card's event lands (#37, timing PR).
- **Plain data.** See "Scope".

## Edge cases

- **A transfer that starts while RESB is 0** runs anyway (see "Unverified").
- **Count mismatch:** a DMA channel whose count asks for more words than the transfer has stops
  receiving requests when the transfer ends; the channel stays armed.
- **Data register read with no transfer:** returns the last word taken.
- **ROM smaller than the chip:** a read between the image's end and the chip's end returns `FFh`
  bytes.

## Unverified

Each item is a choice the model makes where GBATEK is silent or unclear. Each gets a
`TODO(hw-unverified)` at its code site when the code lands.

- **The other CPU's access to the slot registers** reads zero and ignores writes, by analogy with
  GBATEK's GBA-slot rule; GBATEK does not say for the NDS slot.
- **Past the end of the ROM:** GBATEK says addresses that exceed the ROM size mirror into the
  valid range. The model mirrors by the chip size (the power-of-two size above), and reads `FFh`
  for the unused part between the image and the chip end (the header describes the remainder as
  `FFh`-padded).
- **Unknown commands** return `FFFF_FFFFh` in the model; GBATEK describes an encrypted-zero
  stream and a possible failed state.
- **The chip ID's manufacturer byte** is `C2h` (from the list; GBATEK says the byte is "roughly
  based on JEDEC IDs" and does not fix it). The **size byte rounding** to a power of two is
  inferred from the table of known IDs. The **protocol variant bit 7** is set above 64 MiB; the
  table's 64 MiB ID shows 0 for some chips and the model sets the bit only above 64 MiB, since GBATEK's "up to 64MB" and "64MB or more" both
  contain 64 MiB.
- **When busy clears:** the model clears bit 31 and raises the IRQ when the last word is taken.
  GBATEK says only that the IRQ is on the transfer's end.
- **Word taking:** the model takes a ready word on any data register read that includes byte 3.
  Hardware behavior for partial reads is not stated.
- **A slow reader:** the model waits for the CPU before it prepares the next word; hardware may
  overrun or drop data.
- **Start rules:** AUXSPICNT bit 15 (enable) and ROMCTRL RESB are stored but do not gate a
  start; bit 30 (write direction) is stored and ignored; a write that sets or clears bit 31 while
  busy does not restart or abort.
- **Timing constants:** the 8-clock command, the gap placement (leading gap after the command, gap 2 after each
  200h bytes, none after the last) and the one-byte-per-clock rate are our reading of GBATEK's
  descriptions, not stated cycle by cycle.
- **The card's state after direct boot** (ROMCTRL, AUXSPICNT, EXMEMCNT values) is not given by
  GBATEK for a direct boot.
- **EXMEMCNT's reset value** and its bits 13-15 (GBATEK marks bit 13 "always set?") are not
  modeled beyond storage.
- **DMA order:** which of several waiting channels gets a word first, and whether the other CPU's
  channels ever see the request. The model uses channel order and the owner only.
- **IRQ routing:** IF bit 19 goes to the slot owner only; GBATEK does not say which CPU gets it.
- **Ownership change during a transfer** (see "Ownership").
- **IREQ_MC (IF bit 20),** and the SPI side of AUXSPICNT, are not modeled (`TODO(#38)` for SPI).
