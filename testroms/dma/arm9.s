@ SPDX-License-Identifier: Apache-2.0
@ DMA ROM, ARM9 side: the ARM9's four DMA channels (docs/hw/dma.md). No libnds, no crt0, no
@ handshake with the ARM7 (its channels are separate hardware). Loaded and entered at 0x02000000
@ (docs/hw/cart-header.md). The ROM records observed values only; the expected ones live in
@ crates/runner/tests/dma.rs, each citing docs/hw/dma.md.
@
@ Channel n is at 0x040000B0 + 12n: SAD +0, DAD +4, CNT +8 (count in bits 0-20, control above).
@ Fill register n is at 0x040000E0 + 4n. SRC (0x02008000) holds the words 11111111h, 22222222h,
@ ... 88888888h. Every destination buffer DST* is in 0x02009000-0x0200F000, cleared at the start.
@ "bit 31" below is DMAnCNT bit 31, the enable (busy) flag, read as 0 or 1.
@
@ ARM9 record at 0x02004000, one word per slot:
@   0  DST1 word 0 after an Immediate 32-bit copy of 4 words (ch 0)    1  DST1 word 3    2  DST1 word 4
@   3  DST2 word 0 after an Immediate 16-bit copy of 3 halfwords (ch 1)   4  DST2 word 1
@   5  DST3 word 0 after a 3-word copy with the destination decrementing from DST3 (ch 2)
@   6  DST3 word -1        7  DST3 word -2        8  DST3 word +1
@   9  DST4 word 0 after a 3-word copy with a fixed destination (ch 3)    10  DST4 word 1
@   11 DST5 word 0 after a 3-word copy with the source decrementing from SRC word 2 (ch 0)
@   12 DST5 word 1         13 DST5 word 2
@   14 DST6 word 0 after a 3-word copy with a fixed source, SRC word 1 (ch 1)  15 DST6 word 2
@   16 DST6 word 3
@   17 bit 31 read right after the Immediate enable of a 1-word copy (ch 1)
@   18 bit 31 after a short delay      19 DST7 word 0 (the copied word)
@   20 DMA0 fill register read back after writing CAFEF00Dh
@   21 DST8 word 0 after 4 words from the fill register, source fixed (ch 0)   22 DST8 word 3
@   23 DST8 word 4
@   24 fill register 1 after a 2-word copy into 0x040000E4 (ch 0)       25 fill register 2
@   26 DST13 word 0 after a 1-word copy from SRC+2 to DST13+2 (aligned down)  27 DST13 word 1
@   28 DST10 word 0 at line 191, with a VBlank-start copy of 2 words armed at line 100 (ch 2)
@   29 bit 31 of ch 2 at line 191      30 DST9 word 0 at line 191 (ch 1, VBlank, repeat, below)
@   31 bit 31 of ch 1 at line 191
@   32 DST10 word 0 at line 195          33 DST10 word 1 at line 195
@   34 DST14 word 0 at line 195: ch 2's registers were rewritten (new source, destination and
@      count 1) while enable stayed 1, after the arm
@   35 bit 31 of ch 2 at line 195      36 DST9 word 0 at line 195 (first VBlank)
@   37 bit 31 of ch 1 at line 195      38 DST11 word 0 at line 195 (ch 0 and ch 3 both start at the
@      VBlank and write it; ch 0 writes SRC word 0, ch 3 SRC word 1)
@   ch 1 is armed with source SRC, destination DST9 increment-and-reload, count 1, VBlank start,
@   repeat, 32-bit: each VBlank copies one more word of SRC to DST9 word 0.
@   39 DST9 word 0 at line 195 of the next frame (second VBlank)
@   40 DST15 word 0 at that line: ch 2 armed with a VBlank start at line 100, enable cleared at once
@   41 bit 31 of ch 2 at that line
@   42 DST9 word 0 at line 195 of the third frame     43 DST9 word 1 then
@   44 bit 31 of ch 1 after its enable was cleared
@   45 words of HB1 (256 words) that are not 0 after ch 0 (source fixed, destination incrementing,
@      repeat, count 1, HBlank start) ran from the start of line 10 to the start of line 60
@   46 the same count in HB2 after the channel ran from the start of line 100 to the start of
@      line 100 of the next frame
@   47 IF & 0F00h before the IRQ part (nothing so far had the IRQ bit set)
@   48 IRQs the handler took, ch 0 and ch 3 each ending an Immediate copy with the IRQ bit set
@   49 OR of the IF values the handler saw      50 IF & 0F00h after
@   51 CPU read of 0x02700000 with the DTCM mapped there (written with 0DD0DD0Dh)
@   52 DST12 word 0 after a DMA read of 0x02700000: the main-RAM word behind the DTCM
@   53 CPU read of 0x02300010 after a DMA write of 11111111h to 0x02700010
@   54 CPU read of 0x02700010 (the DTCM, written 0 by the CPU)
@   55 CPU read of 0x02300000 (the main-RAM word behind the DTCM, BEEF0001h)
@ Slot 62: progress (1 when done). Slot 63: sentinel, written last.
    .equ REC, 0x02004000
    .equ SRC, 0x02008000
    .equ DST1, 0x02009000
    .equ DST2, 0x02009100
    .equ DST3, 0x02009210
    .equ DST4, 0x02009300
    .equ DST5, 0x02009400
    .equ DST6, 0x02009500
    .equ DST7, 0x02009600
    .equ DST8, 0x02009700
    .equ DST9, 0x02009800
    .equ DST10, 0x02009900
    .equ DST14, 0x02009A00
    .equ DST15, 0x02009B00
    .equ DST11, 0x02009C00
    .equ DST13, 0x02009D00
    .equ DST12, 0x0200C000
    .equ DST16, 0x0200D000
    .equ DST17, 0x0200D100
    .equ HB1, 0x0200E000
    .equ HB2, 0x0200F000

    .macro RECORD slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    @ Spins for about n * 3 instructions.
    .macro SPIN n
    mov r0, #\n
