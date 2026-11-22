@ SPDX-License-Identifier: Apache-2.0
@ Sound ROM, ARM7 side (docs/hw/sound-registers.md). No libnds, no crt0. Loaded and entered at
@ 0x02380000 (docs/hw/cart-header.md).
@
@ The ARM7 writes the sound registers, polls the busy bits and times them with its timer 0 (prescaler
@ F/1: one count per ARM7 cycle, which is two master cycles). It records for the ARM9 to judge (arm9.s):
@   0  SOUNDCNT after a halfword write of FFFFh.            1  SOUNDBIAS after a word write of all ones.
@   2  SOUND2CNT after a word write of all ones (PSG, so it starts and stays busy).
@   3  SOUND2CNT after a word write of 0 (stopped).
@   4  SNDCAP0CNT after a byte write of FFh (a loop capture, busy).
@   5  The write-only registers of channel 4 (SAD, TMR, PNT, LEN), all written with all ones, ORed
@      together as read back.
@   6  Timer counts until a one-shot PCM8 channel of 4 words at TMR FC00h stops being busy. A sample
@      takes 4 * 400h master cycles, 16 samples last 65,536 master cycles = 32,768 ARM7 cycles.
@   7  Bit 31 of a loop channel (same length) read after the one-shot has ended (still busy).
@   8  SOUNDBIAS after SoundBias with r0 = 1.               9  The same with r0 = 0.
@  10  Timer counts until a one-shot PCM16 capture of LEN 0 (taken as 1 word, 2 samples at channel 1's
@      TMR FC00h) stops being busy: 8,192 master cycles = 4,096 ARM7 cycles.
@  11  POWCNT2 at power-on.   12  POWCNT2 after a halfword write of FFFFh.
@  13  Bit 7 of a loop capture 1 (channel 3's TMR FC00h) read after capture 0's one-shot ended (busy).
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02008000
    .equ SND0, 0x04000400
    .equ SND2, 0x04000420
    .equ SND4, 0x04000440
    .equ SOUNDCNT, 0x04000500
    .equ SOUNDBIAS, 0x04000504
    .equ SNDCAP0CNT, 0x04000508
    .equ SNDCAP0LEN, 0x04000514
    .equ SND3, 0x04000430
    .equ SNDCAP1CNT, 0x04000509
    .equ SNDCAP1LEN, 0x0400051C
    .equ POWCNT2, 0x04000304
    .equ TM0CNT_L, 0x04000100
    .equ START, 0x80000000
    @ Start, one-shot (repeat 2), PCM8; and start, loop (repeat 1), PCM8.
    .equ ONE_SHOT_PCM8, 0x90000000
    .equ LOOP_PCM8, 0x88000000
    .equ TMR, 0xFC00

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r9, =TM0CNT_L

    @ ---- 11, 12: POWCNT2.
    ldr r0, =POWCNT2
    ldrh r1, [r0]
    str r1, [r8, #(11 * 4)]
    ldr r1, =0xFFFF
    strh r1, [r0]
    ldrh r1, [r0]
    str r1, [r8, #(12 * 4)]

    @ ---- 0, 1: SOUNDCNT and SOUNDBIAS read-back masks.
    ldr r0, =SOUNDCNT
    ldr r1, =0xFFFF
    strh r1, [r0]
    ldrh r2, [r0]
    str r2, [r8, #(0 * 4)]
    mvn r1, #0
    ldr r0, =SOUNDBIAS
    str r1, [r0]
    ldr r2, [r0]
    str r2, [r8, #(1 * 4)]

    @ ---- 2, 3: all ones in a channel's control register, then zero.
    ldr r0, =SND2
    str r1, [r0]
    ldr r2, [r0]
    str r2, [r8, #(2 * 4)]
    mov r2, #0
    str r2, [r0]
    ldr r2, [r0]
    str r2, [r8, #(3 * 4)]

    @ ---- 4: SNDCAP0CNT, byte write of FFh. Bit 2 is set, so this is a one-shot PCM8 capture; it still
    @ reads busy because channel 1's TMR is 0 here, which makes a sample very long. Then stop it.
    ldr r0, =SNDCAP0CNT
    mov r2, #0xFF
    strb r2, [r0]
    ldrb r2, [r0]
    str r2, [r8, #(4 * 4)]
    mov r2, #0
    strb r2, [r0]

    @ ---- 5: the write-only registers read zero.
    ldr r0, =SND4
    str r1, [r0, #4]
    str r1, [r0, #8]
    str r1, [r0, #0xC]
    ldr r2, [r0, #4]
    ldr r3, [r0, #8]
    orr r2, r2, r3
    ldr r3, [r0, #0xC]
    orr r2, r2, r3
    str r2, [r8, #(5 * 4)]

    @ ---- 6, 7: a loop channel (1) and a one-shot channel (0), the same length and rate.
    ldr r1, =TMR
    mov r2, #4
    ldr r0, =(SND0 + 0x10)
    strh r1, [r0, #8]
    str r2, [r0, #0xC]
    ldr r3, =LOOP_PCM8
    str r3, [r0]
    ldr r0, =SND0
    strh r1, [r0, #8]
    str r2, [r0, #0xC]
    mov r4, #0
    strh r4, [r9]               @ reload value 0
    mov r4, #0x80
    strh r4, [r9, #2]           @ timer 0 on: counts from 0
    ldr r3, =ONE_SHOT_PCM8
    str r3, [r0]
poll_channel:
    ldr r1, [r0]
    tst r1, #START
    bne poll_channel
    ldrh r2, [r9]
    str r2, [r8, #(6 * 4)]
    ldr r0, =(SND0 + 0x10)
    ldr r1, [r0]
    mov r1, r1, lsr #31
    str r1, [r8, #(7 * 4)]
    mov r4, #0
    strh r4, [r9, #2]           @ timer off
    str r4, [r0]                @ stop the loop channel

    @ ---- 10, 13: a one-shot PCM16 capture of length 0 at channel 1's rate, timed; beside it a loop
    @ capture 1 at channel 3's rate, which is still busy when the one-shot has ended.
    ldr r0, =SNDCAP0LEN
    str r4, [r0]
    ldr r1, =TMR
    ldr r0, =SND3
    strh r1, [r0, #8]
    ldr r0, =SNDCAP1LEN
    str r4, [r0]
    ldr r0, =SNDCAP1CNT
    mov r3, #0x80               @ start, loop
    strb r3, [r0]
    ldr r0, =SNDCAP0CNT
    ldr r0, =SNDCAP0CNT
    strh r4, [r9]
    mov r1, #0x80
    strh r1, [r9, #2]
    mov r3, #0x84               @ start, one-shot, PCM16
    strb r3, [r0]
poll_capture:
    ldrb r1, [r0]
    tst r1, #0x80
    bne poll_capture
    ldrh r2, [r9]
    str r2, [r8, #(10 * 4)]
    mov r4, #0
    strh r4, [r9, #2]
    ldr r0, =SNDCAP1CNT
    ldrb r1, [r0]
    mov r1, r1, lsr #7
    str r1, [r8, #(13 * 4)]
    strb r4, [r0]               @ stop the loop capture

    @ ---- 8, 9: SoundBias with r0 = 1 and r0 = 0.
    mov r0, #1
    swi 0x080000
    ldr r0, =SOUNDBIAS
    ldr r1, [r0]
    str r1, [r8, #(8 * 4)]
    mov r0, #0
    swi 0x080000
    ldr r0, =SOUNDBIAS
    ldr r1, [r0]
    str r1, [r8, #(9 * 4)]

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4D534E44         @ "SNDM"
    str r0, [r8, #(31 * 4)]
spin:
    b spin
    .ltorg
