@ SPDX-License-Identifier: Apache-2.0
@ DMA ROM, ARM7 side: the ARM7's own four DMA channels (docs/hw/dma.md). No libnds, no handshake
@ with the ARM9, whose channels are separate hardware. Loaded and entered at 0x02380000
@ (docs/hw/cart-header.md). The ARM7 takes no IRQ (its vector is BIOS, #27): it only records IF,
@ which stays visible because IME is 0. The ROM records observed values only; the expected ones
@ live in crates/runner/tests/dma.rs (docs/hw/dma.md).
@
@ Channel n is at 0x040000B0 + 12n: SAD +0, DAD +4, CNT +8. SRC (0x02020000) holds the words
@ 11111111h, 22222222h, ... 88888888h; the destination buffers DST* are in
@ 0x02021000-0x02022000, cleared at the start. After an Immediate enable the ROM polls bit 31 of
@ CNT (the busy flag) until it is clear, bounded, because an ARM7 start can be late
@ (docs/hw/dma.md, "Model"). "bit 31" below is DMAnCNT bit 31.
@
@ ARM7 record at 0x02005000, one word per slot:
@   0  DST1 word 0 after an Immediate copy of 3 words (ch 1)   1  DST1 word 2   2  DST1 word 3
@   3  bit 31 after the poll
@   4  DST2 word 0 after a copy on ch 2 written with the count 4001h (the field is 14 bits)
@   5  DST2 word 1      6  the low half of ch 2's CNT read after the transfer
@   7  IF & 0F00h with the copies so far having no IRQ bit set
@   8  IF & 0F00h after an Immediate copy on ch 3 with the IRQ bit set
@   9  IF & 0F00h after writing 0F00h to IF
@   10 DST3 word 0 at line 191, with a VBlank-start copy of 2 words and the IRQ bit armed at
@      line 100 (ch 0)     11 bit 31 of ch 0 at line 191
@   12 DST3 word 0 at line 195     13 DST3 word 1 at line 195
@   14 IF & 0F00h at line 195      15 bit 31 of ch 0 at line 195
@   16 IF & 0F00h after writing 0F00h to IF
@   17 the word at 0x040000E0 after writing DEADBEEFh there (the ARM7 has no fill registers)
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02005000
    .equ SRC, 0x02020000
    .equ DST1, 0x02021000
    .equ DST2, 0x02021100
    .equ DST3, 0x02021200
    .equ DST4, 0x02021300

    .macro RECORD slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    @ Writes channel ch's SAD, DAD and CNT, in that order (the CNT write last).
    .macro DMA ch, sad, dad, cnt
    ldr r0, =\sad
    str r0, [r6, #(12 * \ch)]
    ldr r0, =\dad
    str r0, [r6, #(12 * \ch + 4)]
    ldr r0, =\cnt
    str r0, [r6, #(12 * \ch + 8)]
    .endm

    @ Records the word at an address.
    .macro PEEK slot, addr
    ldr r1, =\addr
    ldr r0, [r1]
    RECORD \slot, r0
    .endm

    @ Records bit 31 of channel ch's CNT.
    .macro BUSY slot, ch
    ldr r0, [r6, #(12 * \ch + 8)]
    lsr r0, r0, #31
    RECORD \slot, r0
    .endm

    @ Polls bit 31 of channel ch's CNT until it is clear (bounded).
    .macro POLL ch
    ldr r3, =0x10000
1:  ldr r0, [r6, #(12 * \ch + 8)]
    tst r0, #0x80000000
    beq 2f
    subs r3, r3, #1
    bne 1b
2:
    .endm

    @ Records IF & 0F00h.
    .macro IFBITS slot
    ldr r0, [r9, #0x214]
    and r0, r0, #0xF00
    RECORD \slot, r0
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: IF +0x214, VCOUNT +6
    add r6, r9, #0xB0          @ DMA0SAD

    ldr r1, =SRC
    ldr r4, =0x11111111
    mov r3, r4
    mov r5, #8
seed:
    str r3, [r1], #4
    add r3, r3, r4
    subs r5, r5, #1
    bne seed
    ldr r1, =DST1
    ldr r4, =0x02022000
    mov r3, #0
clear:
    str r3, [r1], #4
    cmp r1, r4
    bne clear

    @ ---- Immediate copy, polled until the busy flag clears
    DMA 1, SRC, DST1, 0x84000003
    POLL 1
    PEEK 0, DST1
    PEEK 1, DST1 + 8
    PEEK 2, DST1 + 12
    BUSY 3, 1

    @ ---- channel 2 has a 14-bit count: 4001h is the count 1
    DMA 2, SRC, DST2, 0x84004001
    POLL 2
    PEEK 4, DST2
    PEEK 5, DST2 + 4
    ldrh r0, [r6, #(12 * 2 + 8)]
    RECORD 6, r0

    @ ---- IF bits 8 + n only with the IRQ bit
    IFBITS 7
    DMA 3, SRC, DST4, 0xC4000001
    POLL 3
    IFBITS 8
    mov r0, #0xF00
    str r0, [r9, #0x214]
    IFBITS 9

    @ ---- VBlank start, with the IRQ bit, armed at line 100
    ldr r0, =100
    bl wait_line
    DMA 0, SRC, DST3, 0xD4000002
    ldr r0, =191
    bl wait_line
    PEEK 10, DST3
    BUSY 11, 0
    ldr r0, =195
    bl wait_line
    PEEK 12, DST3
    PEEK 13, DST3 + 4
    IFBITS 14
    BUSY 15, 0
    mov r0, #0xF00
    str r0, [r9, #0x214]
    IFBITS 16

    @ ---- no fill registers on the ARM7
    ldr r0, =0xDEADBEEF
    str r0, [r9, #0xE0]
    ldr r0, [r9, #0xE0]
    RECORD 17, r0

    mov r0, #1
    RECORD 30, r0
    ldr r0, =0x444D4137        @ "DMA7"
    RECORD 31, r0
spin:
    b spin

@ Polls VCOUNT until it reads r0.
wait_line:
    ldrh r1, [r9, #6]
    cmp r1, r0
    bne wait_line
    mov pc, lr
    .ltorg