1:  subs r0, r0, #1
    bne 1b
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

    @ Waits (bounded) until the handler count at [r2] reaches n.
    .macro WAIT_COUNT n
    ldr r3, =0x10000
1:  ldr r0, [r2]
    cmp r0, #\n
    bhs 2f
    subs r3, r3, #1
    bne 1b
2:
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: IME +0x208, IE +0x210, IF +0x214, VCOUNT +6
    add r6, r9, #0xB0          @ DMA0SAD
    @ Entry state of CP15 is not stated (direct-boot.md): clear the protection unit enable
    @ (bit 0), data cache enable (bit 2), instruction cache enable (bit 12) and the high-vectors
    @ bit (13), so the IRQ vector is at 0x18.
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
    ldr r2, =0x02006000        @ handler counters: IRQ count, OR of the IF values it saw
    mov r1, #0
    str r1, [r2]
    str r1, [r2, #4]

    @ SRC = 11111111h, 22222222h, ... 88888888h; clear the destination buffers.
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
    ldr r4, =0x02010000
    mov r3, #0
clear:
    str r3, [r1], #4
    cmp r1, r4
    bne clear

    @ ---- Immediate copies: width, destination and source control
    DMA 0, SRC, DST1, 0x84000004          @ 4 words, both incrementing
    PEEK 0, DST1
    PEEK 1, DST1 + 12
    PEEK 2, DST1 + 16
    DMA 1, SRC, DST2, 0x80000003          @ 3 halfwords
    PEEK 3, DST2
    PEEK 4, DST2 + 4
    DMA 2, SRC, DST3, 0x84200003          @ destination decrement
    PEEK 5, DST3
    PEEK 6, DST3 - 4
    PEEK 7, DST3 - 8
    PEEK 8, DST3 + 4
    DMA 3, SRC, DST4, 0x84400003          @ destination fixed
    PEEK 9, DST4
    PEEK 10, DST4 + 4
    DMA 0, SRC + 8, DST5, 0x84800003      @ source decrement
    PEEK 11, DST5
    PEEK 12, DST5 + 4
    PEEK 13, DST5 + 8
    DMA 1, SRC + 4, DST6, 0x85000003      @ source fixed
    PEEK 14, DST6
    PEEK 15, DST6 + 8
    PEEK 16, DST6 + 12

    @ ---- the busy flag around an Immediate copy
    DMA 1, SRC, DST7, 0x84000001
    BUSY 17, 1                          @ the very next instruction after the enable write
    SPIN 20
    BUSY 18, 1
    PEEK 19, DST7

    @ ---- the fill register as a fixed source, and a register as a destination
    ldr r0, =0xCAFEF00D
    str r0, [r9, #0xE0]
    ldr r0, [r9, #0xE0]
    RECORD 20, r0
    DMA 0, 0x040000E0, DST8, 0x85000004
    PEEK 21, DST8
    PEEK 22, DST8 + 12
    PEEK 23, DST8 + 16
    DMA 0, SRC, 0x040000E4, 0x84000002  @ words 11111111h, 22222222h into fill registers 1, 2
    ldr r0, [r9, #0xE4]
    RECORD 24, r0
    ldr r0, [r9, #0xE8]
    RECORD 25, r0

    @ ---- addresses are aligned down to the unit
    DMA 0, SRC + 2, DST13 + 2, 0x84000001
    PEEK 26, DST13
    PEEK 27, DST13 + 4

    @ ---- IRQ at the end: IF bits 8 + n, taken through the handler at 0x18
    ldr r0, [r9, #0x214]
    and r0, r0, #0xF00
    RECORD 47, r0
    mov r0, #0xF00
    str r0, [r9, #0x214]       @ clear IF
    str r0, [r9, #0x210]       @ IE = DMA 0-3
    mov r0, #1
    str r0, [r9, #0x208]       @ IME
    mov r0, #0x1F
    msr cpsr_c, r0             @ I clear, System mode
    DMA 0, SRC, DST16, 0xC4000001
    WAIT_COUNT 1
    DMA 3, SRC, DST17, 0xC4000001
    WAIT_COUNT 2
    mov r0, #0x9F
    msr cpsr_c, r0             @ I set again
    mov r0, #0
    str r0, [r9, #0x208]
    ldr r0, [r2]
    RECORD 48, r0
    ldr r0, [r2, #4]
    RECORD 49, r0
    ldr r0, [r9, #0x214]
    and r0, r0, #0xF00
    RECORD 50, r0

    @ ---- DMA does not see the DTCM: the DTCM is mapped over main RAM at 0x02700000, which
    @ mirrors 0x02300000 (arm9-memory-map.md)
    ldr r0, =0x0270000A        @ base 0x02700000, size field 5 (16 KiB)
    mcr p15, 0, r0, c9, c1, 0
    mrc p15, 0, r0, c1, c0, 0
    orr r0, r0, #0x00010000
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    ldr r1, =0x02300000
    ldr r0, =0xBEEF0001
    str r0, [r1]
    ldr r1, =0x02700000
    ldr r0, =0x0DD0DD0D
    str r0, [r1]
    mov r0, #0
    str r0, [r1, #0x10]
    ldr r0, [r1]
    RECORD 51, r0
    DMA 0, 0x02700000, DST12, 0x84000001
    PEEK 52, DST12
    DMA 0, SRC, 0x02700010, 0x84000001
    PEEK 53, 0x02300010
    ldr r1, =0x02700000
    ldr r0, [r1, #0x10]
    RECORD 54, r0
    PEEK 55, 0x02300000
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00010000
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop

    @ ---- VBlank-start transfers armed at line 100 of one frame
    ldr r0, =100
    bl wait_line
    DMA 0, SRC, DST11, 0x8C000001
    DMA 3, SRC + 4, DST11, 0x8C000001
    DMA 1, SRC, DST9, 0x8E600001
    DMA 2, SRC, DST10, 0x8C000002
    DMA 2, SRC + 8, DST14, 0x8C000001    @ rewritten with enable still 1: not latched again
    ldr r0, =191
    bl wait_line
    PEEK 28, DST10
    BUSY 29, 2
    PEEK 30, DST9
    BUSY 31, 1
    ldr r0, =195
    bl wait_line
    PEEK 32, DST10
    PEEK 33, DST10 + 4
    PEEK 34, DST14
    BUSY 35, 2
    PEEK 36, DST9
    BUSY 37, 1
    PEEK 38, DST11

    @ ---- second frame: ch 1 repeats; ch 2 armed, then its enable cleared before the VBlank
    ldr r0, =100
    bl wait_line
    DMA 2, SRC, DST15, 0x8C000001
    ldr r0, =0x0C000001
    str r0, [r6, #(12 * 2 + 8)]
    ldr r0, =195
    bl wait_line
    PEEK 39, DST9
    PEEK 40, DST15
    BUSY 41, 2

    @ ---- third frame
    ldr r0, =100
    bl wait_line
    ldr r0, =195
    bl wait_line
    PEEK 42, DST9
    PEEK 43, DST9 + 4
    mov r0, #0
    str r0, [r6, #(12 * 1 + 8)]        @ enable cleared: the repeat stops
    BUSY 44, 1

    @ ---- HBlank start: units counted over visible lines (docs/hw/dma.md, "Starting")
    ldr r0, =10
    bl wait_line
    DMA 0, SRC, HB1, 0x97000001
    ldr r0, =60
    bl wait_line
    mov r0, #0
    str r0, [r6, #8]
    ldr r0, =HB1
    bl count_nz
    RECORD 45, r0
    ldr r0, =100
    bl wait_line
    DMA 0, SRC, HB2, 0x97000001
    ldr r0, =99
    bl wait_line
    ldr r0, =100
    bl wait_line
    mov r0, #0
    str r0, [r6, #8]
    ldr r0, =HB2
    bl count_nz
    RECORD 46, r0

    mov r0, #1
    RECORD 62, r0
    ldr r0, =0x444D4139        @ "DMA9"
    RECORD 63, r0
spin:
    b spin

@ Polls VCOUNT until it reads r0.
wait_line:
    ldrh r1, [r9, #6]
    cmp r1, r0
    bne wait_line
    mov pc, lr

@ Counts the words that are not 0 in the 256 words at r0; returns the count in r0.
count_nz:
    mov r3, #0
    mov r4, #256
count_loop:
    ldr r5, [r0], #4
    cmp r5, #0
    addne r3, r3, #1
    subs r4, r4, #1
    bne count_loop
    mov r0, r3
    mov pc, lr
    .ltorg

@ The IRQ handler, copied to the vector at ITCM 0x18. It uses r10-r12, which the main code
@ leaves alone (IRQ mode banks only r13 and r14), acknowledges IF, counts and ORs what it saw.
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
    ldr r10, [r12, #4]
    orr r10, r10, r11
    str r10, [r12, #4]
    subs pc, lr, #4
handler_end:
    .ltorg
