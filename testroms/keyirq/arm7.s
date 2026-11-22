@ SPDX-License-Identifier: Apache-2.0
@ Keypad-and-hinge-interrupt ROM, ARM7 side (docs/hw/keypad.md, "The keypad IRQ" and "The hinge
@ IRQ"). No libnds, no crt0. Loaded and entered at 0x02380000 (docs/hw/cart-header.md).
@
@ The ARM7 takes no interrupt: IE and IME stay clear and so does nothing else; it only reads IF.
@ Its KEYCNT is its own (keypad.md): AND mode over A and B, IRQ enabled (C003h). The runner holds
@ the buttons and the hinge as crates/runner/tests/keyirq.rs lists, one input per frame, and this
@ side waits for each state to show in KEYINPUT or EXTKEYIN before it reads IF; a state shows only
@ after the frame's latch, which is where the request is made, so the read is not a race.
@
@ Records at 0x02008000, one word per slot (the ARM9 judges them, arm9.s):
@   8   IF bit 12 with A alone held (AND needs both)
@   9   IF bit 12 with A and B held
@       (the ROM then acknowledges bit 12)
@   10  IF bit 12 with B alone held
@   11  IF bit 22 with the hinge closed
@   12  IF bit 22 once the hinge is open again
@   13  IE (never written, so 0 by construction: it shows the bit is set with IE clear)
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02008000
    .equ IO, 0x04000000
    .equ KEYINPUT, 0x04000130
    .equ KEYCNT, 0x04000132
    .equ EXTKEYIN, 0x04000136
    .equ IF, 0x04000214
    .equ IE, 0x04000210

    @ Waits until (KEYINPUT & \mask) == \want.
    .macro WAIT_KEYS mask, want
1:  ldrh r0, [r9]
    and r0, r0, #\mask
    cmp r0, #\want
    bne 1b
    .endm

    @ Records IF & \bit in slot \slot.
    .macro REC_IF bit, slot
    ldr r0, [r10]
    ldr r1, =\bit
    and r0, r0, r1
    str r0, [r8, #(\slot * 4)]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r9, =KEYINPUT
    ldr r10, =IF
    ldr r11, =EXTKEYIN
    ldr r0, =0xC003
    strh r0, [r9, #2]          @ KEYCNT: AND, IRQ enable, A and B selected

    @ TODO(hw-unverified): the sampling points and the request on every latch are the model's choice
    @ (keypad.md, "Unverified"). Slot 10 cannot tell a re-request after the acknowledge from the AND
    @ condition being false: the machine tests of the core pin that rule.
    WAIT_KEYS 3, 2             @ A alone: bit 0 clear, bit 1 set
    REC_IF 0x1000, 8
    WAIT_KEYS 3, 0             @ A and B
    REC_IF 0x1000, 9
    ldr r0, =0x1000
    str r0, [r10]              @ acknowledge the keypad request
    WAIT_KEYS 3, 1             @ B alone
    REC_IF 0x1000, 10

    @ The hinge: EXTKEYIN bit 7 is 1 while it is closed.
wait_closed:
    ldrh r0, [r11]
    tst r0, #0x80
    beq wait_closed
    @ TODO(hw-unverified): that closing requests nothing is the model's choice (keypad.md, "Unverified").
    REC_IF 0x400000, 11
wait_open:
    ldrh r0, [r11]
    tst r0, #0x80
    bne wait_open
    REC_IF 0x400000, 12
    ldr r0, =IE
    ldr r0, [r0]
    str r0, [r8, #(13 * 4)]

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4B495237        @ "KIR7"
    str r0, [r8, #(31 * 4)]
spin:
    b spin
    .ltorg
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
