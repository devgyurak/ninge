@ SPDX-License-Identifier: Apache-2.0
@ PCM test ROM, ARM7 side (docs/hw/spu.md, "Position"). No libnds, no crt0. Loaded and entered at
@ 0x02380000 (docs/hw/cart-header.md). All data is made by this code, never read from anything.
@
@ Three channels at TMR FE00h, so a channel sample takes 4 * 200h = 2,048 master cycles, one output
@ sample:
@   0  PCM8 loop of 16 words (64 bytes), a rising ramp: byte i = 4 * i - 128. Volume 64, panning 0.
@   1  PCM16 loop of 8 words (16 samples), a rising ramp: sample j = 800h * j - 4000h. Volume 127,
@      panning 127 (all right).
@   2  PCM8 one-shot of 4 words (the first 16 bytes of the ramp) with Hold: it ends after 16 samples
@      and keeps its last sample, byte 15 = -68. Volume 64, panning 64.
@ Master volume 127 with the master enable, bias 200h.
    .equ SND0, 0x04000400
    .equ SOUNDCNT, 0x04000500
    .equ SOUNDBIAS, 0x04000504
    .equ DATA8, 0x02008000
    .equ DATA16, 0x02008100
    .equ TMR, 0xFE00

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

    @ PCM8 ramp: byte i = 4 * (i - 32), i = 0..63.
    ldr r4, =DATA8
    mov r5, #0
ramp8:
    sub r6, r5, #32
    mov r6, r6, lsl #2
    strb r6, [r4, r5]
    add r5, r5, #1
    cmp r5, #64
    bne ramp8

    @ PCM16 ramp: sample j = 800h * j - 4000h, j = 0..15.
    ldr r4, =DATA16
    mov r5, #0
ramp16:
    mov r6, r5, lsl #11
    sub r6, r6, #0x4000
    mov r7, r5, lsl #1
    strh r6, [r4, r7]
    add r5, r5, #1
    cmp r5, #16
    bne ramp16

    @ Channel 0: PCM8 loop, 16 words.
    ldr r0, =SND0
    ldr r1, =DATA8
    str r1, [r0, #4]
    ldr r1, =TMR
    strh r1, [r0, #8]
    mov r1, #0
    strh r1, [r0, #0xA]
    mov r1, #16
    str r1, [r0, #0xC]
    @ Channel 1: PCM16 loop, 8 words.
    ldr r2, =(SND0 + 0x10)
    ldr r1, =DATA16
    str r1, [r2, #4]
    ldr r1, =TMR
    strh r1, [r2, #8]
    mov r1, #0
    strh r1, [r2, #0xA]
    mov r1, #8
    str r1, [r2, #0xC]
    @ Channel 2: PCM8 one-shot, 4 words, Hold.
    ldr r3, =(SND0 + 0x20)
    ldr r1, =DATA8
    str r1, [r3, #4]
    ldr r1, =TMR
    strh r1, [r3, #8]
    mov r1, #0
    strh r1, [r3, #0xA]
    mov r1, #4
    str r1, [r3, #0xC]

    @ Start all three: start bit 31, the repeat mode (loop 1 << 27, one-shot 2 << 27), the format
    @ (PCM8 0, PCM16 1 << 29), Hold 1 << 15, the panning in bits 16-22 and the volume in bits 0-6.
    ldr r1, =0x88000040         @ loop, PCM8, volume 64, panning 0
    str r1, [r0]
    ldr r1, =0xA87F007F         @ loop, PCM16, volume 127, panning 127
    str r1, [r2]
    ldr r1, =0x90408040         @ one-shot, PCM8, Hold, volume 64, panning 64
    str r1, [r3]
spin:
    b spin
    .ltorg
