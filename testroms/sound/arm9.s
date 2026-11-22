@ SPDX-License-Identifier: Apache-2.0
@ Sound ROM, ARM9 side (docs/hw/sound-registers.md). No libnds, no crt0. Loaded at 0x02000000 and
@ entered there (docs/hw/cart-header.md).
@
@ The ARM7 writes the sound registers and records what it read (arm7.s). This side waits for the
@ ARM7's sentinel, judges the record against the values the note gives and shows six checks as
@ 32-row bands on engine A's screen (the bottom one, display swap clear): green passes, red fails.
@   0  SOUNDCNT BF7Fh, SOUNDBIAS 3FFh, a PSG channel FF7F837Fh then 0 after a stop.
@   1  SNDCAP0CNT 8Fh; the write-only registers read 0.
@   2  The one-shot channel stops being busy after 32,768 ARM7 cycles: the count lies in
@      [32700, 33100) (the ARM7 starts the timer a few instructions early and polls a few late).
@   3  The loop channel is still busy (1) after the one-shot ended.
@   4  The capture of LEN 0 stops after 4,096 ARM7 cycles: [4000, 4300).
@   5  SoundBias: 200h for r0 = 1, 000h for r0 = 0; POWCNT2 1 at power-on and 3 after all ones.
@
@ Record at 0x02008000 (slots 0-12 and the sentinel are described in arm7.s; the expected values
@ are in crates/runner/tests/sound.rs):
@   20..25  the band colours (halfwords 03E0h green, 001Fh red)
@ Slot 30: progress (1 when done). Slot 31: the ARM7's sentinel.
    .equ REC, 0x02008000
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000

    @ The first comparison of a band: slot \slot against \val.
    .macro FIRST slot, val
    ldr r0, [r8, #(\slot * 4)]
    ldr r2, =\val
    cmp r0, r2
    .endm

    @ A further comparison, made only while the band still passes.
    .macro NEXT slot, val
    ldreq r0, [r8, #(\slot * 4)]
    ldreq r2, =\val
    cmpeq r0, r2
    .endm


    @ A first comparison of a band: slot \slot minus \base is below \lim (unsigned), so Z is set when
    @ the count lies in [base, base + lim).
    .macro WINDOW slot, base, lim
    ldr r0, [r8, #(\slot * 4)]
    ldr r2, =\base
    sub r0, r0, r2
    cmp r0, #\lim
    movlo r0, #0
    movhs r0, #1
    cmp r0, #0
    .endm

    @ Writes band \idx green (Z set) or red.
    .macro PAINT idx
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #((20 + \idx) * 4)]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ Protection unit and caches off, so every access reaches the bus.
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop

    ldr r8, =REC
    @ Wait for the ARM7's sentinel "SNDM".
    ldr r2, =0x4D534E44
wait_arm7:
    ldr r0, [r8, #(31 * 4)]
    cmp r0, r2
    bne wait_arm7

    ldr r5, =0x03E0
    ldr r6, =0x001F

    FIRST 0, 0xBF7F
    NEXT 1, 0x3FF
    NEXT 2, 0xFF7F837F
    NEXT 3, 0
    PAINT 0
    FIRST 4, 0x8F
    NEXT 5, 0
    PAINT 1
    WINDOW 6, 32700, 400
    PAINT 2
    FIRST 7, 1
    PAINT 3
    WINDOW 10, 4000, 300
    PAINT 4
    FIRST 8, 0x200
    NEXT 9, 0
    NEXT 11, 1
    NEXT 12, 3
    PAINT 5

    @ ---- Show the bands: engine A, display mode 2 (VRAM display), bank A as LCDC.
    ldr r0, =POWCNT1
    mov r1, #3
    strh r1, [r0]
    ldr r0, =VRAMCNT_A
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =DISPCNT
    mov r1, #0x00020000
    str r1, [r0]
    ldr r9, =VRAM_A
    mov r4, #0                 @ y
band_row:
    mov r1, #32
    mov r2, #0                 @ band = y / 32 by repeated subtraction
    mov r0, r4
band_div:
    cmp r0, r1
    blo band_done
    sub r0, r0, r1
    add r2, r2, #1
    b band_div
band_done:
    add r1, r8, #(20 * 4)
    add r1, r1, r2, lsl #2
    ldrh r6, [r1]
    mov r5, #0                 @ x
band_pixel:
    strh r6, [r9], #2
    add r5, r5, #1
    cmp r5, #256
    blo band_pixel
    add r4, r4, #1
    cmp r4, #192
    blo band_row

    mov r0, #1
    str r0, [r8, #(30 * 4)]
spin:
    b spin
    .ltorg
