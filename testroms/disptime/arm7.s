@ SPDX-License-Identifier: Apache-2.0
@ Display-timing ROM, ARM7 side: records VCOUNT over a frame and DISPSTAT and IF at chosen
@ lines, from ARM7's own view. The two CPUs run the same sequence independently, each polling
@ VCOUNT, so there is no handshake. No libnds, no crt0. Loaded and entered at 0x02380000
@ (docs/hw/cart-header.md). The ROM records observed values only; the expected ones live in
@ crates/runner/tests/display_timing.rs, each citing docs/hw/display-timing.md.
@
@ Record at 0x02005000, one word per slot:
@   0  DISPSTAT before anything wrote it
@   1  DISPSTAT just after VCOUNT reads 100      2  ... 192      3  ... 261
@   4  DISPSTAT just after VCOUNT reads 262      5  ... 0 (the wrap)
@   6  DISPSTAT when the HBlank flag is first seen in line 50
@   7  VCOUNT at that moment                     8  VCOUNT when the flag clears again
@   9  DISPSTAT with the setting 100, just after VCOUNT reads 99    10  ... 100    11  ... 101
@   12 DISPSTAT after writing FFFF at line 5
@   13 IF at line 125 (IME = 0, IE = 7, DISPSTAT: setting 120, all enables, IF cleared before)
@   14 IF after writing 2 to it
@   15 IF at line 195 after clearing IF at line 191
@   16 IF after writing 1 to it
@   30 progress (1 when done)  31 sentinel, written last
@ VCOUNT sequence at 0x02012000: 264 halfwords, each new value polled over one frame.
    .equ REC, 0x02005000
    .equ SEQ, 0x02012000

    .macro at_line line, slot
    ldr r0, =\line
    bl wait_line
    ldrh r1, [r9, #4]
    str r1, [r8, #(\slot * 4)]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: DISPSTAT +4, VCOUNT +6, IME +0x208, IE +0x210, IF +0x214

    @ ---- 0: DISPSTAT as it reads before anything wrote it
    ldrh r0, [r9, #4]
    str r0, [r8, #0]

    @ ---- VCOUNT sequence: every new value over one frame, wrap included (264 halfwords)
    ldr r6, =SEQ
    ldrh r4, [r9, #6]
    mov r5, #0
seq_loop:
    ldrh r0, [r9, #6]
    cmp r0, r4
    beq seq_loop
    mov r4, r0
    strh r0, [r6, r5]
    add r5, r5, #2
    cmp r5, #528
    bne seq_loop

    @ ---- 12: a write of FFFF keeps only the enables and the setting (line 5: no flag is set,
    @ and the setting 511 matches no line)
    mov r0, #5
    bl wait_line
    ldr r0, =0xFFFF
    strh r0, [r9, #4]
    ldrh r0, [r9, #4]
    str r0, [r8, #48]
    mov r0, #0
    strh r0, [r9, #4]

    @ ---- 6-8: HBlank flag sets in line 50 and clears at line 51
    mov r0, #50
    bl wait_line
wait_hblank:
    ldrh r0, [r9, #4]
    tst r0, #2
    beq wait_hblank
    str r0, [r8, #24]
    ldrh r1, [r9, #6]
    str r1, [r8, #28]
wait_hblank_end:
    ldrh r0, [r9, #4]
    tst r0, #2
    bne wait_hblank_end
    ldrh r1, [r9, #6]
    str r1, [r8, #32]

    @ ---- 9-11: VCount-match flag with the setting 100
    mov r0, #0x6400
    strh r0, [r9, #4]
    at_line 99, 9
    at_line 100, 10
    at_line 101, 11
    mov r0, #0
    strh r0, [r9, #4]

    @ ---- 13-14: IF latches HBlank and VCount-match with IME = 0, and clears by writing 1
    ldr r0, =0x7838            @ setting 120, all three IRQ enables
    strh r0, [r9, #4]
    mov r0, #7
    str r0, [r9, #0x210]       @ IE
    mov r0, #0
    str r0, [r9, #0x208]       @ IME
    mov r0, #7
    str r0, [r9, #0x214]       @ clear IF
    mov r0, #125
    bl wait_line
    ldr r0, [r9, #0x214]
    str r0, [r8, #52]
    mov r0, #2
    str r0, [r9, #0x214]       @ clear the HBlank bit only
    ldr r0, [r9, #0x214]
    str r0, [r8, #56]

    @ ---- 15-16: IF latches VBlank (line 192) and clears by writing 1
    mov r0, #191
    bl wait_line
    mov r0, #7
    str r0, [r9, #0x214]
    mov r0, #195
    bl wait_line
    ldr r0, [r9, #0x214]
    str r0, [r8, #60]
    mov r0, #1
    str r0, [r9, #0x214]
    ldr r0, [r9, #0x214]
    str r0, [r8, #64]
    mov r0, #0
    strh r0, [r9, #4]
    str r0, [r9, #0x210]
    mov r0, #7
    str r0, [r9, #0x214]

    @ ---- 1-5: the VBlank flag around lines 100, 192, 261, 262 and the wrap to 0
    at_line 100, 1
    at_line 192, 2
    at_line 261, 3
    at_line 262, 4
    at_line 0, 5

    mov r0, #1
    str r0, [r8, #120]         @ slot 30
    ldr r0, =0x44495337
    str r0, [r8, #124]         @ slot 31
spin:
    b spin

@ Polls VCOUNT until it reads r0.
wait_line:
    ldrh r1, [r9, #6]
    cmp r1, r0
    bne wait_line
    mov pc, lr
    .ltorg
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; padding fixed it.
    .space 28, 0
