@ SPDX-License-Identifier: Apache-2.0
@ RTC ROM, ARM9 side (docs/hw/rtc.md). No libnds, no crt0. Loaded at 0x02000000 and entered there
@ (docs/hw/cart-header.md).
@
@ The ARM7 drives the clock chip through RTCCNT and records what it read (arm7.s). This side waits
@ for the ARM7's sentinel, compares the record with the values the note gives and shows six checks
@ as 32-row bands on engine A's screen (the bottom one, display swap clear): green passes, red fails.
@   0  Status register 1 reads 02h: 24-hour mode, no flags.
@   1  The chip starts at the host's time: 2000-01-01 (a Saturday, day of week 5) 00:00:00.
@   2  A written date and time reads back, with the hour byte's PM flag (bit 6) set from noon on in
@      24-hour mode: 23h reads 63h.
@   3  In 12-hour mode 23:59:58 reads as 11 PM: 51h 59h 58h.
@   4  Status register 2 and the free register read back what was written (55h, A5h).
@   5  INT2 reads back its day byte with only bits 0-2 and 7 kept: FFh -> 87h, then 12h 34h.
@
@ Record at 0x02008000 (slots 0-7 and the sentinel are described in arm7.s; the expected values are
@ in crates/runner/tests/rtc.rs):
@   8..13  the band colours (halfwords 03E0h green, 001Fh red)
@ Slot 30: progress (1 when done). Slot 31: the ARM7's sentinel.
    .equ REC, 0x02008000
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000

    @ Sets the Z flag from the comparison and writes band \idx green (Z) or red.
    .macro PAINT idx
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #((8 + \idx) * 4)]
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
    @ Wait for the ARM7's sentinel "RTCM".
    ldr r2, =0x4D435452
wait_arm7:
    ldr r0, [r8, #(31 * 4)]
    cmp r0, r2
    bne wait_arm7

    ldr r5, =0x03E0
    ldr r6, =0x001F

    @ 0: status register 1.
    ldr r0, [r8, #0]
    cmp r0, #0x02
    PAINT 0
    @ 1: the chip's starting time, 00 01 01 05 and 00 00 00.
    ldr r0, [r8, #4]
    ldr r2, =0x05010100
    cmp r0, r2
    ldreq r0, [r8, #8]
    cmpeq r0, #0
    PAINT 1
    @ 2: 24 12 31 01 and 63 59 58.
    ldr r0, [r8, #12]
    ldr r2, =0x01311224
    cmp r0, r2
    ldreq r0, [r8, #16]
    ldreq r2, =0x00585963
    cmpeq r0, r2
    PAINT 2
    @ 3: 51 59 58.
    ldr r0, [r8, #20]
    ldr r2, =0x00585951
    cmp r0, r2
    PAINT 3
    @ 4: status 2 = 55h, free = A5h.
    ldr r0, [r8, #24]
    ldr r2, =0x0000A555
    cmp r0, r2
    PAINT 4
    @ 5: INT2 = 87 12 34.
    ldr r0, [r8, #28]
    ldr r2, =0x00341287
    cmp r0, r2
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
    add r1, r8, #(8 * 4)
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
