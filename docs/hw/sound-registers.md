# Sound registers, capture registers and POWCNT2

Sources: GBATEK "DS Sound Channels 0..15" (SOUNDxCNT, SOUNDxSAD, SOUNDxTMR, SOUNDxPNT, SOUNDxLEN, the
sample counts and the minimum length), "DS Sound Control Registers" (SOUNDCNT, SOUNDBIAS), "DS Sound
Capture" (SNDCAPxCNT, SNDCAPxDAD, SNDCAPxLEN, the shared timers), "DS Power Control" (POWCNT2),
"BIOS Sound Functions" (SoundBias, SWI 08h on the ARM7). The sample timing is derived here from the
timer formula GBATEK gives; see "Model".
Used by: `crates/core/src/spu/`, `crates/core/src/io/power/`, `crates/core/src/hle/swi/`
Phase: 5 (#42)

The ARM7 programs a sound driver runs write these registers and then wait on their status bits.
ninge produces no sound yet: this note covers the registers a program can write and read back, and
the one thing a program can observe without audio, the status bit that says a channel or a capture
is still running. The mixer, the sample output and the capture data are Phase 6.

## Scope

Implemented (#42): the 16 channels' registers, SOUNDCNT, SOUNDBIAS, the two capture units'
registers, the busy bits of channels and captures, POWCNT2, and the SoundBias SWI.

The mixer, the volume and panning, the PSG and noise generation, reading the sample data (PCM8,
PCM16, ADPCM) and the gates (SOUNDCNT, POWCNT2, the sound amplifier of the power-management device)
are in `spu.md` (#45), and what the capture units write to memory is in `sound-capture.md` (#46).
Not implemented (`TODO(phase6)`): the addition of channel 1 or 3 into channel 0 or 2 (#46, the
last part of `sound-capture.md`), and the BIOS table SWIs (sine, pitch and
volume tables, which are BIOS data and need a plan of their own).

## Who sees them

All of these registers are ARM7 only (GBATEK labels every one NDS7). The ARM9 does not see them: a
read there returns 0 and a write is dropped, as for any address no device answers (`memory-map.md`).

## Registers

### The channels: `0400_0400h`-`0400_04FFh`

16 channels of 16 bytes. Channel `x` is at `0400_0400h + 10h * x`.

| Offset | Name | Width | Access |
|---|---|---|---|
| +0 | SOUNDxCNT | 32 | R/W |
| +4 | SOUNDxSAD | 32 | W |
| +8 | SOUNDxTMR | 16 | W |
| +Ah | SOUNDxPNT | 16 | W |
| +Ch | SOUNDxLEN | 32 | W |

#### SOUNDxCNT bits

| Bits | Meaning |
|---|---|
| 0-6 | Volume multiplier, 0 (silent) to 127 |
| 7 | Not used, reads 0 |
| 8-9 | Volume divider: 0 = none, 1 = /2, 2 = /4, 3 = /16 |
| 10-14 | Not used, read 0 |
| 15 | Hold: keep the last sample after a one-shot ends |
| 16-22 | Panning, 0 (left) to 127 (right) |
| 23 | Not used, reads 0 |
| 24-26 | PSG wave duty |
| 27-28 | Repeat mode: 0 = manual, 1 = loop, 2 = one-shot, 3 = prohibited |
| 29-30 | Format: 0 = PCM8, 1 = PCM16, 2 = IMA-ADPCM, 3 = PSG (channels 8-13) or noise (14-15) |
| 31 | Start / busy: 1 = running |

So the readable mask of SOUNDxCNT is `FF7F_837Fh`.

#### The write-only registers

- **SOUNDxSAD:** the source address, 27 bits (bits 0-26), word aligned, so bits 0-1 are always
  zero. Bits 27-31 are unused.
- **SOUNDxTMR:** 16 bits. The sample rate is the ARM7 clock divided by two, divided by `10000h -
  TMR`: a larger value is a faster sample.
- **SOUNDxPNT:** 16 bits, the loop start, counted in words.
- **SOUNDxLEN:** 22 bits (bits 0-21), the length, counted in words. Not used in PSG mode.

### SOUNDCNT: `0400_0500h`, 16 bits, R/W

| Bits | Meaning |
|---|---|
| 0-6 | Master volume |
| 7 | Not used, reads 0 |
| 8-9 | Left output from: 0 = left mixer, 1 = channel 1, 2 = channel 3, 3 = channels 1 and 3 |
| 10-11 | Right output from, the same encoding |
| 12 | Channel 1 is not sent to the mixer (1 = not sent) |
| 13 | Channel 3 is not sent to the mixer |
| 14 | Not used, reads 0 |
| 15 | Master enable |

Readable mask `BF7Fh`. The upper 16 bits of the word are unused and read 0.

### SOUNDBIAS: `0400_0504h`, R/W

Bits 0-9 hold the bias (`000h`-`3FFh`, usually `200h`); the added value that turns the signed mixer
output into an unsigned level. Bits 10-31 are unused and read 0. GBATEK notes that the bias output
is on even when the master enable is off.

### The capture units: `0400_0508h`-`0400_051Fh`

| Address | Name | Width | Access |
|---|---|---|---|
| `0400_0508h` | SNDCAP0CNT | 8 | R/W |
| `0400_0509h` | SNDCAP1CNT | 8 | R/W |
| `0400_0510h` | SNDCAP0DAD | 32 | R/W |
| `0400_0514h` | SNDCAP0LEN | 16 | W |
| `0400_0518h` | SNDCAP1DAD | 32 | R/W |
| `0400_051Ch` | SNDCAP1LEN | 16 | W |

#### SNDCAPxCNT bits

| Bits | Meaning |
|---|---|
| 0 | Add the output of channel 1 (capture 0) or channel 3 (capture 1) to channel 0 or 2; works only while bit 7 is also set |
| 1 | Source: 0 = the mixer (left for capture 0, right for capture 1), 1 = channel 0 or 2 |
| 2 | Repeat: 0 = loop, 1 = one-shot |
| 3 | Format: 0 = PCM16, 1 = PCM8 |
| 4-6 | Not used, read 0 |
| 7 | Start / busy |

Readable mask `8Fh`.

- **SNDCAPxDAD:** the destination address, 27 bits, word aligned (bits 0-1 zero), also the
  restart address of a loop capture.
- **SNDCAPxLEN:** 16 bits, the buffer length in words. GBATEK: a length of 0 is taken as 1.
- **Timer.** There is no capture timer. Capture 0 runs at channel 1's sample rate (SOUND1TMR) and
  capture 1 at channel 3's (SOUND3TMR).

GBATEK also describes two hardware bugs in the capture data when the source is a channel; they
change the data written, which is Phase 6. The data, the writes and the bugs are specified in
`sound-capture.md`.

### POWCNT2: `0400_0304h`, R/W

| Bits | Meaning |
|---|---|
| 0 | Internal speakers: 0 = off, 1 = on (initial setting 1). Headphones are not affected |
| 1 | Wi-Fi: 0 = off, 1 = on (initial setting 0). Off disables the Wi-Fi registers |
| 2-31 | Not used |

Readable mask `0003h`. The register is 2 bytes wide in GBATEK's listing, next to POWCNT1 (ARM9) at
the same address; the ARM7 sees only this one.

### SoundBias: SWI `08h` (ARM7)

`R0` is the target: 0 means level `000h`, any other value means `200h`. `R1` is the delay count:
the DS takes it as a parameter, where the GBA has no `R1` and always uses a fixed delay of 8. The
BIOS steps SOUNDBIAS up or down one at a time, with a short delay between steps, until it reaches
the target. The bits above bit 9 are kept as they were. Nothing is returned.

## Behavior

### The status bit of a channel

Bit 31 of SOUNDxCNT is both the start control and the status. A program writes 1 to start a
channel and polls the bit to learn when a one-shot sound has ended. The bit clears itself when:

- the repeat mode is one-shot (2), and
- the sound has played its whole length.

The length of a one-shot is the sum of PNT and LEN, in words. Call that `N`. The number of samples
GBATEK gives for `N` words is:

| Format | Samples |
|---|---|
| PCM8 | `4 * N` |
| PCM16 | `2 * N` |
| IMA-ADPCM | `8 * (N - 1)` (the first word is the header) |

A loop sound (repeat mode 1) plays the first PNT words once and then repeats the LEN part for ever,
so it never ends. Writing 0 to bit 31 stops a channel at once, whatever its mode.

### The status bit of a capture

Bit 7 of SNDCAPxCNT works the same way. A one-shot capture (bit 2 = 1) stops after `LEN` words (a
length of 0 counts as 1) and the bit clears. A word holds 2 samples in PCM16 and 4 in PCM8. A loop
capture (bit 2 = 0) never ends. Writing 0 to bit 7 stops it at once.

## Model (not hardware)

These are choices of the emulator, because no sound is produced and the exact start-up of a
channel is not documented.

- **One sample takes `4 * (10000h - TMR)` master cycles.** The sample clock is the ARM7 clock
  halved (GBATEK's formula), and one ARM7 cycle is two master cycles (`scheduler.md`).
  A TMR of 0 gives `10000h` cycles per sample, the slowest rate.
- **The end is computed once, at the start.** When bit 31 goes from 0 to 1 the model records
  `end = now + samples * 4 * (10000h - TMR)`, with `now` the master time of the write. A read of
  the register reports busy while the channel has started and `now < end`. No scheduler event and
  no timer are involved. The playback of a channel (`spu.md`, "Position") uses the same end, so
  the busy bit and the output agree. A capture is the same, with a start time and the slots written
  so far instead of an end (`sound-capture.md`, "Model").
- **A capture is the same**, with the sample rate of channel 1 (capture 0) or channel 3
  (capture 1) taken from their TMR at the moment the capture starts.
- **The busy bit never clears** for: loop mode, PSG and noise (format 3), the manual (0) and
  prohibited (3) repeat modes, and a one-shot whose `PNT + LEN` is below 4 words. For the last
  case GBATEK says so itself: below four words the busy bit stays set for ever and nothing plays.
- **The write-only registers read 0** and the values written are kept inside the model (the debugger
  shows them).
- **Writing bit 31 as 1 while it is already 1** does not restart the channel: the end time stays.
- **The SoundBias SWI** lands on the target level at once and has no delay.
- **Start values.** SOUNDCNT, SOUNDBIAS and every channel and capture register start at 0. POWCNT2
  starts at `0001h`, GBATEK's initial setting.

## Edge cases

- A read of an address in `0400_0400h`-`0400_051Fh` that no register uses (the gap at
  `0400_050Ah`-`0400_050Fh`, `0400_0511h`-`0400_0513h`, and so on) returns 0 and drops the write
  (see "Unverified").
- Unused bits read 0 whatever was written.
- A channel with a TMR of `FFFFh` plays one sample per 4 master cycles (the fastest), and one with
  `0` one per `4 * 10000h`.
- The ARM9 reads 0 from every address of this note except POWCNT1 at `0400_0304h`, which is a
  different register (`io` map).

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware, or a comparison with the reference emulator used as an oracle.

- **A start delay before playback begins.** None is modeled; the real channels may take some
  cycles before the first sample. — `spu/` — verify by a test ROM timing the busy bit.
- **TMR, PNT or LEN written while a channel is busy.** The end time is not recomputed. — `spu/`.
- **The manual (0) and prohibited (3) repeat modes** are treated as never ending. GBATEK says
  nothing about their busy bit. — `spu/`.
- **PSG on channels 0-7 and noise on channels other than 14-15.** GBATEK says PSG works on 8-13 and
  noise on 14-15 only; the model keeps the busy bit set in format 3 on any channel. — `spu/`.
- **What the write-only registers read.** 0 here; the real bus may return something else. — `spu/`.
- **The values after direct boot** of SOUNDCNT, SOUNDBIAS and POWCNT2. Zero here (POWCNT2 `0001h`
  per GBATEK's initial setting); the firmware's boot code probably sets some of them. — `spu/`,
  `io/power/`.
- **The SoundBias SWI's delay.** The BIOS delays between steps; here there is none (as for
  WaitByLoop, `hle-swi.md`) and the register reaches the target at once, so the intermediate
  values are never visible. — `hle::swi`.
- **The timing of a capture**, in particular whether it starts at once or at the next sample of the
  shared channel, and that the rate is read at the start. — `spu/`.
- **Unused addresses inside the window** read 0 and drop writes here; GBATEK gives no behavior for
  them. — `spu/`.
- **A partial write** (a byte or halfword into a 32-bit register) changes only those bytes, as on
  any register of this bus; the write-only registers are not read back to merge. — `spu/`.
