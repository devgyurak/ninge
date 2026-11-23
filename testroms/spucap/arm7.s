@ SPDX-License-Identifier: Apache-2.0
@ Sound capture test ROM, ARM7 side (docs/hw/sound-capture.md). No libnds, no crt0. Loaded and entered at
@ 0x02380000 (docs/hw/cart-header.md).
@
@ Channel 0 loops the constant PCM8 byte 40h (4000h) on the left, channel 2 loops 70h (7000h) on the left
@ (silent until a phase turns it on), and channel 1 plays the capture buffer back on the right at volume 64
@ (half), four words long (a channel needs PNT + LEN of at least 4 words to sound). Capture 0 shares
@ channel 1's timer (TMR FE00h, one sample per output sample). Every sound constant is a byte the ROM
@ writes itself. The ROM steps through eight phases with a busy wait of about 200 output samples between
@ them, stopping and restarting the capture each time:
@   A  nothing captured yet: the buffer is zero.     out (4000h, 0)
@   B  capture of the mixer, PCM16, loop.             out (4000h, 2000h)
@   C  channel 2 on: the mixer sum 0B000h clips to 7FFFh in the capture. out (7FC0h, 3FC0h)
@   D  capture of channel 0 alone (bit 1).            out (7FC0h, 2000h)
@   E  channel 0 + channel 1 (bits 1 and 0): channel 1 leaves the right output and joins channel 0.
@                                                      out (7FC0h, 0)
@   F  capture of the mixer as PCM8 (channel 1 plays PCM8 too).   out (7FC0h, 3F80h)
@   G  channel 2 off, a one-shot PCM8 capture of the mixer.        out (4000h, 2000h)
@   H  channel 0 off: the one-shot ended and holds its bytes.      out (0, 2000h)
    .equ SND0, 0x04000400
    .equ SOUNDCNT, 0x04000500
    .equ SOUNDBIAS, 0x04000504
    .equ CAP0, 0x04000508
    .equ DATA0, 0x02008000
    .equ DATA2, 0x02008020
    .equ BUF, 0x02008100
    .equ TMR, 0xFE00
    .equ WAIT, 100000

    @ Busy wait of WAIT iterations of two instructions each (4 master cycles an iteration).
    .macro DELAY
    ldr r9, =WAIT
1:
    subs r9, r9, #1
    bne 1b
    .endm

    @ Channel \x: SAD = \sad, TMR, PNT 0, LEN \len words.
    .macro SETUP x, sad, len
    ldr r0, =(SND0 + 0x10 * \x)
    ldr r1, =\sad
    str r1, [r0, #4]
    ldr r1, =TMR
    strh r1, [r0, #8]
    mov r1, #0
    strh r1, [r0, #0xA]
    mov r1, #\len
    str r1, [r0, #0xC]
    .endm

    @ Stop capture 0, then start it with SNDCAP0CNT = \cnt over \len words at BUF.
    @ r10 holds CAP0, r11 holds BUF.
    .macro CAPTURE cnt, len
    mov r1, #0
    strb r1, [r10]
    str r11, [r10, #8]
    mov r1, #\len
    strh r1, [r10, #0xC]
    mov r1, #\cnt
    strb r1, [r10]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r0, =SOUNDCNT
    ldr r1, =0x807F
    strh r1, [r0]
    ldr r0, =SOUNDBIAS
    ldr r1, =0x200
    strh r1, [r0]
    ldr r10, =CAP0
    ldr r11, =BUF

    @ The data: 16 bytes of 40h at DATA0, 16 bytes of 70h at DATA2, four zero words at BUF.
    ldr r4, =DATA0
    ldr r5, =0x40404040
    ldr r6, =0x70707070
    mov r7, #0
    ldr r8, =DATA2
    mov r3, #4
fill:
    str r5, [r4], #4
    str r6, [r8], #4
    str r7, [r11, r3, lsl #2]
    subs r3, r3, #1
    bne fill
    str r7, [r11]

    SETUP 0, DATA0, 4
    SETUP 1, BUF, 4
    SETUP 2, DATA2, 4
    @ Start the three: channel 0 loop PCM8 volume 127 left; channel 1 loop PCM16 volume 64 right;
    @ channel 2 loop PCM8 volume 0.
    ldr r0, =SND0
    ldr r1, =0x8800007F
    str r1, [r0]
    ldr r1, =0xA87F0040
    str r1, [r0, #0x10]
    ldr r1, =0x88000000
    str r1, [r0, #0x20]

    DELAY                       @ A
    CAPTURE 0x80, 4             @ B
    DELAY
    ldr r1, =0x8800007F         @ C: channel 2 on
    str r1, [r0, #0x20]
    DELAY
    CAPTURE 0x82, 4             @ D
    DELAY
    CAPTURE 0x83, 4             @ E
    DELAY
    mov r1, #0                  @ F: channel 1 plays four words of PCM8
    str r1, [r0, #0x10]
    mov r1, #4
    str r1, [r0, #0x1C]
    ldr r1, =0x887F0040
    str r1, [r0, #0x10]
    CAPTURE 0x88, 4
    DELAY
    ldr r1, =0x88000000         @ G: channel 2 off, one-shot PCM8 capture
    str r1, [r0, #0x20]
    CAPTURE 0x8C, 4
    DELAY
    ldr r1, =0x88000000         @ H: channel 0 off
    str r1, [r0]
    DELAY
spin:
    b spin
    .ltorg
