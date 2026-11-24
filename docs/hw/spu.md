# The sound unit: channels, generation and mixing

Sources: GBATEK "DS Sound Channels 0..15", "DS Sound Control Registers", "DS Sound Block Diagrams",
"DS Sound Notes" (the delayed start, the stop, the hold flag, the 7-bit factors, the bit widths,
PSG and noise, PCM, IMA-ADPCM), "DS Sound Capture" (only where it touches the mixer), "DS Power
Control" (POWCNT2) and "DS Power Management Device" (register 0).
Used by: `crates/core/src/spu/`, `crates/core/src/machine/`, `crates/core/src/io/power/`,
`crates/core/src/spi/power/`   Phase: 6 (#45)

`sound-registers.md` covers the registers a program writes and reads back. This note covers what
the unit does with them: how a channel turns its source data into samples, how the 16 channels are
mixed into a stereo output, and what silences it. The capture units write the output to memory and
feed channels 1 and 3 (#46); they are in `sound-capture.md`, not in this note beyond the lines
marked "capture".

## Clocks

- A channel's **sample clock** is the ARM7 clock halved, divided by `10000h - TMR` (GBATEK's
  formula). In this codebase one ARM7 cycle is two master cycles (`scheduler.md`), so a channel
  takes a new sample every `4 * (10000h - TMR)` master cycles. This is the rule
  `sound-registers.md` already uses for the busy bit.
- The **mixer** works on a faster clock, about 1.04876 MHz with a 24-bit amplitude (GBATEK), and
  its result goes through a PWM stage that makes 10-bit samples at about 32.768 kHz. The ratio of
  the two figures is 32. The 10-bit output is what reaches the speakers and what this note calls an
  **output sample**.

## Output samples

The emulator makes one stereo output sample per **2048 master cycles** (1024 ARM7 cycles, about
32.73 kHz). It is a model choice that sits between GBATEK's rounded figures; see "Unverified". At
each output sample the mixer reads the current sample of every running channel, mixes, applies the
master volume and the bias, and clips to 10 bits. A channel holds its last sample until its own
clock produces the next one, so a slow channel is read several times and a fast one is
subsampled.

The output is a pair of `i16`, left then right. A 10-bit level `L` (`000h`-`3FFh`, bias included)
becomes `(L - 200h) * 64`: the medium level maps to zero, the range is `-8000h`-`7FC0h`. The
resampling to a host device rate is the frontend's job (#47); the core does not resample.

## The channels

### Position

A channel starts when bit 31 of SOUNDxCNT goes from 0 to 1. The master time of that write is the
channel's **start**. From there the channel's sample clock counts samples, each one `4 * (10000h -
TMR)` master cycles long. While TMR is not written, the samples elapsed since the start are
`floor((now - start) / (4 * (10000h - TMR)))`, with `now` the master time of the query. The sample
a channel outputs is a function of that count and the registers latched at the start: nothing is
stepped, so the busy bit and the output cannot disagree.

**A write to SOUNDxTMR while the channel plays** changes the rate from the next sample on: the
sample in progress keeps the length it began with, and every sample that begins after it takes
`4 * (10000h - TMR)` cycles of the new value. The count does not jump, so the data position goes on
from where it was, faster or slower. This is how a program changes the pitch of a note that is
already sounding (a bend, a vibrato, a trill inside one note). The end of a one-shot moves with it:
the samples still to play are counted at the new length. See "Model" and "Unverified".

The data position for a given sample count depends on the format and the repeat mode (PNT and LEN
are in words, SAD is the first byte):

| Format | Samples per word | Samples before the loop start (PNT part) | Samples in the loop (LEN part) |
|---|---|---|---|
| PCM8 | 4 | `4 * PNT` | `4 * LEN` |
| PCM16 | 2 | `2 * PNT` | `2 * LEN` |
| IMA-ADPCM | 8 | `8 * (PNT - 1)` (the first word is the header) | `8 * LEN` |

- **One-shot** (repeat mode 2): the sound plays the PNT part and then the LEN part once, so its
  length is `PNT + LEN` words (GBATEK). The busy bit clears when the last sample has been played.
- **Loop** (repeat mode 1): the PNT part plays once, then the LEN part repeats for ever. The busy
  bit never clears.
- A one-shot whose `PNT + LEN` is below 4 words never plays a sample: GBATEK says the busy bit
  stays set and nothing is output.
- **After the end of a one-shot** the channel outputs 0, or its last sample for ever if the Hold
  flag (bit 15) is set. Writing 0 to bit 31 stops the channel at once and its output is 0.
- **PCM8** samples are signed bytes, `-80h`-`7Fh`; the output level equals `NN00h` as PCM16.
  **PCM16** samples are signed little-endian halfwords. A word holds 4 and 2 samples.
- **IMA-ADPCM**: see below.

The channel reads its data from the ARM7's view of memory, at `SAD + 4 * (word index)` bytes,
sample by sample. A read the ARM7 could not make (an unmapped address) returns what the ARM7 bus
returns there (`memory-map.md`).

### IMA-ADPCM

The first word at SAD is the header: bits 0-15 are the initial `Pcm16` value (`-7FFFh`-`7FFFh`), bits
16-22 the initial table `Index` (0-88), bits 23-31 are unused. The 4-bit values follow, two to a byte,
the low nibble first. Each value `Data4` (a 4-bit pattern) is decoded:

```
Diff  = AdpcmTable[Index] / 8
if Data4 & 1: Diff += AdpcmTable[Index] / 4
if Data4 & 2: Diff += AdpcmTable[Index] / 2
if Data4 & 4: Diff += AdpcmTable[Index]
if Data4 & 8 == 0: Pcm16 = min(Pcm16 + Diff, 7FFFh)
else:              Pcm16 = max(Pcm16 - Diff, -7FFFh)
Index = clamp(Index + IndexTable[Data4 & 7], 0, 88)
```

Each division is an integer division that drops the remainder; this is the "rounding error" GBATEK
says is how the hardware really works. The clamp on the add side leaves `-8000h` alone (possible
when the initial value is `-8000h`) and the clamp on the subtract side lifts `-8000h` to `-7FFFh`.

`IndexTable[0..7] = -1, -1, -1, -1, 2, 4, 6, 8`.

`AdpcmTable[0..88]` (hexadecimal):

```
0007 0008 0009 000A 000B 000C 000D 000E 0010 0011 0013 0015
0017 0019 001C 001F 0022 0025 0029 002D 0032 0037 003C 0042
0049 0050 0058 0061 006B 0076 0082 008F 009D 00AD 00BE 00D1
00E6 00FD 0117 0133 0151 0173 0198 01C1 01EE 0220 0256 0292
02D4 031C 036C 03C3 0424 048E 0502 0583 0610 06AB 0756 0812
08E0 09C3 0ABD 0BD0 0CFF 0E4C 0FBA 114C 1307 14EE 1706 1954
1BDC 1EA5 21B6 2515 28CA 2CDF 315B 364B 3BB9 41B2 4844 4F7E
5771 602F 69CE 7462 7FFF
```

The decoder carries `Pcm16` and `Index` from sample to sample, so it is state: it is advanced as the
sample count grows and it cannot be computed from the count alone. The header seeds it at the start.
**At the loop**: when the playback reaches the end of the LEN part and wraps to the loop start, the
`Pcm16` and `Index` that the decoder had when it first reached the loop start are put back, so
every pass through the loop decodes the same values. The loop start is therefore **snapshotted**
the first time the data position reaches `PNT`. GBATEK adds: put the loop start at the data, not at the
header, and never change it during playback.

### PSG (channels 8-13)

Format 3 on channels 8-13 is a square wave. The wave is eight samples long, so its frequency is the
sample rate divided by 8. The bits 24-26 of SOUNDxCNT pick the duty `D` (0-7): of the eight
samples, the first `7 - D` are LOW and the last `D + 1` are HIGH, except `D = 7`, which is LOW for
all eight (GBATEK's table: 12.5% to 87.5%, then 0%). The wave always starts at the beginning of the
LOW part when the channel starts. HIGH is the PCM16 value `+7FFFh` and LOW is `-7FFFh`. PSG has no
length: the repeat mode and LEN are ignored and the channel never ends.

| D | HIGH samples of 8 |
|---|---|
| 0 | 1 |
| 1 | 2 |
| 2 | 3 |
| 3 | 4 |
| 4 | 5 |
| 5 | 6 |
| 6 | 7 |
| 7 | 0 |

### Noise (channels 14-15)

Format 3 on channels 14 and 15 is noise, one new output per sample (not per 8). A 15-bit register
`X` starts at `7FFFh` when the channel starts. At each sample: shift `X` right by one; if a 1 was
shifted out, the output is LOW and `X` is XORed with `6000h`; otherwise the output is HIGH. HIGH and
LOW are `+7FFFh` and `-7FFFh`, as in PSG. The noise register is state.

### Format 3 on channels 0-7

GBATEK says PSG works on 8-13 and noise on 14-15 only, and nothing else about format 3. See
"Unverified".

## Volume and panning

Each channel's sample `s` (a PCM16 value) goes through, in this order (GBATEK's "Channel/Mixer
Bit-Widths"):

1. **Divider** (SOUNDxCNT bits 8-9): divide by 1, 2, 4 or 16. The result keeps 4 fraction bits.
2. **Volume** (bits 0-6): multiply by `N / 128`, where registers 0-126 mean `N` = 0-126 and register
   127 means `N = 128` (GBATEK's "7bit Volume and Panning Values").
3. **Panning** (bits 16-22): the left factor is `(128 - N) / 128` and the right factor `N / 128`,
   with the same reading of 127 as 128, so a pan of 127 is fully right and silent on the left, and a
   pan of 64 is half on both.
4. **Rounding down**: the fraction beyond 8 bits is dropped, rounding toward minus infinity.

In integers, with `v` the divider shift (0, 1, 2 or 4): `ch = ((s << 4 >> v) * vol * pan) >> 10`,
an arithmetic shift, `vol` and `pan` as above. `ch` has 8 fraction bits (units of 1/256).

The left and right values of channel `x` are `chL[x]` and `chR[x]`.

## The mixer

1. **Add** the 16 channels for each side: `mixL = sum chL[x]`, `mixR = sum chR[x]` (20 integer bits
   and 8 fraction bits). A channel that has not started, or has ended without Hold, adds 0.
   Channels 1 and 3 add 0 to the mixer if SOUNDCNT bit 12 and bit 13 are set (1 = not sent).
2. **Choose the source** of each output (SOUNDCNT bits 8-9 for left, 10-11 for right): 0 = the
   mixer, 1 = channel 1 alone, 2 = channel 3 alone, 3 = channel 1 plus channel 3. The chosen
   channels' values are their own `chL` / `chR` from "Volume and panning", taken before the mix.
3. **Master volume** (SOUNDCNT bits 0-6): `M` is 0-126 as written and 128 for 127. The result is
   `(value * M) >> 21` (an arithmetic shift): the factor is `M / 128 / 64`.
4. **Bias**: add SOUNDBIAS (bits 0-9).
5. **Clip** to `000h`-`3FFh`.

The result is the 10-bit level `L` of "Output samples". The widths in GBATEK's table are
`-80000h`-`+7FFF0h` after the mix, `-2000h`-`+1FF0h` after the master volume.

## The gates

Three switches silence the output. All three are digital here, so the core applies them:

- **SOUNDCNT bit 15** (master enable). When off, the mixer output is not used; the level is the
  bias alone. GBATEK: "BIAS output is always enabled, even when Master Enable is off".
- **POWCNT2 bit 0** (internal speakers). When 0, the output is silent. Headphones are not
  affected on the hardware, and the emulator has one output, which it treats as the speakers.
- **The power-management device, register 0** (SPI): bit 0 is the sound amplifier enable and bit 1
  the amplifier mute. With bit 0 clear the sound is silent (DS Lite) or "very silent, but still
  audible" (original DS); with bit 1 set and bit 0 set the sound is silent (original DS).

A silenced output is `0` in both channels: the value of the medium level. The bias-only level of
the first gate is `(bias - 200h) * 64`.

## What this note does not cover

- **Capture** (#46): the capture units read the output of the mixer or of channel 0 or 2, and
  can add channel 1 or 3 (SNDCAPxCNT bits 0 and 7), which also changes the mixer. The behavior
  is specified in `sound-capture.md` (#46), the writes to memory and the addition mode. The
  mixer of this note is the one without addition; with it on, `sound-capture.md` says which
  channels move.
- **BIOS table SWIs** (sine, pitch and volume tables): see `hle-swi.md`, "The sound table SWIs".
- **The 15 ms rule**: GBATEK says no sound is to be output during the first 15 ms after the sound
  circuit is powered. That is a rule for the program, not a behavior to model.

## Model (not hardware)

- **One output sample per 2048 master cycles**, from the ratio in "Clocks".
- **The channel starts to play at the write.** GBATEK says a sound starts late: PSG and noise after
  1 sample, PCM after 3 samples, ADPCM after 11 samples (3 plus 8 for the header), and the busy
  bit clears at the start of the last sample period, not at its end. The model has none of
  these delays, so that the busy bit of `sound-registers.md` and the output share one position; see
  "Unverified".
- **The registers are latched at the start, except TMR.** SAD, PNT, LEN, the format and the repeat
  mode are read when the channel starts and a write while it plays does not move it. Volume,
  divider, panning, hold and duty are read live. GBATEK says nothing either way.
- **TMR is read at each sample boundary.** GBATEK calls SOUNDxTMR a timer value, and the model
  treats it as the reload of a counter: a write does not touch the count in progress, so the
  sample in progress ends when it would have, and the new value sets the length of the samples
  after it. Two writes inside one sample leave only the last one. The model keeps the count of the
  sample that begins next, the master time it begins and the length in force; the samples elapsed
  are that count plus the whole lengths since. A query at a time before that boundary gets the
  sample in progress at the write, also when the query's time is before the write itself (the ARM7
  can write at a time past the master clock).
- **The manual (0) and prohibited (3) repeat modes play like a loop.** They never end, as the busy
  bit already models.
- **An empty loop** (loop mode with LEN = 0 and a PNT part of at least 4 words) plays the PNT part
  and then outputs 0 for ever.
- **A started channel with its enable bit clear in SOUNDCNT** still plays; the master enable only
  decides what the output shows.
- **The output is `i16`**, from the 10-bit level as in "Output samples".

## Edge cases

- A TMR of `FFFFh` makes a sample per 4 master cycles, faster than the output rate, so a PCM
  channel is subsampled; TMR 0 makes one per `4 * 10000h` cycles.
- A TMR write is two byte writes at one time on the bus. Both fall inside the same sample, so the
  half-written value is never the length of a sample.
- A TMR write to a channel that is stopped, or whose one-shot has ended, only stores the value:
  the next start reads it.
- A TMR write during the last sample of a one-shot does not move the end: no sample begins after
  it.
- Volume 127 and panning 127 read as 128, so `vol * pan` can reach `128 * 128`.
- A PCM8 value of `-80h` is the PCM16 value `-8000h`; an ADPCM header can start at `-8000h`
  (GBATEK) and the add clamp leaves it.
- All the sums use wrapping arithmetic on integers wide enough that nothing wraps: 64 bits for the
  channel products, 32 for the mix.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware (`cleancart.nds` where it fits), or a comparison with the reference emulator used as an
oracle.

- **The output sample period.** GBATEK's 32.768 kHz and 1.04876 MHz do not follow from the ARM7
  clock (33513982 Hz) exactly: 1024 ARM7 cycles give 32728.5 Hz. The 2048 master cycles are the
  nearest whole number of cycles. — `spu/` — verify by timing the output.
- **The start delay** (1 / 3 / 11 samples) and the busy bit clearing at the begin of the last
  sample, both stated by GBATEK, are not modeled. — `spu/`.
- **The order of rounding inside a channel and the mixer.** The steps and widths are GBATEK's; the
  exact point where each fraction is dropped (floor of the shifted product) is read from its table,
  not stated as an equation. — `spu/` — verify by comparing samples with the oracle.
- **The conversion to `i16`** is ours; GBATEK gives only the 10-bit level.
- **The silenced output value**, `0` for the speaker and amplifier gates, and the bias-only level
  when the master enable is off. GBATEK says only that the bias stays on. — `spu/`.
- **The original DS amplifier**, "very silent, but still audible" with bit 0 clear, is treated as
  silent. — `spi/power/`.
- **The Hold flag during a restart.** GBATEK describes the old level being kept for one sample and
  dropping to zero for two more. Not modeled, as the delay is not. — `spu/`.
- **The manual and prohibited repeat modes**, **format 3 on channels 0-7**, and **PNT + LEN
  below 4 words in loop mode**. GBATEK says nothing on the output of these. The model plays the
  first two like a loop, plays silence for format 3 on channels 0-7 (busy stays set), and plays
  nothing for the third. — `spu/`.
- **An empty loop** (LEN = 0 after a PNT part). — `spu/`.
- **Whether the first noise output is the first step or the initial register.** GBATEK gives the
  start value `7FFFh` and the step; the model takes the first step at the first sample of the
  clock (so the first output is LOW and the register `5FFFh`). — `spu/psg/`.
- **Register writes while a channel plays**: SAD, PNT, LEN, the format and the repeat mode are
  latched at the start here. — `spu/`.
- **A TMR write while a channel plays.** GBATEK gives the timer formula and nothing on a write
  during playback. The rate follows the write here because a program was seen to depend on it: it
  wrote SOUNDxTMR several times within one note (the same start, the same source registers),
  which changes the note's pitch only if the playing channel changes its rate. That is an
  observation of a program, not of the hardware. When the new value takes effect is a choice: at
  the next sample here (a reload read at the counter's overflow); at once is the other reading. —
  `spu/` — verify by a test ROM that writes TMR mid-sound and times the busy bit of a one-shot.
- **The phase of PSG and noise on restart** is GBATEK's (LOW first, `X = 7FFFh`); what a write of 1
  to a running channel does is not stated, and here it does not restart. — `spu/`.
- **Whether the header's initial `Pcm16` is itself output.** GBATEK counts `8 * (N - 1)` ADPCM
  samples, which leaves the header out; the model outputs the first decoded value first. — `spu/`.
- **When the ADPCM header is read.** The model reads it when the first output sample after the start
  catches the decoder up, not at the start, so a program that rewrites the header within one output
  sample (2048 master cycles) of starting is read late. — `spu/`.
- **An ADPCM loop whose start is the header** (PNT = 0 or 1). GBATEK says to put it at the data. — `spu/`.
- **What a channel reads from an address the ARM7 bus does not map.** — `spu/`.
