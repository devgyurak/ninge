@ SPDX-License-Identifier: Apache-2.0
@ Keypad ROM, ARM9 side: reads KEYINPUT and KEYCNT of the ARM9's own copy. No libnds, no
@ handshake with the ARM7 (its KEYCNT is separate, docs/hw/keypad.md). Loaded and entered at
@ 0x02000000 (docs/hw/cart-header.md). The ROM records observed values only; the expected ones
@ live in crates/runner/tests/keypad.rs, each citing docs/hw/keypad.md.
@
@ ARM9 record at 0x02004000, one word per slot:
@   0  word read at 0x04000130 (KEYINPUT low half, KEYCNT high half)
@   1  halfword read of KEYINPUT
@   2  KEYCNT read back after a halfword write of FFFFh
@   3  KEYINPUT read back after a halfword write of 0
@   4  word read at 0x04000134 (RCNT and EXTKEYIN: nothing there on the ARM9)
@   5  halfword read at 0x04000136 after a halfword write of FFFFh there
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02004000

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r9, =0x04000130        @ KEYINPUT; halfword offsets reach 255 only
    ldr r0, [r9, #0]
    str r0, [r8, #0]
    ldrh r0, [r9, #0]
    str r0, [r8, #4]
    ldr r1, =0xFFFF
    strh r1, [r9, #2]
    ldrh r0, [r9, #2]
    str r0, [r8, #8]
    mov r1, #0
    strh r1, [r9, #0]
    ldrh r0, [r9, #0]
    str r0, [r8, #12]
    ldr r0, [r9, #4]
    str r0, [r8, #16]
    ldr r1, =0xFFFF
    strh r1, [r9, #6]
    ldrh r0, [r9, #6]
    str r0, [r8, #20]
    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4B455939        @ "KEY9"
    str r0, [r8, #(31 * 4)]
spin:
    b spin
    .ltorg
