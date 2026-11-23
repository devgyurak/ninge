# The real-time clock (RTCCNT)

Sources: GBATEK "DS Real-Time Clock (RTC)" (the register, the transfer, the command and the chip's
registers, the interrupt); GBATEK "DS Serial Port" (the RCNT remark and where /INT goes); GBATEK
"DS Interrupts" (IF bit 7).
Used by: `crates/core/src/io/rtc/`, `crates/core/src/rtc/`, `crates/core/src/input/`   Phase: 5 (#40)

A small clock chip with a three-wire serial bus, wired to the ARM7 through one register. The chip
counts the calendar time and holds a few status and alarm registers; the program talks to it one
pin at a time, so the serial protocol is done by the *program* toggling bits of RTCCNT. The chip is
only reached by the ARM7.

This note covers the pin register, the serial transfer, the chip's registers and how the model
gets its time. The alarm and frequency interrupts are described and **not implemented** (see "The
interrupt").

## Scope

Implemented (#40): RTCCNT, the serial transfer, status 1, status 2, the date-time and time
registers, INT1, INT2, the clock-adjust and free registers (stored and read back), the 12- and
24-hour modes, and the model's time (`rtc/`).

Not implemented: the interrupt line (INT1 and INT2 raising ARM7 IF bit 7, and RCNT's SI input;
`TODO(phase5)` at the site until its follow-up issue exists), the DSi's extra chip (extended commands, up counter,
alarm dates, FOUT), and the oscillator-accuracy effect of the clock-adjust register. The firmware's
RTC offset (user-settings byte 68h, `firmware.md`) belongs to the firmware, not the chip.

## Registers

ARM7 only. On the ARM9 these addresses hold nothing: reads give 0 and writes are dropped, as for
EXTKEYIN and SPICNT (`keypad.md`, `spi.md`).

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| 0x0400_0138 | RTCCNT | 16 | R/W | 0 (our choice, see "Unverified") |

### RTCCNT bits

Three pins matter: the serial data line (SIO), the serial clock (SCK) and the chip select (CS).
Each has a level bit and a direction bit; the chip answers on SIO.

| Bits | Meaning |
|---|---|
| 0 | SIO level: written when the direction bit is 1, read when it is 0 |
| 1 | SCK level (the clock is active low: data moves on its falling edge) |
| 2 | CS level (1 = chip selected) |
| 3 | An unused line (level) |
| 4 | SIO direction: 0 = the program reads the line, 1 = the program drives it |
| 5 | SCK direction: 1 = the program drives it ("should be 1") |
| 6 | CS direction: 1 = the program drives it ("should be 1") |
| 7 | Direction of line 3 (usually 0) |
| 8-11 | Four more unused lines (levels) |
| 12-15 | Directions of lines 8-11 (usually 0) |
| 16-31 | Not used |

A bit whose direction is "drive" reads back what was written. A line the program reads shows what
is on the pin: for SIO that is the bit the chip is sending (when it sends one), and for every
other line nothing is connected. A program also has to keep RCNT (`4000134h`) in general-purpose
mode "before accessing" the RTC; the model does not require it (see "Unverified").

## Behavior

### A transfer

The program selects the chip, sends a **command byte**, then sends or receives the **parameter
bytes** that command names, then deselects. GBATEK's recipe:

1. Start with CS low and SCK high; wait a microsecond.
2. Raise CS; wait.
3. Send the command byte.
4. Send or receive the parameter bytes of the command.
5. Drop CS.

A bit is moved by one SCK cycle: the program drives SCK low (and, for a write, puts the data bit on
SIO), waits, drives SCK high, waits, and for a read samples SIO. The chip takes a written bit at
the rising edge of SCK and puts a bit it sends on SIO after the falling edge, so the program sees
it once SCK is high again. The waits are the program's: the model does no timing.

### Bit order

Bits go out **least significant bit first**, for the command byte and for every parameter byte.
GBATEK's flowchart says so, and notes that the chip's original data sheet recommends the command
byte most-significant-first (with the data least-significant-first), and that later data sheets
recommend most-significant-first for everything.

So the same command can be written two ways. GBATEK names the layouts "Fwd" (the byte as it would
be if sent LSB-first) and "Rev" (the byte sent MSB-first). A program that sends the Rev byte
MSB-first puts exactly the same four-bit pattern on the wire as one that sends the Fwd byte
LSB-first, so on the wire there is one protocol. This note describes the Fwd layout, assembling
the byte in the order the bits arrive (the first bit is bit 0), and gives the Rev byte beside it
where it helps.

### The command byte

| Fwd bit | Meaning |
|---|---|
| 0-3 | Fixed code: bit 0 = 0, bit 1 = 1, bit 2 = 1, bit 3 = 0 (so the low nibble is 6) |
| 4-6 | The command (the table below) |
| 7 | Access: 1 = read, 0 = write |

(Bit 3 = 1 is the DSi's extended command; this console treats a byte with a wrong fixed code as no
command, see "Edge cases".)

| Command (bits 4-6) | Rev value | Parameter | Name |
|---|---|---|---|
| 0 | 0 | 1 byte | Status register 1 |
| 4 | 1 | 1 byte | Status register 2 |
| 2 | 2 | 7 bytes: year, month, day, day of week, hour, minute, second | Date and time |
| 6 | 3 | 3 bytes: hour, minute, second | Time |
| 1 | 4 | 1 byte, or 3 bytes (see below) | INT1 |
| 5 | 5 | 3 bytes: day of week, hour, minute | INT2 (alarm 2) |
| 3 | 6 | 1 byte | Clock adjustment |
| 7 | 7 | 1 byte | Free register |

The INT1 parameter's size depends on status register 2, bit 2: with the bit clear INT1 is the
"selected frequency" setting, one byte; with it set INT1 is alarm 1, three bytes (day of week, hour,
minute). The single frequency byte and alarm 1's minute byte are the same storage: writing one
changes the other.

The common commands as the usual Rev bytes (sent MSB-first, the way a program sees them in a
header): status 1 write/read `60h`/`61h`, status 2 `62h`/`63h`, date and time `64h`/`65h`, time
`66h`/`67h`, INT1 `68h`/`69h`, INT2 `6Ah`/`6Bh`, clock adjust `6Ch`/`6Dh`, free `6Eh`/`6Fh`.

### Status register 1

| Bits | Access | Meaning |
|---|---|---|
| 0 | W | Reset: writing 1 resets the chip (reads 0) |
| 1 | R/W | 0 = 12-hour mode, 1 = 24-hour mode |
| 2-3 | R/W | General-purpose bits |
| 4 | R | INT1 flag; cleared by the read |
| 5 | R | INT2 flag; cleared by the read |
| 6 | R | Power-low flag; cleared by the read |
| 7 | R | Power-off flag (the battery was removed or flat); cleared by the read |

Power off means every register was reset (to `00h`, or `01h` where a month or day starts at one) and
needs initialising. A reset (bit 0) gets the same registers.

### Status register 2

| Bits | Access | Meaning |
|---|---|---|
| 0-3 | R/W | INT1 mode and enable (below) |
| 4-5 | R/W | General-purpose bits |
| 6 | R/W | INT2 enable: 0 = off, 1 = alarm 2 |
| 7 | R/W | Test mode, "don't use"; cleared by a reset |

INT1 mode: `0000b` off; `0x01b` the selected-frequency steady interrupt; `0x10b` the per-minute
edge interrupt; `0011b` per-minute steady (30.0 s duty); `0100b` alarm 1; `0111b` per-minute
steady (0.0079 s duty); `1xxxb` 32 kHz output. The model stores the register as written.

### The date and time registers

All values are BCD.

| Register | Bits | Meaning |
|---|---|---|
| Year | 0-7 | `00h`..`99h` = 2000..2099 |
| Month | 0-4 | `01h`..`12h`; bits 5-7 read zero |
| Day | 0-5 | `01h`..`31h` as the month and year allow; bits 6-7 read zero |
| Day of week | 0-2 | `00h`..`06h`, a counter whose assignment is the program's (GBATEK: "usually 0 = Monday?"); bits 3-7 read zero |
| Hour | 0-5 | 24-hour mode: `00h`..`23h`. 12-hour mode: `00h`..`11h`, where 12 o'clock is `00h` |
| Hour | 6 | AM/PM (0 = AM, 1 = PM). 24-hour mode: read-only, 1 from `12h` on. 12-hour mode: read/write |
| Hour | 7 | Zero |
| Minute | 0-6 | `00h`..`59h`; bit 7 reads zero |
| Second | 0-6 | `00h`..`59h`; bit 7 reads zero |

The chip counts the calendar itself, leap years included (the range 2000-2099 has no century
exception: 2000 is a leap year and 2100 is outside). Writing the time-only command changes the
hour, minute and second and leaves the date and the day-of-week counter alone. The day-of-week
counter is not computed from the date: the program writes it, and the chip steps it once a day.

### Alarm registers

INT1 (alarm mode) and INT2 each hold three bytes: day of week, hour, minute. The day-of-week byte
has the day in bits 0-2 and a compare enable in bit 7 (0 = every day); the hour byte has the hour
(bits 0-5, as in the clock), AM/PM (bit 6) and a compare enable (bit 7); the minute byte has the
minute (bits 0-6) and a compare enable (bit 7). The selected-frequency byte (INT1 in frequency
mode, shared with alarm 1's minute) has one enable bit per frequency: bit 0 = 1 Hz, bit 1 = 2 Hz,
bit 2 = 4 Hz, bit 3 = 8 Hz, bit 4 = 16 Hz, bits 5-7 general purpose.

The model stores all three bytes of each alarm and reads them back as written, with bits 3-6 of a
day-of-week byte zero.

### The clock-adjust and free registers

One byte each, eight bits read/write. The clock-adjust byte compensates the oscillator (`00h` is no
adjustment) and has no effect in the model. The free register is general purpose.

## How the model gets its time (not hardware)

The real chip has its own oscillator; ninge-core may not read the host's clock (`AGENTS.md` §2,
rule 1), and the same ROM and input log must give the same result (rule 6). So the time is an
**input**, like the buttons (`keypad.md`, "Where the bits come from"):

- Each frame's `Input` carries the host's wall-clock time as whole seconds since
  2000-01-01 00:00:00 in the host's local time. A plain integer: no time zone in core. The default
  `Input` carries 0, the epoch, so a test with no setup sees 2000-01-01 00:00:00.
- The chip holds only what the program set: the difference between the time it last wrote and the
  host's time at that moment, plus the status, alarm, adjust and free bytes. A read is the host's
  time plus the difference, split into the calendar fields.
- The clock therefore advances at frame boundaries (the time the host gave for the current frame),
  not between two reads inside one frame. A program that sets the time and reads it again in the
  same frame sees the time it set; the next frame advances it by the host's elapsed seconds.
- A host time or a stored difference that puts the clock outside 2000-2099 wraps within those
  hundred years (the year register has two digits).
- The day-of-week counter is stored the same way: the model keeps the value that maps the epoch
  day to a weekday and steps it with the calendar day.

## Power-on state

The chip starts in 24-hour mode with the flags clear, status 2, the alarms, the clock-adjust and
the free registers zero, the time at the host's time (a difference of zero), and the day of week
the weekday of 2000-01-01 in a Monday = 0 numbering. These are what the firmware would leave after
its own initialisation (see "Unverified"). Direct boot's copies of the RTC status at `027FF816h` and
`027FFC16h` stay zero (`direct-boot.md`).

A reset command puts the chip in the same state except that status 1 reads 0 (12-hour mode) and the
time becomes 2000-01-01 00:00:00, a difference taken at that moment.

## The interrupt (deferred)

There is one /INT output for INT1 and INT2, wired to the serial port's SI input (RCNT bit 0, so it
is shared with SIO interrupts) rather than to its own interrupt source. To use it a program sets
RCNT to `8144h` (general-purpose mode, SI interrupt enable, the output bits high as a pull-up); the
interrupt comes on a high-to-low transition of the line when RCNT's interrupt enable is on, and
turning the enable on does not itself raise one. This reaches the ARM7 as IF bit 7.

Not implemented in #40: INT1 and INT2 never raise anything, and the status register's flags (INT1,
INT2, power low, power off) never read 1.

