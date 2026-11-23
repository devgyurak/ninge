@ SPDX-License-Identifier: Apache-2.0
@ Touch ROM, ARM7 side (docs/hw/touch-power.md). No libnds, no crt0. Loaded and entered at
@ 0x02380000 (docs/hw/cart-header.md).
@
@ The ARM7 drives the touch-screen controller (SPI device select 2) and the power-management device
@ (select 0) through SPICNT and SPIDATA, one byte at a time, holding the chip selected until the
@ last byte of a conversion. The runner presses the panel at pixel (100, 50) for the first frames
@ and then releases it. A 12-bit result is rebuilt from the two reply bytes as
@ ((hi << 8) | lo) >> 3 (the dummy bit sits on top; the top bit of hi is zero, so no mask is needed).
@
@ It records, for the ARM9 to judge (see arm9.s), while the panel is pressed:
@   0  EXTKEYIN at power-on (the pen output is enabled in mode 0).
@   1  X, 12-bit.      2  Y, 12-bit.      3  Z1.      4  Z2.
@   5  X, 8-bit, rebuilt with the same formula.
@   6  X in the low half and Y in the high half, read with the next command in the low-byte slot.
@   7  Temperature 0 in the low half, temperature 1 in the high half.
@   8  The microphone channel in the low half, the battery channel in the high half.
@   9  EXTKEYIN after a command in power-down mode 1.
@  10  EXTKEYIN after a command in power-down mode 0 again.
@ then the power device, then, once EXTKEYIN shows the release (mode 0 enabled the pen output):
@  11  EXTKEYIN released.   12  X released.   13  Y released.
@  14  Power register 0, written 0Ch and read back.
@  15  The battery register (1).
@  16  Power register 0, written BFh and read back.
@  17  Index 4, which mirrors register 0 on this console.
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02008000
    .equ SPICNT, 0x040001C0
    .equ EXTKEYIN, 0x04000136
    @ SPICNT: bus on, baud 1 (2 MHz, 8-bit), touch selected, with and without hold.
    .equ TSC_HOLD, 0x8A01
    .equ TSC_LAST, 0x8201
    @ SPICNT: bus on, baud 2 (1 MHz), power selected, with and without hold.
    .equ PM_HOLD, 0x8802
    .equ PM_LAST, 0x8002

    @ One byte of a touch session, held or last.
    .macro XH byte
    mov r0, #\byte
    ldr r1, =TSC_HOLD
    bl xfer
    .endm
    .macro XL byte
    mov r0, #\byte
    ldr r1, =TSC_LAST
    bl xfer
    .endm

    @ A whole 12-bit conversion: the control byte and two reads; the result in r0.
    .macro TSC cmd
    mov r0, #\cmd
    bl tsc3
    .endm

    @ The result from the high byte in r4 and the low byte in r0.
    .macro COMB
    orr r0, r0, r4, lsl #8
    mov r0, r0, lsr #3
    .endm

    @ Power register write and read.
    .macro PMW reg, val
    mov r0, #\reg
    ldr r1, =PM_HOLD
    bl xfer
    mov r0, #\val
    ldr r1, =PM_LAST
    bl xfer
    .endm
    .macro PMR reg
    mov r0, #(\reg | 0x80)
    ldr r1, =PM_HOLD
    bl xfer
    mov r0, #0
    ldr r1, =PM_LAST
    bl xfer
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC

    bl pen
    str r0, [r8, #(0 * 4)]
    TSC 0xD0                    @ X, 12-bit, differential, mode 0
    str r0, [r8, #(1 * 4)]
    TSC 0x90                    @ Y
    str r0, [r8, #(2 * 4)]
    TSC 0xB4                    @ Z1, single-ended
    str r0, [r8, #(3 * 4)]
    TSC 0xC4                    @ Z2
    str r0, [r8, #(4 * 4)]
    TSC 0xD8                    @ X, 8-bit
    str r0, [r8, #(5 * 4)]

    @ ---- 6: the next command in the low-byte slot: X, zero, Y (receives X's low byte), zero, zero.
    XH 0xD0
    XH 0
    mov r4, r0
    XH 0x90
    COMB
    mov r6, r0
    XH 0
    mov r4, r0
    XL 0
    COMB
    orr r0, r6, r0, lsl #16
    str r0, [r8, #(6 * 4)]

    @ ---- 7: temperature 0 and 1 (single-ended). 8: the microphone and the battery.
    TSC 0x84
    mov r5, r0
    TSC 0xF4
    orr r0, r5, r0, lsl #16
    str r0, [r8, #(7 * 4)]
    TSC 0xE4
    mov r5, r0
    TSC 0xA4
    orr r0, r5, r0, lsl #16
    str r0, [r8, #(8 * 4)]

    @ ---- 9, 10: the pen output follows the power-down mode of the last command.
    TSC 0xD1
    bl pen
    str r0, [r8, #(9 * 4)]
    TSC 0xD0
    bl pen
    str r0, [r8, #(10 * 4)]

    @ ---- 14-17: the power device.
    PMW 0, 0x0C
    PMR 0
    str r0, [r8, #(14 * 4)]
    PMR 1
    str r0, [r8, #(15 * 4)]
    PMW 0, 0xBF
    PMR 0
    str r0, [r8, #(16 * 4)]
    PMR 4
    str r0, [r8, #(17 * 4)]

    @ ---- 11-13: wait for the release (the pen bit, EXTKEYIN bit 6, reads 1), then read again.
wait_release:
    bl pen
    tst r0, #0x40
    beq wait_release
    str r0, [r8, #(11 * 4)]
    TSC 0xD0
    str r0, [r8, #(12 * 4)]
    TSC 0x90
    str r0, [r8, #(13 * 4)]

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4D484354         @ "TCHM"
    str r0, [r8, #(31 * 4)]
spin:
    b spin

@ EXTKEYIN into r0.
pen:
    ldr r2, =EXTKEYIN
    ldrh r0, [r2]
    bx lr

@ Exchanges the byte in r0 with SPICNT set to r1 and waits for the unit; the reply in r0.
xfer:
    ldr r2, =SPICNT
    strh r1, [r2]
    strh r0, [r2, #2]
xf_wait:
    ldrh r3, [r2]
    tst r3, #0x80
    bne xf_wait
    ldrh r0, [r2, #2]
    and r0, r0, #0xFF
    bx lr

@ A conversion with the control byte in r0: the byte, then two reads; the 12-bit result in r0.
@ Keeps the return address in r10 (there is no stack).
tsc3:
    mov r10, lr
    ldr r1, =TSC_HOLD
    bl xfer
    mov r0, #0
    ldr r1, =TSC_HOLD
    bl xfer
    mov r4, r0
    mov r0, #0
    ldr r1, =TSC_LAST
    bl xfer
    COMB
    bx r10
    .ltorg
