@ SPDX-License-Identifier: Apache-2.0
@ Touch ROM, ARM9 side (docs/hw/touch-power.md). No libnds, no crt0. Loaded at 0x02000000 and
@ entered there (docs/hw/cart-header.md).
@
@ The ARM7 reads the touch-screen controller and the power device and records what it read
@ (arm7.s). This side waits for the ARM7's sentinel, compares the record with the values the note
@ gives and shows six checks as 32-row bands on engine A's screen (the bottom one, display swap
@ clear): green passes, red fails. The panel is pressed at pixel (100, 50), so X = 100 * 16 = 640h
@ and Y = 50 * 16 = 320h.
@   0  Pen bit pressed at power-on (EXTKEYIN 3Fh), X 640h, Y 320h.
@   1  Z1 600h, Z2 A00h.
@   2  X in 8-bit mode 640h; X and Y with the next command in the low-byte slot, 03200640h.
@   3  Temperatures 600h and 68Eh (068E0600h); microphone 800h, battery 000h.
@   4  EXTKEYIN 7Fh in power-down mode 1, 3Fh in mode 0, 7Fh once the panel is released.
@   5  Released: X 000h, Y FFFh; power register 0 reads 0Ch, the battery register 0, the register
@      written BFh reads 3Fh, and index 4 mirrors register 0 (3Fh).
@
@ Record at 0x02008000 (slots 0-17 and the sentinel are described in arm7.s; the expected values
@ are in crates/runner/tests/touch.rs):
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
    @ Wait for the ARM7's sentinel "TCHM".
    ldr r2, =0x4D484354
wait_arm7:
    ldr r0, [r8, #(31 * 4)]
    cmp r0, r2
    bne wait_arm7

    ldr r5, =0x03E0
    ldr r6, =0x001F

    FIRST 0, 0x3F
    NEXT 1, 0x640
    NEXT 2, 0x320
    PAINT 0
    FIRST 3, 0x600
    NEXT 4, 0xA00
    PAINT 1
    FIRST 5, 0x640
    NEXT 6, 0x03200640
    PAINT 2
    FIRST 7, 0x068E0600
    NEXT 8, 0x800
    PAINT 3
    FIRST 9, 0x7F
    NEXT 10, 0x3F
    NEXT 11, 0x7F
    PAINT 4
    FIRST 12, 0
    NEXT 13, 0xFFF
    NEXT 14, 0x0C
    NEXT 15, 0
    NEXT 16, 0x3F
    NEXT 17, 0x3F
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