## Edge cases

- A command byte whose fixed code is wrong is not a command: the chip ignores everything until CS
  drops.
- A write command takes its whole parameter before it takes effect. A write that is cut short (CS
  drops before the last parameter bit) changes nothing. Extra clocks after the last parameter byte
  are ignored.
- A read command sends the register's bytes in turn; bits beyond the last byte read as zero.
- A read takes a snapshot of the register when the command byte ends: every byte of one read comes
  from the same instant, whatever the host time does during the transfer (a model choice).
- A date-and-time or time write whose value is not a calendar value (a BCD digit above 9, month 0
  or 13, day 0 or past the month's end, weekday 7, hour 24 or, in 12-hour mode, 12 or more,
  minute or second 60 or more) is dropped whole: the time is unchanged.
- Changing the 12/24-hour bit does not change the time, only how the hour register shows it.
- RTCCNT's reads never change anything: the transfer moves only when a write changes a pin.
- CS dropping at any moment ends the session; the next rising CS starts a new command.
- On the ARM9 the register reads 0 and ignores writes.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware, or a trace against the reference emulator used as an oracle.

- **RTCCNT's reset value** (0, every pin an input) and **what an undriven input reads**: 0 for
  SCK, CS and the unused lines, and 0 for SIO when the chip is not sending. GBATEK states neither.
  `io/rtc/`.
