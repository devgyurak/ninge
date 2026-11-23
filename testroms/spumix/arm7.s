@ SPDX-License-Identifier: Apache-2.0
@ Mixer test ROM, ARM7 side (docs/hw/spu.md, "Volume and panning", "The mixer", "The gates"). No libnds,
@ no crt0. Loaded and entered at 0x02380000 (docs/hw/cart-header.md).
@
@ Four channels each loop 16 bytes of one constant PCM8 value, so the output is a steady level
@ that the registers decide, and the ROM steps through nine settings with a busy wait between them
@ (about 200 output samples each):
@   channel 0: byte 40h (4000h), panning 0 (left).     channel 1: byte 20h (2000h), panning 127.
@   channel 2: byte 10h (1000h), panning 64 (centre).  channel 3: byte 08h (0800h), panning 0.
@ Every channel has volume 127 when it is on; a channel that is off has volume 0 but still plays.
@
@   A  channel 0 only, master 127, bias 200h.        B  channel 1 on.        C  channel 2 on.
@   D  master volume 64.                              E  channel 3 on.
@   F  master 127 and channel 1 not sent to the mixer (SOUNDCNT bit 12).
@   G  as F, and the left output taken from channel 3 alone (SOUNDCNT bits 8-9 = 2).
@   H  all four in the mixer again and SOUNDBIAS 3FFh: both sides clip.
@   I  master enable off and SOUNDBIAS 240h: the output is the bias alone.
    .equ SND0, 0x04000400
    .equ SOUNDCNT, 0x04000500
    .equ SOUNDBIAS, 0x04000504
    .equ DATA, 0x02008000
    .equ TMR, 0xFE00
    .equ WAIT, 100000

    @ Busy wait of WAIT iterations of two instructions each (4 master cycles an iteration).
    .macro DELAY
    ldr r9, =WAIT
1:
    subs r9, r9, #1
    bne 1b
    .endm

    @ Channel \x: SAD = DATA + 16 * x, TMR, PNT 0, LEN 4 words (16 bytes).
    .macro SETUP x
    ldr r0, =(SND0 + 0x10 * \x)
    ldr r1, =(DATA + 16 * \x)
    str r1, [r0, #4]
    ldr r1, =TMR
    strh r1, [r0, #8]
    mov r1, #0
    strh r1, [r0, #0xA]
    mov r1, #4
    str r1, [r0, #0xC]
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

    @ The four constant blocks: 40h, 20h, 10h, 08h, sixteen bytes each.
    ldr r4, =DATA
    ldr r5, =0x40404040
    ldr r6, =0x20202020
    ldr r7, =0x10101010
    ldr r8, =0x08080808
    mov r3, #4
fill:
    str r5, [r4]
    str r6, [r4, #16]
    str r7, [r4, #32]
    str r8, [r4, #48]
    add r4, r4, #4
    subs r3, r3, #1
    bne fill

    SETUP 0
    SETUP 1
    SETUP 2
    SETUP 3
    @ Start all four (loop 1 << 27, PCM8): channel 0 on, the others at volume 0.
    ldr r0, =SND0
    ldr r1, =0x8800007F
    str r1, [r0]
    ldr r1, =0x887F0000
    str r1, [r0, #0x10]
    ldr r1, =0x88400000
    str r1, [r0, #0x20]
    ldr r1, =0x88000000
    str r1, [r0, #0x30]

    DELAY                       @ A
    ldr r1, =0x887F007F         @ B: channel 1 on
    str r1, [r0, #0x10]
    DELAY
    ldr r1, =0x8840007F         @ C: channel 2 on
    str r1, [r0, #0x20]
    DELAY
    ldr r2, =SOUNDCNT           @ D: master volume 64
    ldr r1, =0x8040
    strh r1, [r2]
    DELAY
    ldr r1, =0x8800007F         @ E: channel 3 on
    str r1, [r0, #0x30]
    DELAY
    ldr r1, =0x907F             @ F: master 127, channel 1 not sent to the mixer
    strh r1, [r2]
    DELAY
    ldr r1, =0x927F             @ G: and the left output from channel 3
    strh r1, [r2]
    DELAY
    ldr r1, =0x807F             @ H: all in the mixer, bias 3FFh
    strh r1, [r2]
    ldr r3, =SOUNDBIAS
    ldr r1, =0x3FF
    strh r1, [r3]
    DELAY
    mov r1, #0                  @ I: master enable off, bias 240h
    strh r1, [r2]
    ldr r1, =0x240
    strh r1, [r3]
spin:
    b spin
    .ltorg
