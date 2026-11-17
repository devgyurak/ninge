# The SPI bus (SPICNT, SPIDATA)

Sources: GBATEK "DS Serial Peripheral Interface Bus (SPI)" (SPICNT, SPIDATA, the glitches);
GBATEK "DS Interrupts" (IF bit 23); GBATEK "DS Firmware Serial Flash Memory" (the device on
select 1); GBATEK "DS Technical Data" (the bus clock).
Used by: `crates/core/src/io/spi/`, `crates/core/src/spi/`   Phase: 5 (#39)

The console's internal serial bus. Three chips hang on it, each with its own chip select: the
power-management device, the firmware flash and the touch-screen controller. Only the ARM7 reaches
the registers. This note covers the bus controller and what the bus does with a byte; the firmware
chip is in `firmware.md`, and the power and touch devices belong to #41.

The cartridge slot has a second, separate serial bus (AUXSPI, `auxspi-saves.md`); the two share
nothing but the idea.

## Scope

Implemented (#39): SPICNT, SPIDATA, the SPI session (device select, hold, busy, the IRQ) and the
firmware device on select 1. The power device (select 0) and the touch device (select 2) are
selectable but answer a fixed value until #41 (`TODO(#41)` at the site).

Not implemented: the DSi's 8 MHz baud rate (SPICNT bit 2) and everything else DSi-only.

## Registers

ARM7 only. On the ARM9 these addresses hold nothing: reads give 0 and writes are dropped, as for
EXTKEYIN (`keypad.md`).

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_01C0 | SPICNT | 16 | R/W (bit 7 read-only) | 0 |
| 0x0400_01C2 | SPIDATA | 16 | R/W | 0 (our choice, see "Unverified") |

### SPICNT bits

| Bits | Meaning |
|---|---|
| 0-1 | Baud rate: 0 = 4 MHz (the firmware's rate), 1 = 2 MHz (touch), 2 = 1 MHz (power), 3 = 512 kHz |
| 2-6 | Not used, zero (bit 2 is the DSi's baud-rate MSB; on this console it is zero) |
| 7 | Busy: 1 while a unit is on the wire. Read-only: a write does not change it |
| 8-9 | Device select: 0 = power management, 1 = firmware, 2 = touch screen, 3 = reserved |
| 10 | Transfer size: 0 = 8-bit (normal), 1 = 16-bit ("bugged") |
| 11 | Chip-select hold: 0 = deselect after this transfer, 1 = keep selected |
| 12-13 | Not used, zero (the BIOS writes them while it talks to the firmware, and they read back zero) |
| 14 | Interrupt request: 0 = off, 1 = on |
| 15 | Bus enable: 0 = off, 1 = on |

### SPIDATA bits

| Bits | Meaning |
|---|---|
| 0-7 | Write: the byte to send, which starts the transfer. Read: the byte received by the last transfer |
| 8-15 | Not used, read as zero, even in 16-bit mode |

## Behavior

### A transfer

A write to SPIDATA with the bus enabled (SPICNT bit 15) exchanges a unit with the selected device:
the written value goes out and, at the same time, a value comes back. A program that only wants to
read still has to write a dummy value (zero) to clock the reply in. The reply is what SPIDATA reads
once the busy flag is clear.

While the unit is on the wire SPICNT bit 7 reads 1. When it ends the bit drops, and, if SPICNT bit
14 is set, the ARM7 gets IF bit 23 ("SPI bus", `interrupts.md`).

### Chip select and hold

A device is selected for as long as a *session* lasts, as on the cartridge bus (`auxspi-saves.md`,
"Chip select and hold"). The first transfer after the device was deselected is the device's
command byte, and every later transfer of the session is a parameter or data byte. The hold bit
(bit 11), as it stands when the transfer starts, decides whether the session goes on: with hold = 1
the device stays selected; with hold = 0 it is deselected when this transfer completes. GBATEK's
advice follows: clear hold before the last unit of a command, and wait a few cycles after it before
the next command.

### Baud rate and the busy time

GBATEK gives the rates as 4 MHz, 2 MHz, 1 MHz and 512 kHz. They are the bus clock (33.51 MHz,
`scheduler.md`) divided by 8, 16, 32 and 64, the same reading as the cartridge bus. A bit takes 8,
16, 32 or 64 bus clocks, and a byte is 8 bits; in master cycles (two per bus clock) an 8-bit unit
takes **128, 256, 512 or 1024 master cycles**. A 16-bit unit takes twice that.

### The 16-bit mode

GBATEK says SPIDATA is only 8 bits wide, so in "bugged" 16-bit mode only every second byte
appears in it. The model reads that as: a unit moves two bytes on the wire (the written byte, then
a zero byte) and SPIDATA holds the byte received during the second one. See "Unverified" for what
is a guess here.

### Bus disabled

With bit 15 clear a write to SPIDATA does nothing: no unit, no busy, no IRQ, and SPIDATA keeps its
value. SPICNT's other bits are stored and read back.

## Model (not hardware)

These are choices of the emulator. They do not describe a part.

- **Lazy busy.** The busy flag is not state that an event clears. The transfer stores the master
  time it ends (`done_at`), and the flag reads as `now < done_at`, as the cartridge bus does.
- **The IRQ is the only event.** A completion event is queued only for a transfer that starts with
  SPICNT bit 14 set. It fires at `done_at` and raises ARM7 IF bit 23. A transfer without the IRQ
  queues nothing. The event is not saved: a loaded state rebuilds it from `done_at` and the stored
  IRQ bit (the same approach as every other timed device, `scheduler.md`).
- **The reply is known at once.** The device answers when the transfer starts; busy only delays
  when software sees it.

## Edge cases

- A write to SPIDATA while busy is ignored.
- A read of SPIDATA gives the last received value, busy or not.
- A byte write to the low byte of SPIDATA starts a transfer; a byte write to the high byte
  (`0400_01C3h`) does nothing. A halfword or word write that covers SPIDATA uses its low 8 bits.
- A write to SPICNT changes the bits that are stored and never bit 7. A transfer already on the
  wire keeps the device, hold and IRQ values it started with.
- Select 3 (reserved) and the devices of #41 answer `FFh` (the output line floats high, as on the
  cartridge bus) and keep no state.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware, or a trace against the reference emulator used as an oracle.

- **The clock divisor** for "4 MHz" etc.: bus clock divided by 8, 16, 32, 64 (4.19 MHz for baud 0),
  where GBATEK gives round numbers. `io/spi/` (the busy time). Verify by timing a transfer.
- **What an unconnected or reserved device answers** (select 3), and what the power and touch
  stubs answer before #41: `FFh` here, chosen. `io/spi/`.
- **SPIDATA's reset value** (0) and its read while busy (the last received byte). `io/spi/`.
- **A write to SPIDATA while busy** is ignored; the real bus may corrupt the unit or queue it.
  `io/spi/`.
- **16-bit mode:** the second byte sent is zero, the byte kept is the second one received, and the
  unit takes twice as long. GBATEK says only that every second byte appears. `io/spi/`.
- **When the IRQ fires** if SPICNT bit 14 changes during a transfer: the bit as it was at the
  start decides. `io/spi/`.
- **The bus enable bit:** a write with bit 15 clear is dropped. GBATEK does not say what the
  hardware does. `io/spi/`.
- **Changing the device select during a held session:** the model leaves the first device's
  session as it was and sends the byte to the new one. `io/spi/`.
- **Bits 2-6 and 12-13 read zero** even though the BIOS writes bits 12-13 (GBATEK notes it). `io/spi/`.
- **What an ARM9 access sees** (0 and dropped here, as for EXTKEYIN); GBATEK marks the register
  NDS7 and says no more. `io/spi/`.