- **The unused lines (3 and 8-11):** the model stores their levels and directions and reads a
  driven line back, an undriven one as 0. `io/rtc/`.
- **Chip select and the clock rising in one write.** GBATEK raises select with the clock already
  high and waits; a program that raises both at once is outside that sequence. The model sees the
  select and does not see the clock edge of the same write, so the data line at that moment is not
  taken as a command bit. `io/rtc/`.
- **A direction bit changing on its own** counts as the pin's level changing (an input reads as low),
  so clearing the select direction while select is high deselects the chip, and setting the clock
  direction while its level bit is high is a rising clock edge. `io/rtc/`.
- **High bits of a written date byte** (month bits 5-7, day bits 6-7, weekday bits 3-7, hour bit 7,
  minute and second bit 7) are ignored, not rejected. `rtc/`.
- **RCNT is not checked.** GBATEK says RCNT should be in general-purpose mode first, "no idea why";
  the model works with RCNT at its reset value. `io/rtc/`.
- **A wrong fixed code** (and bit 3 set, the DSi's extended command): the session is ignored until
  CS drops. `rtc/`.
- **A write cut short** is discarded whole; the hardware may take the bytes it already received.
  `rtc/`.
- **A date or time write with a value that is not a calendar value** is dropped whole; GBATEK does
  not say what the chip does with one. `rtc/`.
- **Bits beyond the last parameter byte of a read** are zero, and extra clocks on a write are
  ignored. `rtc/`.
- **The chip's power-on state:** 24-hour mode, flags clear, registers zero, day of week for
  2000-01-01 in Monday = 0 numbering (5). The hardware's power-on flags and what the firmware
  leaves are not in GBATEK. `rtc/`.
- **A reset's effect:** the time becomes 2000-01-01 00:00:00, status 1 reads 0 (so 12-hour mode),
  the other registers clear; GBATEK says "00h (or 01h)". `rtc/`.
- **Writing the 12/24-hour bit, bits 2-3 of status 1 and every bit of the other status and
  alarm registers** is stored as written with the masks above; the effect of the test-mode bit is
  not modelled. `rtc/`.
- **The wrap at the end of the range** (2099-12-31 23:59:59 followed by 2000-01-01 00:00:00); GBATEK
  gives only the range. The hundred years are not a whole number of weeks, so across the wrap the
  day-of-week counter does not step by one: it shows the weekday the model's mapping gives for the
  first day of 2000. `rtc/`.
- **Time moves at frame boundaries** only, a choice of the model, not a hardware fact.
  `io/rtc/`.
- **What an ARM9 access sees** (0 and dropped here); GBATEK marks the register NDS7 and says no
  more. `io/rtc/`.
- **Direct boot's RTC status words** (`027FF816h`, `027FFC16h`) stay zero; what the firmware copies
  there is not in GBATEK. `hle/direct_boot.rs`.
