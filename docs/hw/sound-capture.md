# Sound capture: what the two units write, and channel addition

Sources: GBATEK "DS Sound Capture" (SNDCAPxCNT, SNDCAPxDAD, SNDCAPxLEN, the shared timers, Capture
Bugs, ch(a)+ch(b) Channel Addition), "DS Sound Block Diagrams" (where the capture and the addition
sit), "DS Sound Notes" (Capture Clipping/Rounding, Max Output Levels, Channel/Mixer Bit-Widths).
Used by: `crates/core/src/spu/`, `crates/core/src/machine/`   Phase: 6 (#46)

`sound-registers.md` covers the capture registers a program writes and the busy bit. `spu.md` covers
the channels and the mixer. This note covers what a running capture does to memory and to the
mixer: which value it takes, how it turns it into PCM16 or PCM8, where it writes, and what the
addition mode (SNDCAPxCNT bit 0) changes.

Naming, as in GBATEK: for capture 0, `a` = channel 0, `b` = channel 1, `x` = 0. For capture 1,
`a` = channel 2, `b` = channel 3, `x` = 1. `ch(a)` is the output of channel `a` after its volume
divider and volume factor and before its panning; it is a 16.8 value that is **not** clipped.

## Sources

SNDCAPxCNT bit 1 picks the value to capture (the register layout is in `sound-registers.md`):

- **Bit 1 = 0, the mixer.** Capture 0 takes the left mixer, capture 1 the right mixer. The value is
  the sum of the 16 channels **before** the master volume, which is 20.8 bits wide; it is clipped
  to `-8000h`..`+7FFFh` (16.8).
- **Bit 1 = 1, channel `a`.** Capture 0 takes channel 0, capture 1 channel 2: `ch(a)`, 16.8,
  without clipping. This is the source the two hardware bugs below belong to.

## Timing and length

- **No capture timer.** Capture 0 runs at the sample rate of channel 1 (SOUND1TMR), capture 1 at
  that of channel 3 (SOUND3TMR). The captured data is meant to be played back by those channels.
  One capture sample takes `4 * (10000h - TMR)` master cycles (`sound-registers.md`, "Model").
- **`LEN`** is in words, `1..FFFFh`; 0 counts as 1. A word holds two PCM16 samples or four PCM8
  samples, so the buffer holds `2 * LEN` or `4 * LEN` samples.
- **`DAD`** is the first address written and, for a loop capture, the address it returns to.
- **Loop (bit 2 = 0)** writes `LEN` words, then starts again at `DAD`, for ever. **One-shot
  (bit 2 = 1)** writes `LEN` words, stops, and clears bit 7 (the busy bit).
- **Writing 0 to bit 7** stops the capture at once.

## Format

- **PCM16 (bit 3 = 0).** One signed 16-bit value per sample, written as a halfword at
  `DAD + 2 * i`.
- **PCM8 (bit 3 = 1).** The 16.8 value is divided by `100h`, which gives an 8-bit integer part and
  a 16-bit fraction. If the most significant bit of the fraction is set, the value is rounded
  toward zero (a positive value is rounded down, a negative value up); then the fraction is
  dropped. One byte at `DAD + i`.
- GBATEK states the rounding rule for PCM8 only. What PCM16 does with the 8-bit fraction of the
  16.8 value is not stated (see "Unverified").

## The addition mode

Addition is on only when **both** SNDCAPxCNT bit 0 and bit 7 are 1 (the capture is running). The
unit then adds `ch(b)` to `ch(a)`. The addition has two outputs that differ.

**For the capture** (source = channel `a`, bit 1 = 1):

- The addition is always done, whatever SOUNDCNT bits 12 and 13 say.
- If channel `a` is disabled, the result is plain `ch(b)`.
- The result is 16 bits plus fraction and **can overflow** (the overflow bug below).

**For the mixer** (toward the speakers, and for a capture whose source is the mixer):

- `ch(b)` is muted if channel `a` is disabled.
- `ch(b)` is muted if SOUNDCNT bit 12 (channel 1) or bit 13 (channel 3) says "not to mixer".
- The result is 17 bits plus fraction and does **not** overflow.
- Because it changes the mixer, addition also changes the **other** capture unit when that unit
  reads the mixer. It changes the speaker output.

In the block diagram, channel `a`'s path (volume, add, panning) feeds the capture's source
selector, and channel `b`'s own panning input comes through a selector (the "Sel" box) that sits
after the addition. What that selector chooses in addition mode is not written down (see
"Unverified").

## The two bugs

Both happen only when the source is channel `a` (bit 1 = 1), and neither touches the speakers. A
capture of the mixer is correct.

1. **Both negative bug** (bit 1 = 1, bit 0 = 0, addition off). If `ch(a)` and `ch(b)` are both
   below 0, the captured value is `-8000h`. Otherwise it is plain `ch(a)`, not affected by
   `ch(b)`. `ch(b)` is read even though it is not added.
2. **Overflow bug** (bit 1 = 1, bit 0 = 1, addition on). The sum `ch(a) + ch(b)` is not clipped to
   `-8000h`..`+7FFFh`; it is ANDed with `FFFFh`, so a sum beyond the range wraps and loses its sign.

## Model (not hardware)

These are choices of the emulator, made so that capture, busy bit and the mixer share one clock.

- **The capture is written inside the output-sample event** (`spu.md`, "Output samples"), not by an
  event of its own. When that event has computed the 16 channel samples and the mixer output, each
  running capture writes **every slot whose time has passed since the previous output sample**,
  and each of those slots gets the source value of **this** output sample. The slot index is a pure
  function of the start time, the latched period and `now`; the busy bit uses the same formula, but
  the memory is written only at the output-sample event, so a slot can reach memory up to one
  output sample (2048 master cycles) after the bit says it is done (see "Unverified").
- **A slot is written when the shared clock completes it**: slot `i` is due once
  `(i + 1) * period` cycles have passed since the start. A one-shot's busy bit clears when its last
  slot is completed; that slot reaches memory at the next output-sample event.
- **One sample of delay.** A capture written in an output-sample event is read by channel 1 or 3 at
  the next one at the earliest. A program that plays the capture buffer back with channel 1 or 3
  hears it one output sample (2048 master cycles) late. This is the model.
- **What a capture latches at its start**: the period from the shared TMR, LEN, the format, the
  repeat mode and DAD. A write to any of them while the capture runs does not move it (as for a
  channel, `spu.md`). Bit 1 and bit 0 are read live.
- **Writes go to memory at once**, one sample at a time, along the path a DMA write of the ARM7
  takes (`dma.md`), so the shared WRAM split, the mirrors and the I/O window behave as for a DMA.
  The 8-word FIFO of the diagram is not modeled.
- **The addition for the mixer** is on while the capture's busy bit reads set and SNDCAPxCNT bit 0
  is set. Channel `a`'s mixer value is then `a`'s panning applied to the sum of the two channels'
  values after volume (before panning), and channel `b` reaches the mixer only through that sum:
  it is left out of the sum when channel `a` is not busy or SOUNDCNT bit 12 (channel 1) or 13
  (channel 3) sends it away from the mixer. The speaker output selection of SOUNDCNT bits 8-11
  still sees channel `b`'s own panned value.
- **The addition for the capture** (source = channel `a`) is `a + b` with the integer part wrapped
  to 16 bits (the overflow bug); the 8 fraction bits stay. For PCM8 the wrap happens before the
  conversion. With addition off, the both-negative bug applies.
- **A rate faster than the output rate** (a TMR near `FFFFh`) has several slots to fill in one
  output sample; all of them get the same value.
- **The mixer source ignores SOUNDCNT bit 15 and the speaker gates.** The capture reads the sum
  before the master stage, whether or not the master enable is set (`spu.md`, "The gates").
- **The clip keeps the fraction**: a 16.8 value is clipped to `-8000h.00` .. `7FFF.FFh`.
- **Disabled** for "ch(a) disabled" means "not busy" in the model: the channel is not playing.
- **A write of 1 to bit 7 while the capture is busy** keeps the current run (the existing
  behavior of the busy bit).

## Edge cases

- `LEN` = 0 captures one word.
- A one-shot capture of one word with PCM8 writes four bytes.
- A loop capture never clears bit 7; a one-shot clears it when the last slot has been written.
- Stopping and restarting a capture starts again at `DAD`.
- A capture whose `DAD` points at memory the ARM7 cannot write (ROM, unmapped) drops the writes, as
  a DMA does; it does not stop.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware (`cleancart.nds` where it fits), or a comparison with the reference emulator used as an
oracle.

- **The start delay of a capture.** GBATEK gives the start delay of a channel (1, 3 or 11 samples)
  but none for a capture; whether the first sample is written at once or at the next sample of the
  shared channel. None is modeled. — `spu/` — verify by a test ROM reading the buffer after a
  known number of cycles.
- **FIFO granularity and timing.** The diagram shows a FIFO between the capture and memory; its
  depth and the moment of the memory write are not given. Samples are written one at a time and at
  once here. — `spu/`, `machine/events.rs`.
- **When the last slot reaches memory.** The busy bit of a one-shot clears when the shared clock
  completes the last slot; the write happens at the next output-sample event, up to 2048 master
  cycles later, so a program that polls the bit and then reads the buffer can see the last slot's
  old value. Hardware timing is not given. — `machine/events.rs`, `spu/`.
- **A capture rate faster than the output rate.** The slots between two output samples all get one
  value. — `machine/events.rs`.
- **The mixer source while the master enable is off, or the speakers are gated.** The capture reads
  the sum regardless here. — `spu/`.
- **The fraction of a negative PCM8 value.** The model rounds on the two's complement fraction, so
  -5.5 steps gives -5 and -5.50002 gives -6; the rule could instead read the magnitude. — `spu/mixer/`.
- **The clip keeps the fraction.** The mixer value is clipped to `-8000h.00` .. `7FFF.FFh`; GBATEK
  names the range without the fraction. — `spu/mixer/`.
- **The value of a channel `a` that is not playing** is 0 here. — `spu/`.
- **PCM16 rounding.** The rule for PCM8 is stated; PCM16 drops the fraction by flooring here. —
  `spu/`.
- **What "channel `a` disabled" means** for the addition. The model uses "not busy"; the SOUNDCNT
  or SOUNDxCNT enable bit is another reading. — `spu/`.
- **Channel `b`'s own path in addition mode.** Whether the "Sel" box still sends `ch(b)` to the
  mixer and to the output selection of SOUNDCNT bits 8-11. The model gives channel `a`'s mixer
  contribution the pan of `a` applied to `vol(a) + vol(b)`, and mutes `ch(b)`'s own contribution
  as the text says. — `spu/mixer/`.
- **The AND of the overflow bug for PCM8 and the fraction.** GBATEK gives `FFFFh` for the 16-bit
  value; the model wraps the integer part and keeps the fraction. — `spu/mixer/`.
- **A one-shot capture's last event.** The addition for the mixer stops when the busy bit clears;
  the final slots of that run are still written with the capture-side addition. — `spu/`.
- **A write of 1 to bit 7 while busy.** It keeps the current run here; GBATEK says nothing. —
  `spu/`.
- **A write to TMR, LEN, DAD or the format while a capture runs.** Latched at the start here. —
  `spu/`.
- **The source value when channel `a` is not playing.** The model takes 0 for `ch(a)` (and the
  mixer value as it is). — `spu/`.
