@ SPDX-License-Identifier: Apache-2.0
@ VRAM-banks ROM, ARM7 side: the ARM7's own mapping of banks C and D (docs/hw/vram-banks.md,
@ "VRAMSTAT (ARM7)"). No libnds. Loaded and entered at 0x02380000. The ROM records observed values
@ only; the expected ones live in crates/runner/tests/vrambanks.rs.
@
@ The two CPUs take turns through IPCSYNC (docs/hw/ipc.md). In step s (1-8) the ARM9 sets banks C
@ and D (the `arm7_steps` table in arm9.s), then posts s in IPCSYNC bits 8-11; this side waits for
@ s in bits 0-3, records what it sees, and posts s back. Banks hold the markers the ARM9 stored in
@ them in its group 1 (first word 0x56000000 | i, last word 0xE0000000 | i; C is 2, D is 3).
@
@ Record at 0x02005000, one word per slot; step s uses slots 8 * (s - 1) onward:
@   step 1 (C: MST 2, OFS 0)        0 VRAMSTAT   1 word at 0x06000000   2 word at 0x0601FFFC
@                                   3 word at 0x06020000   4 STR then LDR at 0x06000010
@                                   5 STRB over it, LDR   6 STRH then LDRH at 0x06000014
@   step 2 (C: MST 2, OFS 1)        8 VRAMSTAT   9 word at 0x06020000   10 word at 0x0603FFFC
@                                   11 word at 0x06000000   12 word at 0x06020010 (the stored one)
@   step 3 (C and D: MST 2, OFS 0)  16 VRAMSTAT   17 word at 0x06000000   18 word at 0x0601FFFC
@   step 4 (C OFS 0, D OFS 1)       24 VRAMSTAT   25 word at 0x06000000   26 word at 0x06020000
@                                   27 word at 0x0603FFFC
@   step 5 (C: MST 0, D: MST 1)     32 VRAMSTAT   33 word at 0x06000000   34 word at 0x06020000
@   step 6 (C, D: MST 2, disabled)  40 VRAMSTAT   41 word at 0x06000000
@   step 7 (C: MST 2, OFS 3)        48 VRAMSTAT   49 word at 0x06020000   50 word at 0x06000000
@   step 8 (C: MST 2, OFS 2)        56 VRAMSTAT   57 word at 0x06000000   58 word at 0x06020000
@   62 progress: the number of the last finished step   63 sentinel, written last
    .equ REC, 0x02005000
    .equ IPCSYNC, 0x04000180
    .equ VRAMSTAT, 0x04000240

    @ Waits until the ARM9's nibble in IPCSYNC bits 0-3 is \step.
    .macro WAIT step
1:  ldrh r0, [r6]
    and r0, r0, #0xF
    cmp r0, #\step
    bne 1b
    .endm

    @ Posts \step in bits 8-11 and records it as the progress.
    .macro DONE step
    mov r0, #(\step << 8)
    strh r0, [r6]
    mov r1, #\step
    str r1, [r8, #(62 * 4)]
    .endm

    .macro RECORD slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    @ VRAMSTAT (a byte read) into \slot.
    .macro STAT slot
    ldrb r1, [r7]
    RECORD \slot, r1
    .endm

    @ The word at \addr into \slot.
    .macro WORD slot, addr
    ldr r2, =\addr
    ldr r1, [r2]
    RECORD \slot, r1
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r6, =IPCSYNC
    ldr r7, =VRAMSTAT
    ldr r8, =REC

    WAIT 1
    STAT 0
    WORD 1, 0x06000000
    WORD 2, 0x0601FFFC
    WORD 3, 0x06020000
    ldr r2, =0x06000010
    ldr r3, =0x11223344
    str r3, [r2]
    ldr r1, [r2]
    RECORD 4, r1
    mov r3, #0xEE
    strb r3, [r2]                @ a byte store: allowed in the ARM7's own mapping
    ldr r1, [r2]
    RECORD 5, r1
    ldr r2, =0x06000014
    ldr r3, =0xBEEF
    strh r3, [r2]
    ldrh r1, [r2]
    RECORD 6, r1
    DONE 1

    WAIT 2
    STAT 8
    WORD 9, 0x06020000
    WORD 10, 0x0603FFFC
    WORD 11, 0x06000000
    WORD 12, 0x06020010
    DONE 2

    WAIT 3
    STAT 16
    WORD 17, 0x06000000
    WORD 18, 0x0601FFFC
    DONE 3

    WAIT 4
    STAT 24
    WORD 25, 0x06000000
    WORD 26, 0x06020000
    WORD 27, 0x0603FFFC
    DONE 4

    WAIT 5
    STAT 32
    WORD 33, 0x06000000
    WORD 34, 0x06020000
    DONE 5

    WAIT 6
    STAT 40
    WORD 41, 0x06000000
    DONE 6

    WAIT 7
    STAT 48
    WORD 49, 0x06020000
    WORD 50, 0x06000000
    DONE 7

    WAIT 8
    STAT 56
    WORD 57, 0x06000000
    WORD 58, 0x06020000
    DONE 8

    ldr r1, =0x56375421
    RECORD 63, r1
spin:
    b spin
    .ltorg
