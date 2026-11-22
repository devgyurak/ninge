# The touch-screen controller and the power-management device

Sources: GBATEK "DS Touch Screen Controller (TSC)" (the control byte, the channels, the power-down
modes, the reply, the overlapped command, the calibration formula, the pen interrupt); GBATEK "DS
Power Management Device" (the index register and registers 0-4, the mirrors); GBATEK "DS Keypad"
(EXTKEYIN bit 6, the pen); GBATEK "DS Serial Peripheral Interface Bus (SPI)" (device select, hold).
Used by: `crates/core/src/spi/touch/`, `crates/core/src/spi/power/`, `crates/core/src/io/spi/`,
`crates/core/src/io/keypad/`   Phase: 5 (#41)

Two of the three chips on the internal serial bus (`spi.md`): the power-management device on device
select 0 and the touch-screen controller (TSC) on device select 2. Only the ARM7 reaches them,
through SPICNT and SPIDATA. This note covers what each chip does with the bytes the bus clocks
through it; the bus controller is in `spi.md`, the firmware chip on select 1 is in `firmware.md`.

## Scope

Implemented (#41): the TSC's control byte, every channel, the reply and the overlapped next
command, the power-down mode and the pen line that follows it; EXTKEYIN bit 6 (the pen); the
power-management device's registers 0-3 and their mirrors; the original console's model.

Not implemented: the DS-Lite and DSi chips (the Asahi Kasei touch controller and its changed
channels and power-down modes, the Mitsumi 3205B and its register 4, the DSi's registers), the
keypad and hinge interrupts (#231), the console shutting down or dimming the screens in answer to
the power register, the sound amplifier, the microphone data (#44), sleep mode (HALTCNT).

## The touch-screen controller

### The control byte

The first byte of a conversion, sent MSB first. A byte is a command only if bit 7 is set; a byte
with bit 7 clear is padding and starts nothing.

| Bits | Meaning |
|---|---|
| 0-1 | Power-down mode (see below) |
| 2 | Reference: 0 = differential, 1 = single-ended |
| 3 | Conversion: 0 = 12-bit, 1 = 8-bit |
| 4-6 | Channel, 0-7 |
| 7 | Start bit: must be 1 for the byte to be a command |

### The channels

GBATEK lists the channels as follows. "Differential" is only available on channels 1, 3, 4 and 5;
all eight can be read single-ended.

| Channel | Name | What it returns |
|---|---|---|
| 0 | Temperature 0 | a voltage that falls about 2.1 mV per degree |
| 1 | Y position | roughly `0B0h`-`F20h` while touched, `FFFh` when released |
| 2 | Battery voltage | always `000h` (the input is tied to ground on this console) |
| 3 | Z1 | diagonal position, for the pressure |
| 4 | Z2 | diagonal position, for the pressure |
| 5 | X position | roughly `100h`-`ED0h` while touched, `000h` when released |
| 6 | AUX | the microphone |
| 7 | Temperature 1 | the difference to temperature 0, about 0.39 mV per degree |

A position is turned into a pixel by the program with the two calibration points from the firmware
(`firmware.md`): `scr = (adc - adc1) * (scr2 - scr1) / (adc2 - adc1) + (scr1 - 1)`, per axis.

Temperature in kelvin, as GBATEK gives it without a calibration value:
`K = (TP1 - TP0) * 8568 / 4096`. The pressure is `X * (Z2 / Z1 - 1)`.

### The reply

After the control byte the chip sends, MSB first: one dummy bit (zero), then the conversion result
(12 or 8 bits), then zeros for as long as the program keeps clocking. The bus moves whole bytes, so
the program sees the reply in the bytes that follow the control byte:

- 12-bit: the byte right after the control byte is `0` followed by result bits 11-5 (7 bits); the third byte is result
  bits 4-0 followed by three zero bits. A program rebuilds the result as `((hi << 8) | lo) >> 3`.
- 8-bit: the byte right after the control byte is `0` followed by result bits 7-1; the third byte is result bit 0
  followed by seven zero bits, so that the same `>> 3` formula gives the 8-bit result shifted left
  by four.

The byte that carries the control byte receives whatever the chip was still sending of an earlier
reply (below), and zero otherwise. The reply can go stale: GBATEK warns that the value can be
unreliable if the program waits long before reading it.

### The next command, during the reply

A new command may start as soon as the result bit 6 has been received. Because the bus works in
whole bytes, the usual form is: send the control byte, send a zero byte (receive the high reply
byte), send the *next* control byte (receive the low reply byte of the first conversion), send a
zero byte (receive the high byte of the second), and so on. Each conversion costs two bus bytes
instead of three.

### Chip select

A session starts with chip select low and ends when it goes high (a transfer with the hold bit
clear, `spi.md`). GBATEK does not say what raising it does to a reply that was still on its way;
see "Unverified".

### Power-down mode and the pen line

The two low bits of every control byte choose a mode. It says whether the chip's `/PENIRQ` output
is enabled, whether the internal reference is on and whether the converter is left on:

| Mode | `/PENIRQ` | Reference | ADC | Recommended use |
|---|---|---|---|---|
| 0 | enabled | auto | auto | differential mode: the touch screen and the pen interrupt |
| 1 | disabled | off | on | single-ended: temperature, microphone |
| 2 | enabled | on | off | "don't use" |
| 3 | disabled | on | on | "don't use" |

So `/PENIRQ` is enabled in modes 0 and 2 and disabled in modes 1 and 3. GBATEK says that after a
conversion in mode 1 the program should go back to mode 0 to turn the pen signal on again. The
mode of the *last control byte* stays in force until the next one.

### The pen bit (EXTKEYIN bit 6)

EXTKEYIN bit 6 is the `/PENIRQ` line: it reads 0 ("pressed") while the panel is touched **and** the
pen output is enabled, and 1 ("released or disabled") otherwise (`keypad.md`). A program that wants
to know whether the screen is pressed leaves the chip in mode 0 and reads that bit before it takes
a measurement.

## The power-management device

### The index byte and the data byte

The first byte of a session is the index register; the second is the data, and the program ends the
session after it:

| Bits | Meaning |
|---|---|
| 0-6 | Register select |
| 7 | Direction: 0 = write, 1 = read |

On the original console registers 4-7Fh are mirrors of 0-3: only the low two bits of the select
matter. (The DS-Lite has a register 4 of its own and mirrors 5-7 of it; the DSi adds more. None of
that is modeled.)

### Registers

| Register | Name | Access | Bits |
|---|---|---|---|
| 0 | Power-management control | R/W | 0 sound amplifier enable · 1 sound amplifier mute (original console) · 2 lower backlight · 3 upper backlight · 4 power LED blink enable · 5 blink speed (1 = fast) · 6 system power (1 = shut down) · 7 not used, zero |
| 1 | Battery status | R | 0: battery LED (0 = power good, 1 = power low) · 1-7 not used |
| 2 | Microphone amplifier | R/W | 0 amplifier enable · 1-7 not used, zero |
| 3 | Microphone gain | R/W | 0-1 gain (0-3 = 20, 40, 80, 160) · 2-7 not used, zero |

GBATEK says the microphone amplifier is off after power-up, which is why the AUX channel gives no
signal until a program sets register 2.

## Model (not hardware)

These are choices of the emulator. They do not describe a part.

- **The console is the original DS**, matching the synthetic firmware's console type `FFh`
  (`firmware.md`). Register 4 therefore mirrors register 0.
- **Calibration is shared.** The X and Y conversions use the one constant of `spi/`
  (`CALIBRATION`, `ADC_PER_PIXEL`, `firmware.md`, "The touch calibration constant"): a touched
  pixel `p` reads as the ADC value `16 * p` on its axis, so the program's formula gives `p` back
  exactly. The 8-bit result is the 12-bit result shifted right by four.
- **A conversion is instant.** The reply is known when the control byte arrives; only the SPI unit
  time of `spi.md` passes.
- **The microphone is silent.** Channel 6 returns a fixed value, the middle of the range, whatever
  the amplifier registers say. The blow key (#44) brings real data; the code site says so.
- **The power register is stored, not acted on.** The shutdown bit, the backlight bits and the
  amplifier bits are kept and read back (and shown in the debugger); nothing reacts. The sound
  amplifier is Phase 6 work; the shutdown and the backlights are #231; each code site carries the
  matching `TODO(phase6)` or `TODO(#231)`.
- **Register 0 starts at `0Dh`.** Direct boot skips the firmware boot, and a program that starts
  after it finds the sound amplifier and both backlights on (`direct-boot.md`, "Devices at entry").
  Registers 2 and 3 start at zero.
- **The pen line is computed by the bus owner.** A device never looks at another device: the I/O
  block asks the TSC whether its pen output is enabled and hands that to the keypad together with
  the host's touch state.
- **Fixed values.** The values GBATEK leaves open (below) are fixed numbers of ours; no value is
  random.

## Edge cases

- A byte with bit 7 clear is padding and starts nothing (GBATEK: the start bit "must be set to
  access the control byte"; what the chip does with a byte without it is our reading, below).
- Reading the reply of a conversion that was never started returns zero.
- A control byte that selects a differential conversion on a channel that has none (0, 2, 6, 7)
  returns zero (see "Unverified").
- A reply that has been read to its end keeps returning zero.
- A write to the battery register (1) is dropped (GBATEK marks it read-only; see "Unverified").
- A read of a register keeps returning its value for every further byte of the session.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware, or a comparison with the reference emulator used as an oracle.

- **The TSC's power-on mode.** GBATEK states none; the model starts in mode 0 (pen output
  enabled). `spi/touch/`.
- **The conversion time.** GBATEK gives only the clock limits; the model converts instantly.
  `spi/touch/`.
- **Z1 and Z2.** GBATEK gives no values. The model returns fixed values while touched (Z1 `0600h`,
  Z2 `0A00h`) and Z1 `000h`, Z2 `FFFh` when released. `spi/touch/`.
- **The temperature channels.** Fixed values of ours: TP0 `0600h`, TP1 `068Eh`, which GBATEK's
  difference formula turns into about 297 K. `spi/touch/`.
- **The microphone channel's value** (`0800h`, mid-range). `spi/touch/`.
- **A differential conversion on a channel that only exists single-ended** returns zero here;
  GBATEK says the channel "can only be accessed" in the other mode. A single-ended conversion of
  X, Y, Z1 or Z2 returns the same value as the differential one. `spi/touch/`.
- **8-bit results** are the top eight bits of the 12-bit result. `spi/touch/`.
- **A command that arrives before the reply is done** (a control byte in the byte right after the
  control byte, earlier than GBATEK allows): the model starts the new conversion and drops what was left of
  the old reply. `spi/touch/`.
- **Chip select going high mid-reply** drops the rest of the reply; the power-down mode stays.
  `spi/touch/`.
- **What the control byte's own transfer receives** when nothing is pending: zero. `spi/touch/`.
- **The pen line's timing.** The bit follows the touch state latched at the start of the frame and
  the mode set by the last control byte; the real line is analog and can glitch when the screen is
  newly pressed or released (GBATEK's note). `io/keypad/`.
- **The power-management device's answer to the index byte** (zero here) and to extra data bytes
  (the register again). `spi/power/`.
- **A write to the read-only battery register** is dropped, and a byte with the start bit clear is
  padding. `spi/power/`, `spi/touch/`.
- **The battery status** reads "power good" (0) always. `spi/power/`.
- **Which bits of register 0 a real original console stores.** GBATEK lists bits 0-6; the model
  stores exactly those. `spi/power/`.
- **Register 0's start value, `0Dh`** (amplifier and both backlights on): GBATEK gives none; the
  value is what a program is assumed to find after the firmware boot that direct boot skips.
  `spi/power/`.
