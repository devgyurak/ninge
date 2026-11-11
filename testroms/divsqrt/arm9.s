@ SPDX-License-Identifier: Apache-2.0
@ Divider and square-root ROM, ARM9 side: runs each mode of DIVCNT and SQRTCNT over known operands
@ and records what it reads. No libnds, no handshake with the ARM7 (it only spins). Loaded and
@ entered at 0x02000000 (docs/hw/cart-header.md). The ROM records observed values only; the
@ expected ones live in crates/runner/tests/divsqrt.rs, each worked out from docs/hw/divsqrt.md.
@
@ Division op k fills six words at slot 8k of the record at 0x02004000:
@   +0 DIVCNT bit 15 right after the last operand write (the busy bit)
@   +1 DIVCNT after the busy bit cleared     +2 DIV_RESULT low   +3 DIV_RESULT high
@   +4 DIVREM_RESULT low   +5 DIVREM_RESULT high
@ Ops: 0 mode 0 100/7   1 mode 0 -7/2   2 mode 0 5/0   3 mode 0 80000000h/FFFFFFFFh
@   4 mode 1 2^32/3   5 mode 2 2^40/(2^32+1)   6 mode 2 -5/0   7 mode 2 -2^63/-1
@   8 mode 3 2^32/3   9 mode 0 -7/2 with garbage in the upper halves
@   10 mode 1 -2^32/-3
@ Square-root op k fills three words at slot 96 + 4k:
@   +0 SQRTCNT bit 15 right after the last parameter write   +1 SQRTCNT after the busy bit cleared
@   +2 SQRT_RESULT
@ Ops: 0 mode 0 1000000   1 mode 1 2^32   2 mode 1 FFFFFFFFFFFFFFFFh   3 mode 0 low 100, high
@   FFFFFFFFh   4 mode 0 99
@ Slot 118: DIV_RESULT low after a write of 1234h to it   119: DIVCNT bit 15 right after that write
@ Slot 120: progress (1 when done). Slot 121: sentinel, written last.
    .equ REC, 0x02004000

    @ One division: DIVCNT, then numerator and denominator as low and high words, the denominator
    @ high word last.
    .macro DIVOP op, mode, nlo, nhi, dlo, dhi
    ldr r0, =\mode
    str r0, [r9, #0x00]
    ldr r0, =\nlo
    str r0, [r9, #0x10]
    ldr r0, =\nhi
    str r0, [r9, #0x14]
    ldr r0, =\dlo
    str r0, [r9, #0x18]
    ldr r0, =\dhi
    str r0, [r9, #0x1C]
    ldr r0, [r9, #0x00]
    and r0, r0, #0x8000
    str r0, [r8, #(\op * 32)]
1:  ldr r0, [r9, #0x00]
    tst r0, #0x8000
    bne 1b
    str r0, [r8, #(\op * 32 + 4)]
    ldr r0, [r9, #0x20]
    str r0, [r8, #(\op * 32 + 8)]
    ldr r0, [r9, #0x24]
    str r0, [r8, #(\op * 32 + 12)]
    ldr r0, [r9, #0x28]
    str r0, [r8, #(\op * 32 + 16)]
    ldr r0, [r9, #0x2C]
    str r0, [r8, #(\op * 32 + 20)]
    .endm

    @ One square root: SQRTCNT, then the parameter's low and high words, the high word last.
    .macro SQRTOP op, mode, plo, phi
    ldr r0, =\mode
    str r0, [r9, #0x30]
    ldr r0, =\plo
    str r0, [r9, #0x38]
    ldr r0, =\phi
    str r0, [r9, #0x3C]
    ldr r0, [r9, #0x30]
    and r0, r0, #0x8000
    str r0, [r8, #(384 + \op * 16)]
2:  ldr r0, [r9, #0x30]
    tst r0, #0x8000
    bne 2b
    str r0, [r8, #(384 + \op * 16 + 4)]
    ldr r0, [r9, #0x34]
    str r0, [r8, #(384 + \op * 16 + 8)]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r9, =0x04000280         @ DIVCNT; the units sit at +0x00 to +0x3F

    DIVOP 0, 0, 100, 0, 7, 0
    DIVOP 1, 0, 0xFFFFFFF9, 0, 2, 0
    DIVOP 2, 0, 5, 0, 0, 0
    DIVOP 3, 0, 0x80000000, 0, 0xFFFFFFFF, 0
    DIVOP 4, 1, 0, 1, 3, 0
    DIVOP 5, 2, 0, 0x100, 1, 1
    DIVOP 6, 2, 0xFFFFFFFB, 0xFFFFFFFF, 0, 0
    DIVOP 7, 2, 0, 0x80000000, 0xFFFFFFFF, 0xFFFFFFFF
    DIVOP 8, 3, 0, 1, 3, 0
    DIVOP 9, 0, 0xFFFFFFF9, 0x12345678, 2, 7
    DIVOP 10, 1, 0, 0xFFFFFFFF, 0xFFFFFFFD, 0

    SQRTOP 0, 0, 1000000, 0
    SQRTOP 1, 1, 0, 1
    SQRTOP 2, 1, 0xFFFFFFFF, 0xFFFFFFFF
    SQRTOP 3, 0, 100, 0xFFFFFFFF
    SQRTOP 4, 0, 99, 0

    @ A write to a result register changes nothing and starts nothing.
    ldr r0, =0x1234
    str r0, [r9, #0x20]
    ldr r0, [r9, #0x00]
    and r0, r0, #0x8000
    str r0, [r8, #(119 * 4)]
    ldr r0, [r9, #0x20]
    str r0, [r8, #(118 * 4)]

    mov r0, #1
    str r0, [r8, #(120 * 4)]
    ldr r0, =0x44495637         @ "DIV7"
    str r0, [r8, #(121 * 4)]
spin:
    b spin
    .ltorg
