@ SPDX-License-Identifier: Apache-2.0
@ Keypad ROM, ARM7 side: KEYINPUT, KEYCNT and EXTKEYIN of the ARM7. No libnds, no handshake with
@ the ARM9. Loaded and entered at 0x02380000 (docs/hw/cart-header.md). The ROM records observed
@ values only; the expected ones live in crates/runner/tests/keypad.rs (docs/hw/keypad.md).
@
@ ARM7 record at 0x02005000, one word per slot:
@   0  halfword read of KEYINPUT    1  halfword read of EXTKEYIN
@   2  word read at 0x04000130 (KEYINPUT low half, KEYCNT high half)
@   3  KEYCNT read back after a halfword write of FFFFh
@   4  word read at 0x04000134 (RCNT low half, EXTKEYIN high half)
@   5  EXTKEYIN read back after a halfword write of 0
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02005000

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r9, =0x04000130        @ KEYINPUT; halfword offsets reach 255 only
    ldrh r0, [r9, #0]
    str r0, [r8, #0]
    ldrh r0, [r9, #6]
    str r0, [r8, #4]
    ldr r0, [r9, #0]
    str r0, [r8, #8]
    ldr r1, =0xFFFF
    strh r1, [r9, #2]
    ldrh r0, [r9, #2]
    str r0, [r8, #12]
    ldr r0, [r9, #4]
    str r0, [r8, #16]
    mov r1, #0
    strh r1, [r9, #6]
    ldrh r0, [r9, #6]
    str r0, [r8, #20]
    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4B455937        @ "KEY7"
    str r0, [r8, #(31 * 4)]
spin:
    b spin
    .ltorg
