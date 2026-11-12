@ SPDX-License-Identifier: Apache-2.0
@ VRAM-banks ROM, ARM9 side: the nine VRAM banks A-I in LCDC mode (docs/hw/vram-banks.md).
@ No libnds, no crt0. Loaded at 0x02000000, entered at 0x02000100. The ROM records observed
@ values only; the expected values live in crates/runner/tests/vrambanks.rs, computed from the
@ note and not from this file.
@
@ Banks are visited through the `banks` table at the end: VRAMCNT address, LCDC start, size.
@ Bank number i is 0 (A) to 8 (I). Every enable is VRAMCNT = 0x80 (enabled, MST 0).
@
@ Record at 0x02004000, one word per slot (offset = slot * 4); the sentinel is written last.
@
@   slot     check
@   0-8      all nine banks on: marker word 0x56000000 | i stored at the first word of bank i's
@            window, then read back there (slot i)
@   9-17     marker word 0xE0000000 | i stored at the last word of the window, read back
@            (slot 9 + i)
@   18-26    bank i on alone (all others off): the word just below its window (slot 18 + i)
@   27-35    ... the word just above its window (slot 27 + i)
@   36-44    all nine on: STRB 0xEE over the first word of bank i, then LDR of the word
@            (slot 36 + i)
@   45       all banks off: LDR at the first word of bank A
@   46       bank A on again: the same LDR (the contents survived)
@   47       LDR of 0x04000240 with bank A on (VRAMCNT_A..D are write-only)
@   48       progress: the number of the last finished group (1 markers, 2 neighbours,
@            3 byte stores, 4 disable and re-enable)
@   49       sentinel, written last
@
@ The protection unit and both caches are switched off at the start, as in the memmap ROM, so
@ the record reads back through the bus. Code runs from main RAM only.
    .equ REC, 0x02004000
    .equ BANKS, 9

    .macro rec slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    .section .head, "ax"
    .arm
    .space 0x100, 0

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    adr r9, banks
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop

    @ ---- group 1: markers at the first and last word of every window ----
    mov r4, #0
g1w:
    add r5, r4, r4, lsl #1
    add r5, r9, r5, lsl #2
    ldmia r5, {r0, r1, r2}      @ VRAMCNT address, window start, size
    mov r3, #0x80
    strb r3, [r0]               @ bank i on, MST 0
    ldr r6, =0x56000000
    orr r6, r6, r4
    str r6, [r1]
    ldr r7, =0xE0000000
    orr r7, r7, r4
    add r10, r1, r2
    str r7, [r10, #-4]
    add r4, r4, #1
    cmp r4, #BANKS
    blo g1w
    mov r4, #0
g1r:
    add r5, r4, r4, lsl #1
    add r5, r9, r5, lsl #2
    ldmia r5, {r0, r1, r2}
    ldr r6, [r1]
    str r6, [r8, r4, lsl #2]    @ slot i
    add r10, r1, r2
    ldr r7, [r10, #-4]
    add r11, r4, #9
    str r7, [r8, r11, lsl #2]   @ slot 9 + i
    add r4, r4, #1
    cmp r4, #BANKS
    blo g1r
    mov r1, #1
    rec 48, r1

    @ ---- group 2: bank i alone, the words just outside its window ----
    mov r4, #0
g2:
    bl all_off
    add r5, r4, r4, lsl #1
    add r5, r9, r5, lsl #2
    ldmia r5, {r0, r1, r2}
    mov r3, #0x80
    strb r3, [r0]
    ldr r6, [r1, #-4]
    add r11, r4, #18
    str r6, [r8, r11, lsl #2]   @ slot 18 + i
    ldr r7, [r1, r2]
    add r11, r4, #27
    str r7, [r8, r11, lsl #2]   @ slot 27 + i
    add r4, r4, #1
    cmp r4, #BANKS
    blo g2
    mov r1, #2
    rec 48, r1

    @ ---- group 3: a byte store changes nothing, in every bank ----
    bl all_on
    mov r4, #0
g3:
    add r5, r4, r4, lsl #1
    add r5, r9, r5, lsl #2
    ldmia r5, {r0, r1, r2}
    mov r3, #0xEE
    strb r3, [r1]
    ldr r6, [r1]
    add r11, r4, #36
    str r6, [r8, r11, lsl #2]   @ slot 36 + i
    add r4, r4, #1
    cmp r4, #BANKS
    blo g3
    mov r1, #3
    rec 48, r1

    @ ---- group 4: off, then on again; VRAMCNT reads ----
    bl all_off
    ldr r1, =0x06800000
    ldr r2, [r1]
    rec 45, r2
    ldr r0, =0x04000240
    mov r3, #0x80
    strb r3, [r0]
    ldr r2, [r1]
    rec 46, r2
    ldr r2, [r0]
    rec 47, r2
    mov r1, #4
    rec 48, r1
    ldr r1, =0x56425421
    rec 49, r1
spin:
    b spin

    @ Every bank off (VRAMCNT = 0). Clobbers r0, r3, r5, r6, r7.
all_off:
    mov r3, #0
    b set_all
    @ Every bank on, MST 0.
all_on:
    mov r3, #0x80
set_all:
    mov r6, #0
set_all_next:
    add r5, r6, r6, lsl #1
    add r5, r9, r5, lsl #2
    ldr r0, [r5]
    strb r3, [r0]
    add r6, r6, #1
    cmp r6, #BANKS
    blo set_all_next
    bx lr

    .ltorg
    .align 2
banks:
    .word 0x04000240, 0x06800000, 0x20000      @ A
    .word 0x04000241, 0x06820000, 0x20000      @ B
    .word 0x04000242, 0x06840000, 0x20000      @ C
    .word 0x04000243, 0x06860000, 0x20000      @ D
    .word 0x04000244, 0x06880000, 0x10000      @ E
    .word 0x04000245, 0x06890000, 0x04000      @ F
    .word 0x04000246, 0x06894000, 0x04000      @ G
    .word 0x04000248, 0x06898000, 0x08000      @ H
    .word 0x04000249, 0x068A0000, 0x04000      @ I
