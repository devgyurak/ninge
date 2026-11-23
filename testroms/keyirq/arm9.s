@ SPDX-License-Identifier: Apache-2.0
@ Keypad-and-hinge-interrupt ROM, ARM9 side (docs/hw/keypad.md, "The keypad IRQ"). No libnds, no
@ crt0. Loaded and entered at 0x02000000 (docs/hw/cart-header.md).
@
@ The ARM9's KEYCNT is its own (keypad.md): OR mode over A and B, IRQ enabled (4003h), with IE bit
@ 12 and IME on, so each request is taken at once and the handler (copied to the IRQ vector in the
@ ITCM, as testroms/timers does) counts it at 0x02006000. The runner holds the buttons as
@ crates/runner/tests/keyirq.rs lists, one input per frame. This side waits for each state to show
@ in KEYINPUT and records the count then; the requests are made at the frame's latch, so the
@ handler has run before the state can be read. Records at 0x02008000:
@   0  count with A held     1  with A and B held     2  with nothing held
@   3  with B alone held     4  with START (not selected) held
@ The ARM7 records slots 8-13 (arm7.s) and the sentinel; this side then shows six 32-row bands on
@ engine A's screen (the bottom one): green passes, red fails.
@   0  slots 0 and 1: one request per frame while a selected key is held
@   1  slots 2 and 3: none while released, one for B alone (OR)
@   2  slot 4: none for a key KEYCNT does not select
@   3  ARM7 slots 8 and 9: AND needs both keys
@   4  ARM7 slot 10: B alone is not enough for AND after an acknowledge
@   5  ARM7 slots 11-13: the hinge opening requests bit 22 with IE clear, closing does not
@ Slot 30: progress (1 when done). Slot 31: the ARM7's sentinel.
    .equ REC, 0x02008000
    .equ COUNT, 0x02006000
    .equ KEYINPUT, 0x04000130
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000

    @ Waits until (KEYINPUT & \mask) == \want, then records the request count in slot \slot.
    .macro SEE mask, want, slot
1:  ldrh r0, [r9]
    ldr r1, =\mask
    and r0, r0, r1
    ldr r1, =\want
    cmp r0, r1
    bne 1b
    ldr r0, [r7]
    str r0, [r8, #(\slot * 4)]
    .endm

    .macro FIRST slot, val
    ldr r0, [r8, #(\slot * 4)]
    ldr r2, =\val
    cmp r0, r2
    .endm

    .macro NEXT slot, val
    ldreq r0, [r8, #(\slot * 4)]
    ldreq r2, =\val
    cmpeq r0, r2
    .endm

    .macro PAINT idx
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #((20 + \idx) * 4)]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r7, =COUNT
    ldr r9, =KEYINPUT
    @ Protection unit, caches and high vectors off, so the IRQ vector is at 0x18.
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00002000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    @ The ITCM is off after direct boot (direct-boot.md): 32 KiB, enabled, handler copied in.
    mov r0, #0x0C
    mcr p15, 0, r0, c9, c1, 1
    mrc p15, 0, r0, c1, c0, 0
    orr r0, r0, #0x00040000
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    ldr r0, =handler_start
    ldr r2, =handler_end
    mov r1, #0x18
copy_handler:
    ldr r3, [r0], #4
    str r3, [r1], #4
    cmp r0, r2
    bne copy_handler
    mov r1, #0
    str r1, [r7]

    mov r12, #0x04000000
    mov r0, #0x1000
    str r0, [r12, #0x210]      @ IE = keypad
    mov r0, #1
    str r0, [r12, #0x208]      @ IME
    ldr r0, =0x4003
    strh r0, [r9, #2]          @ KEYCNT: OR, IRQ enable, A and B selected
    mov r0, #0x1F
    msr cpsr_c, r0             @ I clear, System mode

    @ TODO(hw-unverified): one request per frame while a key is held, and the sampling points, are the
    @ model's choice (keypad.md, "Unverified").
    SEE 1, 0, 0                @ A held
    SEE 3, 0, 1                @ A and B held
    SEE 0x3FF, 0x3FF, 2        @ nothing held
    SEE 3, 1, 3                @ B alone
    SEE 8, 0, 4                @ START held

    @ Wait for the ARM7's sentinel "KIR7".
    ldr r2, =0x4B495237
wait_arm7:
    ldr r0, [r8, #(31 * 4)]
    cmp r0, r2
    bne wait_arm7

    ldr r5, =0x03E0
    ldr r6, =0x001F
    FIRST 0, 1
    NEXT 1, 2
    PAINT 0
    FIRST 2, 2
    NEXT 3, 3
    PAINT 1
    FIRST 4, 3
    PAINT 2
    FIRST 8, 0
    NEXT 9, 0x1000
    PAINT 3
    FIRST 10, 0
    PAINT 4
    FIRST 11, 0
    NEXT 12, 0x400000
    NEXT 13, 0
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

@ The IRQ handler, copied to the vector at ITCM 0x18. It uses r10-r12, which the main code leaves
@ alone (IRQ mode banks only r13 and r14): it acknowledges IF and counts the request.
    .align 2
handler_start:
    mov r12, #0x04000000
    ldr r11, [r12, #0x214]
    str r11, [r12, #0x214]
    mov r12, #0x02000000
    add r12, r12, #0x6000
    ldr r10, [r12]
    add r10, r10, #1
    str r10, [r12]
    subs pc, lr, #4
handler_end:
    .ltorg
