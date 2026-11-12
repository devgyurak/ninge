@ SPDX-License-Identifier: Apache-2.0
@ VRAM-banks ROM, ARM9 side: the nine VRAM banks A-I: LCDC mode, the BG and OBJ windows of both
@ engines, overlap, and the modes the CPU cannot see (docs/hw/vram-banks.md).
@ No libnds, no crt0. Loaded at 0x02000000, entered at 0x02000100. The ROM records observed
@ values only; the expected values live in crates/runner/tests/vrambanks.rs, computed from the
@ note and not from this file. The ARM7 side (arm7.s) records at 0x02005000; its steps are
@ described there, and group 10 below sets up each one.
@
@ Banks are visited through the `banks` table at the end: VRAMCNT address, LCDC start, size.
@ Bank number i is 0 (A) to 8 (I). Groups 1-4 enable with VRAMCNT = 0x80 (enabled, MST 0);
@ the later groups write the value their table row names. Each bank's first word holds
@ 0x56000000 | i and its last word 0xE0000000 | i (group 1), so a read that finds a marker
@ names the bank it came from, and every window is read through those bytes.
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
@            3 byte stores, 4 disable and re-enable, 5 windows, 6 overlap, 7 invisible,
@            8 byte stores in windows, 9 moves, 10 the ARM7's steps)
@   49       sentinel, written last
@   500-523  group 10, step s (1-8) of `arm7_steps`: 3 words at 500 + 3 (s - 1): the ARM9's own
@            reads at 0x06000000, 0x06840000 (bank C's LCDC address) and 0x06860000 (bank D's)
@            before it posts step s, so what the ARM9 sees while C and D are the ARM7's
@   530      after the last step, bank C back in LCDC mode: the word at 0x06840010 the ARM7
@            stored (STR then STRB)
@   531      ... the word at 0x06840014 (the ARM7's STRH)
@   100-334  group 5, entry e of the `entries` table (bank, VRAMCNT value, CPU address of the
@            window start), only that bank on: slot 100 + 5e is the address used, +1 the word
@            there, +2 the last word of the window, +3 the word just below, +4 just above
@   340-353  group 6, case c of `cases` (three (bank, value) pairs, 0xFF = none, then an
@            address): the word read at the address with those banks on (all else off)
@   360-371  group 7, case c of `empties` (bank, value; the CPU cannot see the bank): the OR of
@            the words at the bank's LCDC start and at 0x06000000, 0x06200000, 0x06400000,
@            0x06600000
@   380-383  group 8, case c of `bytes` (bank, value, address of a window start): STRB 0xEE
@            there, then the LDR of the word
@   390-393  group 9, bank A: a word stored through the BG window at 0x06000100, read in the
@            LCDC window (390), the OBJ window at 0x06400100 (391), BG at OFS 1 and 0x06020100
@            (392), then as a texture slot and back in LCDC (393)
@
@ The protection unit and both caches are switched off at the start, as in the memmap ROM, so
@ the record reads back through the bus. Code runs from main RAM only.
    .equ REC, 0x02004000
    .equ BANKS, 9
    .equ ENTRIES, 47
    .equ CASES, 14
    .equ EMPTIES, 12
    .equ BYTES, 4

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
    ldr r9, =banks
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

    @ ---- group 5: every MST and OFS that puts a bank in a window ----
    mov r4, #0
g5:
    bl all_off
    ldr r10, =entries
    add r12, r4, r4, lsl #1
    add r12, r10, r12, lsl #2
    ldmia r12, {r1, r2, r3}     @ bank, VRAMCNT value, CPU address
    add r5, r1, r1, lsl #1
    add r5, r9, r5, lsl #2
    ldmia r5, {r0, r6, r7}      @ VRAMCNT address, (LCDC start), size
    strb r2, [r0]
    add r11, r4, r4, lsl #2
    add r11, r11, #100
    add r11, r8, r11, lsl #2    @ this entry's five slots
    str r3, [r11]
    ldr r0, [r3]
    str r0, [r11, #4]
    add r6, r3, r7
    ldr r0, [r6, #-4]
    str r0, [r11, #8]
    ldr r0, [r3, #-4]
    str r0, [r11, #12]
    ldr r0, [r6]
    str r0, [r11, #16]
    add r4, r4, #1
    cmp r4, #ENTRIES
    blo g5
    mov r1, #5
    rec 48, r1

    @ ---- group 6: overlapping banks ----
    mov r4, #0
g6:
    bl all_off
    ldr r10, =cases
    rsb r1, r4, r4, lsl #3      @ case * 7 words
    add r1, r10, r1, lsl #2
    mov r5, #0
g6p:
    add r2, r1, r5, lsl #3
    ldr r6, [r2]
    ldr r7, [r2, #4]
    cmp r6, #0xFF
    beq g6s
    add r0, r6, r6, lsl #1
    add r0, r9, r0, lsl #2
    ldr r0, [r0]
    strb r7, [r0]
g6s:
    add r5, r5, #1
    cmp r5, #3
    blo g6p
    ldr r3, [r1, #24]
    ldr r0, [r3]
    add r11, r4, #340
    str r0, [r8, r11, lsl #2]
    add r4, r4, #1
    cmp r4, #CASES
    blo g6
    mov r1, #6
    rec 48, r1

    @ ---- group 7: extended palette, texture and undefined modes leave every address empty ----
    mov r4, #0
g7:
    bl all_off
    ldr r10, =empties
    add r1, r10, r4, lsl #3
    ldr r6, [r1]
    ldr r7, [r1, #4]
    add r5, r6, r6, lsl #1
    add r5, r9, r5, lsl #2
    ldmia r5, {r0, r2, r3}      @ VRAMCNT address, LCDC start, size
    strb r7, [r0]
    ldr r12, [r2]
    ldr r0, =0x06000000
    ldr r1, [r0]
    orr r12, r12, r1
    ldr r0, =0x06200000
    ldr r1, [r0]
    orr r12, r12, r1
    ldr r0, =0x06400000
    ldr r1, [r0]
    orr r12, r12, r1
    ldr r0, =0x06600000
    ldr r1, [r0]
    orr r12, r12, r1
    add r11, r4, #360
    str r12, [r8, r11, lsl #2]
    add r4, r4, #1
    cmp r4, #EMPTIES
    blo g7
    mov r1, #7
    rec 48, r1

    @ ---- group 8: a byte store to a window changes nothing ----
    mov r4, #0
g8:
    bl all_off
    ldr r10, =bytes
    add r5, r4, r4, lsl #1
    add r5, r10, r5, lsl #2
    ldmia r5, {r1, r2, r3}      @ bank, VRAMCNT value, window address
    add r0, r1, r1, lsl #1
    add r0, r9, r0, lsl #2
    ldr r0, [r0]
    strb r2, [r0]
    mov r6, #0xEE
    strb r6, [r3]
    ldr r7, [r3]
    add r11, r4, #380
    str r7, [r8, r11, lsl #2]
    add r4, r4, #1
    cmp r4, #BYTES
    blo g8
    mov r1, #8
    rec 48, r1

    @ ---- group 9: a bank keeps its bytes when it moves ----
    bl all_off
    ldr r10, =0x04000240        @ VRAMCNT_A
    mov r1, #0x81
    strb r1, [r10]              @ BG window
    ldr r2, =0x06000100
    ldr r3, =0xC0FFEE01
    str r3, [r2]
    mov r1, #0x80
    strb r1, [r10]              @ LCDC
    ldr r2, =0x06800100
    ldr r0, [r2]
    rec 390, r0
    mov r1, #0x82
    strb r1, [r10]              @ OBJ window
    ldr r2, =0x06400100
    ldr r0, [r2]
    rec 391, r0
    mov r1, #0x89
    strb r1, [r10]              @ BG window, OFS 1
    ldr r2, =0x06020100
    ldr r0, [r2]
    rec 392, r0
    mov r1, #0x83
    strb r1, [r10]             @ texture slot 0: not in the CPU space
    mov r1, #0x80
    strb r1, [r10]              @ back to LCDC
    ldr r2, =0x06800100
    ldr r0, [r2]
    rec 393, r0
    mov r1, #9
    rec 48, r1
    @ ---- group 10: the ARM7's mapping, one IPCSYNC step per setting of C and D ----
    ldr r10, =0x04000180        @ IPCSYNC
    mov r4, #1
g10:
    bl all_off
    ldr r5, =arm7_steps
    sub r1, r4, #1
    add r5, r5, r1, lsl #3
    ldr r2, [r5]                @ VRAMCNT_C
    ldr r3, [r5, #4]            @ VRAMCNT_D
    ldr r0, =0x04000242
    strb r2, [r0]
    ldr r0, =0x04000243
    strb r3, [r0]
    add r11, r1, r1, lsl #1
    add r11, r11, #500
    add r11, r8, r11, lsl #2    @ this step's three slots
    ldr r0, =0x06000000
    ldr r6, [r0]
    str r6, [r11]
    ldr r0, =0x06840000
    ldr r6, [r0]
    str r6, [r11, #4]
    ldr r0, =0x06860000
    ldr r6, [r0]
    str r6, [r11, #8]
    mov r6, r4, lsl #8
    strh r6, [r10]              @ post step s
g10w:
    ldrh r6, [r10]
    and r6, r6, #0xF
    cmp r6, r4
    bne g10w                    @ the ARM7 posts s back when it has recorded
    add r4, r4, #1
    cmp r4, #9
    blo g10
    bl all_off
    ldr r0, =0x04000242
    mov r1, #0x80
    strb r1, [r0]               @ bank C back to LCDC
    ldr r0, =0x06840010
    ldr r1, [r0]
    rec 530, r1
    ldr r0, =0x06840014
    ldr r1, [r0]
    rec 531, r1
    mov r1, #10
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
    .align 2
entries:
    .word 0, 0x81, 0x06000000     @ A BG A OFS 0
    .word 0, 0x89, 0x06020000     @ A BG A OFS 1
    .word 0, 0x91, 0x06040000     @ A BG A OFS 2
    .word 0, 0x99, 0x06060000     @ A BG A OFS 3
    .word 1, 0x81, 0x06000000     @ B BG A OFS 0
    .word 1, 0x89, 0x06020000     @ B BG A OFS 1
    .word 1, 0x91, 0x06040000     @ B BG A OFS 2
    .word 1, 0x99, 0x06060000     @ B BG A OFS 3
    .word 2, 0x81, 0x06000000     @ C BG A OFS 0
    .word 2, 0x89, 0x06020000     @ C BG A OFS 1
    .word 2, 0x91, 0x06040000     @ C BG A OFS 2
    .word 2, 0x99, 0x06060000     @ C BG A OFS 3
    .word 3, 0x81, 0x06000000     @ D BG A OFS 0
    .word 3, 0x89, 0x06020000     @ D BG A OFS 1
    .word 3, 0x91, 0x06040000     @ D BG A OFS 2
    .word 3, 0x99, 0x06060000     @ D BG A OFS 3
    .word 4, 0x81, 0x06000000     @ E BG A
    .word 5, 0x81, 0x06000000     @ F BG A OFS 0
    .word 5, 0x89, 0x06004000     @ F BG A OFS 1
    .word 5, 0x91, 0x06010000     @ F BG A OFS 2
    .word 5, 0x99, 0x06014000     @ F BG A OFS 3
    .word 6, 0x81, 0x06000000     @ G BG A OFS 0
    .word 6, 0x89, 0x06004000     @ G BG A OFS 1
    .word 6, 0x91, 0x06010000     @ G BG A OFS 2
    .word 6, 0x99, 0x06014000     @ G BG A OFS 3
    .word 0, 0x82, 0x06400000     @ A OBJ A OFS 0
    .word 0, 0x8A, 0x06420000     @ A OBJ A OFS 1
    .word 0, 0x92, 0x06400000     @ A OBJ A OFS 2
    .word 0, 0x9A, 0x06420000     @ A OBJ A OFS 3
    .word 1, 0x82, 0x06400000     @ B OBJ A OFS 0
    .word 1, 0x8A, 0x06420000     @ B OBJ A OFS 1
    .word 1, 0x92, 0x06400000     @ B OBJ A OFS 2
    .word 1, 0x9A, 0x06420000     @ B OBJ A OFS 3
    .word 4, 0x82, 0x06400000     @ E OBJ A
    .word 5, 0x82, 0x06400000     @ F OBJ A OFS 0
    .word 5, 0x8A, 0x06404000     @ F OBJ A OFS 1
    .word 5, 0x92, 0x06410000     @ F OBJ A OFS 2
    .word 5, 0x9A, 0x06414000     @ F OBJ A OFS 3
    .word 6, 0x82, 0x06400000     @ G OBJ A OFS 0
    .word 6, 0x8A, 0x06404000     @ G OBJ A OFS 1
    .word 6, 0x92, 0x06410000     @ G OBJ A OFS 2
    .word 6, 0x9A, 0x06414000     @ G OBJ A OFS 3
    .word 2, 0x84, 0x06200000     @ C BG B
    .word 7, 0x81, 0x06200000     @ H BG B
    .word 8, 0x81, 0x06208000     @ I BG B
    .word 3, 0x84, 0x06600000     @ D OBJ B
    .word 8, 0x82, 0x06600000     @ I OBJ B
    .align 2
cases:
    .word 0, 0x81, 1, 0x81, 255, 0x00, 0x06000000     @ case 0
    .word 1, 0x81, 255, 0x00, 255, 0x00, 0x06000000     @ case 1
    .word 4, 0x81, 0, 0x81, 255, 0x00, 0x06000000     @ case 2
    .word 4, 0x81, 0, 0x81, 255, 0x00, 0x0601FFFC     @ case 3
    .word 6, 0x81, 5, 0x81, 255, 0x00, 0x06000000     @ case 4
    .word 5, 0x81, 6, 0x89, 255, 0x00, 0x06000000     @ case 5
    .word 5, 0x81, 6, 0x89, 255, 0x00, 0x06004000     @ case 6
    .word 2, 0x84, 7, 0x81, 8, 0x81, 0x06200000     @ case 7
    .word 2, 0x84, 7, 0x81, 8, 0x81, 0x06208000     @ case 8
    .word 3, 0x84, 8, 0x82, 255, 0x00, 0x06600000     @ case 9
    .word 7, 0x81, 8, 0x81, 255, 0x00, 0x06208000     @ case 10
    .word 7, 0x81, 8, 0x81, 255, 0x00, 0x06200000     @ case 11
    .word 4, 0x81, 0, 0x89, 255, 0x00, 0x06000000     @ case 12
    .word 4, 0x81, 0, 0x89, 255, 0x00, 0x06020000     @ case 13
    .align 2
empties:
    .word 0, 0x83
    .word 1, 0x8B
    .word 2, 0x93
    .word 3, 0x9B
    .word 4, 0x83
    .word 4, 0x84
    .word 5, 0x84
    .word 5, 0x85
    .word 6, 0x83
    .word 7, 0x82
    .word 8, 0x83
    .word 2, 0x85
    .align 2
bytes:
    .word 0, 0x81, 0x06000000
    .word 0, 0x82, 0x06400000
    .word 2, 0x84, 0x06200000
    .word 3, 0x84, 0x06600000
    .align 2
arm7_steps:                     @ VRAMCNT_C, VRAMCNT_D per step
    .word 0x82, 0x00            @ 1: C MST 2, OFS 0
    .word 0x8A, 0x00            @ 2: C MST 2, OFS 1
    .word 0x82, 0x82            @ 3: both, OFS 0
    .word 0x82, 0x8A            @ 4: C OFS 0, D OFS 1
    .word 0x80, 0x81            @ 5: C LCDC, D BG A
    .word 0x02, 0x0A            @ 6: MST 2 but disabled
    .word 0x9A, 0x00            @ 7: C MST 2, OFS 3 (OFS.1 not used)
    .word 0x92, 0x00            @ 8: C MST 2, OFS 2
