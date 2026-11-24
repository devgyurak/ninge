@ SPDX-License-Identifier: Apache-2.0
@ Geometry FIFO ROM, ARM9 side (docs/hw/geometry-engine.md). No libnds, no crt0. Loaded at
@ 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ SWAP_BUFFERS holds the geometry engine until the next VBlank, so while it waits the ROM can
@ fill the command FIFO and look at GXSTAT, the FIFO IRQ, the stall of a write to a full FIFO and
@ the DMA start mode 7 bursts. The ROM records what it reads in main RAM; the expected words are in
@ crates/runner/tests/gxfifo.rs, worked out from the note and not from running this ROM.
@
@ Record at 0x02008000 (word slots):
@   0  GXSTAT at power-on.
@   1  GXSTAT after SWAP_BUFFERS (command word, then its parameter): the engine is busy, waiting
@      for VBlank, and the FIFO is empty.
@   2  GXSTAT after ten more MTX_PUSH: the PIPE holds four, the FIFO six.
@   3  IF bit 21 after FIFO IRQ mode 2 (empty) is set: the FIFO is not empty, so clear.
@   4  IF bit 21 after FIFO IRQ mode 1 (less than half full) is set: the FIFO holds 6, so set.
@   5  IF bit 21 after the mode is 0 and the bit is acknowledged.
@   6  GXSTAT after 250 more pushes: PIPE and FIFO are full (the FIFO count is 256).
@   7  VCOUNT right after one more push, which waits for the engine: the write ends once SWAP_BUFFERS
@      ran at line 192.
@   8  GXSTAT once the engine is idle again: 261 pushes in projection mode.
@   9  GXSTAT after bit 15 is written with 1 (the error flag and the projection pointer clear).
@  10  GXSTAT while a 300-word list sent by DMA in start mode 7 waits behind a second SWAP_BUFFERS:
@      two bursts of 112 words went in (the first four words fill the PIPE), the third waits for the
@      FIFO to drain below half.
@  11  DMA0CNT bit 31 at that point: the channel is still enabled.
@  12  IF bit 8 (DMA channel 0) once the channel has finished.
@  13  GXSTAT once the engine is idle: the 300 pushes ran.
@ Slot 30: progress (1 when done). Slot 31: this ROM's sentinel "GXFI".
@
@ TODO(hw-unverified): the entry counts, the burst pause and the stall follow the model of
@ docs/hw/geometry-engine.md ("Model", "Unverified"); a pass says the emulator follows the note, not
@ that hardware was measured.
@
@ The commands are sent without writing POWCNT1, so this ROM relies on the documented POWCNT1 reset
@ value 820Fh having bit 3 (the geometry engine) set (docs/hw/display-mode-vram.md, "Unverified").
    .equ REC, 0x02008000
    .equ LIST, 0x02100000
    .equ GXFIFO, 0x04000400
    .equ GXSTAT, 0x04000600
    .equ IF, 0x04000214
    .equ VCOUNT, 0x04000006
    .equ DMA0SAD, 0x040000B0
    @ DMA control: enable, transfer-complete IRQ, 32-bit units, fixed destination, ARM9 start
    @ timing 7 (the geometry command FIFO), 300 units.
    .equ DMA_GX, 0xFC40012C
    .equ IF_DMA0, 0x100
    .equ IF_GXFIFO, 0x00200000

    @ Record GXSTAT in slot \slot.
    .macro REC_GXSTAT slot
    ldr r0, [r9]
    str r0, [r8, #(\slot * 4)]
    .endm

    @ Record IF masked with \mask in slot \slot.
    .macro REC_IF slot, mask
    ldr r0, [r10]
    ldr r1, =\mask
    and r0, r0, r1
    str r0, [r8, #(\slot * 4)]
    .endm

    @ Wait until GXSTAT bit 27 (the engine is busy) is clear.
    .macro WAIT_IDLE
1:  ldr r0, [r9]
    tst r0, #0x08000000
    bne 1b
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ Protection unit and caches off, so every access reaches the bus.
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop

    ldr r8, =REC
    ldr r7, =GXFIFO
    ldr r9, =GXSTAT
    ldr r10, =IF
    ldr r11, =VCOUNT

    @ Start in a visible line, so SWAP_BUFFERS has a VBlank ahead of it.
wait_line:
    ldrh r0, [r11]
    cmp r0, #10
    bne wait_line

    @ ---- Slot 0-2: the engine busy behind SWAP_BUFFERS, the queue filling.
    REC_GXSTAT 0
    mov r0, #0x50              @ SWAP_BUFFERS
    str r0, [r7]
    mov r0, #0
    str r0, [r7]               @ its parameter
    REC_GXSTAT 1
    mov r0, #0x11              @ MTX_PUSH
    mov r3, #10
push10:
    str r0, [r7]
    subs r3, r3, #1
    bne push10
    REC_GXSTAT 2

    @ ---- Slot 3-5: the FIFO IRQ follows the FIFO's level.
    ldr r0, =0x80000000        @ mode 2: empty
    str r0, [r9]
    REC_IF 3, IF_GXFIFO
    ldr r0, =0x40000000        @ mode 1: less than half full
    str r0, [r9]
    REC_IF 4, IF_GXFIFO
    mov r0, #0
    str r0, [r9]               @ mode 0
    ldr r0, =IF_GXFIFO
    str r0, [r10]              @ acknowledge
    REC_IF 5, IF_GXFIFO

    @ ---- Slot 6-7: fill the FIFO, then one write that has to wait.
    mov r0, #0x11
    ldr r3, =250
fill:
    str r0, [r7]
    subs r3, r3, #1
    bne fill
    REC_GXSTAT 6
    mov r0, #0x11
    str r0, [r7]               @ the 261st entry: held until SWAP_BUFFERS has run
    ldrh r1, [r11]
    str r1, [r8, #(7 * 4)]

    @ ---- Slot 8-9: the queue drains; clear the error flag the pushes raised.
    WAIT_IDLE
    REC_GXSTAT 8
    mov r0, #0x8000
    str r0, [r9]
    REC_GXSTAT 9

    @ ---- Slot 10-13: a display list by DMA while the engine waits again.
    ldr r4, =LIST
    mov r0, #0x11
    ldr r3, =300
build_list:
    str r0, [r4], #4
    subs r3, r3, #1
    bne build_list
    mov r0, #0x50              @ SWAP_BUFFERS, to hold the engine
    str r0, [r7]
    mov r0, #0
    str r0, [r7]
    ldr r4, =DMA0SAD
    ldr r1, =LIST
    str r1, [r4]               @ DMA0SAD
    str r7, [r4, #4]           @ DMA0DAD = GXFIFO
    ldr r1, =DMA_GX
    str r1, [r4, #8]           @ DMA0CNT: enable
    ldr r3, =3000              @ time for the two bursts
settle:
    subs r3, r3, #1
    bne settle
    REC_GXSTAT 10
    ldr r1, [r4, #8]
    mov r1, r1, lsr #31
    str r1, [r8, #(11 * 4)]
    @ Wait until the channel clears its enable bit; bounded, so a channel that never finishes
    @ fails the check instead of hanging the ROM.
    ldr r3, =0x100000
wait_dma:
    ldr r1, [r4, #8]
    tst r1, #0x80000000
    beq dma_done
    subs r3, r3, #1
    bne wait_dma
dma_done:
    REC_IF 12, IF_DMA0
    WAIT_IDLE
    REC_GXSTAT 13

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x49465847        @ "GXFI"
    str r0, [r8, #(31 * 4)]
spin:
    b spin
    .ltorg
