@ SPDX-License-Identifier: Apache-2.0
@ IMA-ADPCM test ROM, ARM7 side (docs/hw/spu.md, "IMA-ADPCM"). No libnds, no crt0. Loaded and
@ entered at 0x02380000 (docs/hw/cart-header.md). The data below is our own: a header word and seven
@ words of 4-bit values chosen to move the decoder up and down through the step table.
@
@ Two channels at TMR FE00h (one channel sample per output sample), both reading the same 8 words
@ at 02008000h:
@   0  loop, PNT 1, LEN 7: the loop starts at the first data sample, so after 56 samples the decoder
@      state saved there (the header's) is put back and every pass repeats. Volume 127, panning 0.
@   1  one-shot, PNT 0, LEN 8: it plays the 56 samples once and ends (no Hold). Volume 127,
@      panning 127, divider 2 (a quarter).
@ Master volume 127 with the master enable, bias 200h.
    .equ SND0, 0x04000400
    .equ SOUNDCNT, 0x04000500
    .equ SOUNDBIAS, 0x04000504
    .equ DATA, 0x02008000
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

    @ The header (value 0, table index 20) and the seven data words, copied from the table below.
    ldr r4, =DATA
    adr r5, words
    mov r6, #8
copy:
    ldr r7, [r5], #4
    str r7, [r4], #4
    subs r6, r6, #1
    bne copy

    @ Channel 0: loop, ADPCM, PNT 1, LEN 7.
    ldr r0, =SND0
    ldr r1, =DATA
    str r1, [r0, #4]
    ldr r1, =TMR
    strh r1, [r0, #8]
    mov r1, #1
    strh r1, [r0, #0xA]
    mov r1, #7
    str r1, [r0, #0xC]
    @ Channel 1: one-shot, ADPCM, PNT 0, LEN 8.
    ldr r2, =(SND0 + 0x10)
    ldr r1, =DATA
    str r1, [r2, #4]
    ldr r1, =TMR
    strh r1, [r2, #8]
    mov r1, #0
    strh r1, [r2, #0xA]
    mov r1, #8
    str r1, [r2, #0xC]

    @ Start: bit 31, loop (1 << 27) or one-shot (2 << 27), ADPCM (2 << 29), the divider (bits 8-9),
    @ the panning (bits 16-22) and the volume (bits 0-6).
    ldr r1, =0xC800007F         @ loop, ADPCM, volume 127, panning 0
    str r1, [r0]
    ldr r1, =0xD07F027F         @ one-shot, ADPCM, volume 127, panning 127, divider 2
    str r1, [r2]
spin:
    b spin

    .balign 4
words:
    .word 0x00140000            @ header: value 0, index 20
    .word 0x75317531
    .word 0xFDB9FDB9
    .word 0x33333333
    .word 0xBBBBBBBB
    .word 0x55555555
    .word 0xDDDDDDDD
    .word 0x12345678
    .ltorg
