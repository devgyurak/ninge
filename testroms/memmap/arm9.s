@ SPDX-License-Identifier: Apache-2.0
@ Memory-map ROM, ARM9 side: exercises main RAM, its mirrors, both TCMs, VRAM bank A and an
@ undefined I/O port, and records what it read. No libnds, no crt0. Loaded at 0x02000000,
@ entered at 0x02000100, like the boot ROM. The ROM records observed values only; the expected
@ values live in crates/runner/tests/memmap.rs, each citing docs/hw/arm9-memory-map.md,
@ vram-banks.md or cp15.md.
@
@ Record at 0x02004000, one word per slot (offset = slot * 4). A group updates the progress
@ slot when it ends, and the sentinel is written last, so a hang shows how far the ROM got.
@
@   slot  check
@   0     main RAM: STR then LDR
@   1     LDRH at +0 of that word
@   2     LDRH at +2
@   3     LDRB at +0
@   4     LDRB at +3
@   5     STRH at +2 of a word of ones, LDR of the word
@   6     LDRH of that halfword
@   7     STRB at +1 of a zero word, LDR of the word
@   8     LDRB of that byte
@   9     unaligned LDR (address & 3 = 1)
@   10    word written at 0x02110000, read at 0x02510000
@   11    ... read at 0x02910000
@   12    ... read at 0x02D10000
@   13    word written at 0x02610000, read at 0x02210000
@   14    word at 0x023FFFFC (last word of the 4 MiB), read there
@   15    ... read at 0x027FFFFC
@   16    marker word at 0x02000000, read at 0x02400000 (first word past the 4 MiB)
@   17    ... 0x023FFFFC read at 0x02FFFFFC (last word of the 16 MiB block)
@   18    DTCM (base 0x02300000, 16 KiB) on: write then read at its base
@   19    DTCM off: the same address, read again
@   20    DTCM region 32 KiB: word written at +4, read at +0x4004
@   21    DTCM region 32 KiB: read at +0x8000, just past the region
@   22    DTCM in load mode: read of an address that holds a word in main RAM
@   23    DTCM off after a load-mode store: the main RAM word
@   24    DTCM on (read/write mode) again: the word the load-mode store wrote
@   25    ITCM (32 KiB) on: write then read at 0
@   26    ITCM region 64 KiB: word written at +4, read at +0x8004
@   27    two-instruction routine copied to ITCM 0x100 and called: r0 on return
@   28    VRAM bank A on (MST 0): STR then LDR at 0x06800000
@   29    STRH then LDRH at 0x06800004
@   30    STRB over a stored word at 0x06800008, LDR of the word
@   31    bank off: LDR at 0x06800000
@   32    LDR of 0x04000090, an address no ARM9 I/O register occupies (GBATEK's ARM9 I/O map
@         lists 0x0400006C, then the DMA registers from 0x040000B0)
@   33    progress: the number of the last finished group (1 main RAM, 2 mirrors, 3 DTCM,
@         4 load mode, 5 ITCM, 6 VRAM, 7 I/O)
@   34    sentinel, written last
@
@ Both TCMs are switched off at the start and before the spin, so the record reads back
@ through the bus. Code runs from main RAM only, away from every TCM region.
    .equ REC, 0x02004000

    .macro rec slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    @ Sets the four TCM bits of the control register (bit 16 DTCM enable, 17 DTCM load mode,
    @ 18 ITCM enable, 19 ITCM load mode) to exactly \bits; the other bits are kept.
    .macro tcm bits
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x000F0000
    .if \bits
    orr r0, r0, #\bits
    .endif
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    .endm

    .section .head, "ax"
    .arm
    .word 0x6D656D6D           @ marker at 0x02000000, read through the 4 MiB mirror
    .space 0xFC, 0

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    tcm 0

    @ ---- group 1: main RAM widths ----
    ldr r9, =0x02100000
    ldr r1, =0xA1B2C3D4
    str r1, [r9]
    ldr r2, [r9]
    rec 0, r2
    ldrh r2, [r9]
    rec 1, r2
    ldrh r2, [r9, #2]
    rec 2, r2
    ldrb r2, [r9]
    rec 3, r2
    ldrb r2, [r9, #3]
    rec 4, r2
    mvn r1, #0
    str r1, [r9, #0x10]
    ldr r1, =0x1234
    strh r1, [r9, #0x12]
    ldr r2, [r9, #0x10]
    rec 5, r2
    ldrh r2, [r9, #0x12]
    rec 6, r2
    mov r1, #0
    str r1, [r9, #0x20]
    mov r1, #0x5A
    strb r1, [r9, #0x21]
    ldr r2, [r9, #0x20]
    rec 7, r2
    ldrb r2, [r9, #0x21]
    rec 8, r2
    add r3, r9, #1
    ldr r2, [r3]
    rec 9, r2
    mov r1, #1
    rec 33, r1

    @ ---- group 2: mirrors ----
    ldr r1, =0x4D415252
    ldr r9, =0x02110000
    str r1, [r9]
    ldr r9, =0x02510000
    ldr r2, [r9]
    rec 10, r2
    ldr r9, =0x02910000
    ldr r2, [r9]
    rec 11, r2
    ldr r9, =0x02D10000
    ldr r2, [r9]
    rec 12, r2
    ldr r1, =0x0B0B0B0B
    ldr r9, =0x02610000
    str r1, [r9]
    ldr r9, =0x02210000
    ldr r2, [r9]
    rec 13, r2
    ldr r1, =0xFEEDF00D
    ldr r9, =0x023FFFFC
    str r1, [r9]
    ldr r2, [r9]
    rec 14, r2
    ldr r9, =0x027FFFFC
    ldr r2, [r9]
    rec 15, r2
    ldr r9, =0x02400000
    ldr r2, [r9]
    rec 16, r2
    ldr r9, =0x02FFFFFC
    ldr r2, [r9]
    rec 17, r2
    mov r1, #2
    rec 33, r1

    @ ---- group 3: DTCM ----
    ldr r9, =0x02300000
    ldr r1, =0x11111111
    str r1, [r9]               @ the main RAM word under the DTCM
    ldr r1, =0x0230000A        @ base 0x02300000, N = 5 (16 KiB)
    mcr p15, 0, r1, c9, c1, 0
    tcm 0x10000
    ldr r1, =0xD7C30001
    str r1, [r9]
    ldr r2, [r9]
    rec 18, r2
    tcm 0
    ldr r2, [r9]
    rec 19, r2
    ldr r1, =0x0230000C        @ N = 6 (32 KiB)
    mcr p15, 0, r1, c9, c1, 0
    add r3, r9, #0x8000
    ldr r1, =0x22222222
    str r1, [r3]               @ the main RAM word just past the region
    tcm 0x10000
    ldr r1, =0xD7C30002
    str r1, [r9, #4]
    add r4, r9, #0x4000
    ldr r2, [r4, #4]
    rec 20, r2
    ldr r2, [r3]
    rec 21, r2
    tcm 0
    mov r1, #3
    rec 33, r1

    @ ---- group 4: DTCM load mode ----
    ldr r1, =0x0230000A
    mcr p15, 0, r1, c9, c1, 0
    ldr r9, =0x02300010
    ldr r1, =0x33333333
    str r1, [r9]               @ the main RAM word under the DTCM
    tcm 0x30000
    ldr r2, [r9]
    rec 22, r2
    ldr r1, =0x44444444
    str r1, [r9]               @ lands in the DTCM
    tcm 0
    ldr r2, [r9]
    rec 23, r2
    tcm 0x10000
    ldr r2, [r9]
    rec 24, r2
    tcm 0
    mov r1, #4
    rec 33, r1

    @ ---- group 5: ITCM ----
    mov r1, #0xC               @ N = 6 (32 KiB); the base is always 0
    mcr p15, 0, r1, c9, c1, 1
    tcm 0x40000
    mov r9, #0
    ldr r1, =0x1BC0DE01
    str r1, [r9]
    ldr r2, [r9]
    rec 25, r2
    mov r1, #0xE               @ N = 7 (64 KiB)
    mcr p15, 0, r1, c9, c1, 1
    ldr r1, =0x1BC0DE02
    str r1, [r9, #4]
    mov r3, #0x8000
    ldr r2, [r3, #4]
    rec 26, r2
    adr r4, itcm_routine
    mov r5, #0x100
    ldmia r4, {r6, r7}
    stmia r5, {r6, r7}
    blx r5
    rec 27, r0
    tcm 0
    mov r1, #5
    rec 33, r1

    @ ---- group 6: VRAM bank A ----
    ldr r9, =0x04000240        @ VRAMCNT_A
    mov r1, #0x80              @ enabled, MST 0
    strb r1, [r9]
    ldr r9, =0x06800000
    ldr r1, =0x56415431
    str r1, [r9]
    ldr r2, [r9]
    rec 28, r2
    ldr r1, =0xBEEF
    strh r1, [r9, #4]
    ldrh r2, [r9, #4]
    rec 29, r2
    ldr r1, =0x11223344
    str r1, [r9, #8]
    mov r1, #0xEE
    strb r1, [r9, #8]
    ldr r2, [r9, #8]
    rec 30, r2
    ldr r10, =0x04000240
    mov r1, #0
    strb r1, [r10]             @ bank off
    ldr r2, [r9]
    rec 31, r2
    mov r1, #6
    rec 33, r1

    @ ---- group 7: undefined I/O ----
    ldr r9, =0x04000090
    ldr r2, [r9]
    rec 32, r2
    mov r1, #7
    rec 33, r1

    tcm 0
    ldr r1, =0x6D656D21
    rec 34, r1
spin:
    b spin

    @ The routine copied into the ITCM: two instructions, "mov r0, #0x42; bx lr".
itcm_routine:
    mov r0, #0x42
    bx lr
    .ltorg
