@ SPDX-License-Identifier: Apache-2.0
@ Poweroff ROM, ARM7 side: writes the power-management device's register 0 with the shutdown bit
@ set (bit 6) and both backlights off, then spins (docs/hw/touch-power.md, "What the power
@ register does"). A host that runs it sees the machine report the shutdown after the first frame.
    .equ SPICNT, 0x040001C0
    @ SPICNT: bus on, baud 2 (1 MHz), power-management device selected, with and without hold.
    .equ PM_HOLD, 0x8802
    .equ PM_LAST, 0x8002
    @ Register 0: shutdown (bit 6), backlights off (bits 2 and 3 clear).
    .equ SHUTDOWN, 0x40

    .section .text, "ax"
    .arm
    .global _start
_start:
    mov r0, #0                @ index byte: write register 0
    ldr r1, =PM_HOLD
    bl xfer
    mov r0, #SHUTDOWN
    ldr r1, =PM_LAST
    bl xfer
spin:
    b spin

@ Exchanges the byte in r0 with SPICNT set to r1 and waits for the unit.
xfer:
    ldr r2, =SPICNT
    strh r1, [r2]
    strh r0, [r2, #2]
xf_wait:
    ldrh r3, [r2]
    tst r3, #0x80
    bne xf_wait
    bx lr
    .ltorg
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
