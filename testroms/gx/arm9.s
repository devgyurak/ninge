@ SPDX-License-Identifier: Apache-2.0
@ Geometry engine stub ROM, ARM9 side (docs/hw/gx-stub.md). No libnds, no crt0. Loaded at
@ 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ The ARM9 sends geometry commands, reads GXSTAT and IF after each step, records what it read in
@ main RAM and shows six checks as 32-row bands on engine A's screen (the bottom one, display swap
@ clear): green passes, red fails. The expected words are in crates/runner/tests/gx.rs and are
@ worked out from the note, not from running this ROM.
@
@ Record at 0x02008000 (word slots; the halfword band colours are at slots 20-25):
@   0  GXSTAT at power-on.
@   1  GXSTAT after MTX_MODE 1 sent with one STM (a command word and its parameter in two
@      consecutive GXFIFO mirrors) and one packed word of three MTX_PUSH.
@   2  After an MTX_PUSH through its command port (a dummy write).
@   3  After an MTX_POP through its port, with parameter 1.
@   4  After SHININESS (34h) sent through GXFIFO with its 32 parameters (each the word 11h, which
@      would be a push if taken as a command) and one more word 11h, a real push.
@   5  After 32 more pushes, eight packed words of four: the coordinate pointer passes entry 31.
@   6  After GXSTAT bit 15 is written with 1.
@   7  After MTX_MODE 0 and one push: the projection pointer is 1.
@   8  After a second push: the projection stack overflows.
@   9  After GXSTAT bit 15 is written with 1.
@  10  After a 300-word display list (MTX_MODE 1, then 298 pushes) is sent by DMA in start mode 7.
@  11  DMA0CNT bit 31 after the transfer: the channel cleared its enable bit.
@  12  IF bit 8 (DMA channel 0), raised by the transfer-complete IRQ.
@  13  IF bit 21 after GXSTAT bits 30-31 are set to 1 (FIFO IRQ: less than half full).
@  14  IF bit 21 after an acknowledge written to IF bit 21 while the FIFO IRQ is on.
@  15  IF bit 21 after the mode is set to 0 and the bit is acknowledged.
@ Slot 30: progress (1 when done). Slot 31: this ROM's sentinel "GXST".
@
@ TODO(hw-unverified): the expected words of slots 4, 5, 8, 10 and 15 follow stub choices that
@ docs/hw/gx-stub.md lists under "Unverified" (SHININESS takes 32 parameters; the coordinate
@ pointer moves and entries 1Fh and above raise the error flag; the projection pointer wraps in one
@ bit; IF bit 21 after the mode goes to 0). A green band says the stub follows the note, not that
@ hardware was measured.
@
@ The commands are sent before POWCNT1 is written (at the end, to show the bands), so this ROM
@ relies on the documented POWCNT1 reset value 820Fh having bit 3 (the geometry engine) set
@ (docs/hw/display-mode-vram.md, "Unverified"); with bit 3 clear every command is dropped.
    .equ REC, 0x02008000
    .equ LIST, 0x02100000
    .equ GXFIFO, 0x04000400
    .equ GXSTAT, 0x04000600
    .equ IF, 0x04000214
    .equ DMA0SAD, 0x040000B0
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000
    @ DMA control: enable, transfer-complete IRQ, 32-bit units, fixed destination, ARM9 start
    @ timing 7 (the geometry command FIFO), 300 units.
    .equ DMA_GX, 0xFC40012C
    .equ IF_DMA0, 0x100
    .equ IF_GXFIFO, 0x00200000

    @ The first comparison of a band: slot \slot against \val.
    .macro FIRST slot, val
    ldr r0, [r8, #(\slot * 4)]
    ldr r2, =\val
    cmp r0, r2
    .endm

    @ A further comparison, made only while the band still passes.
    .macro NEXT slot, val
    ldreq r0, [r8, #(\slot * 4)]
    ldreq r2, =\val
    cmpeq r0, r2
    .endm

    @ Writes band \idx green (Z set) or red.
    .macro PAINT idx
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #((20 + \idx) * 4)]
    .endm

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

    @ ---- Band 0: reset value, STM, a packed word.
    REC_GXSTAT 0
    mov r0, #0x10              @ MTX_MODE
    mov r1, #1                 @ position matrix mode
    stmia r7, {r0, r1}
    ldr r0, =0x00111111        @ three MTX_PUSH, the fourth byte NOP
    str r0, [r7]
    REC_GXSTAT 1

    @ ---- Band 1: the command ports, a 32-parameter command.
    mov r0, #0
    str r0, [r7, #0x44]        @ MTX_PUSH port, a dummy write
    REC_GXSTAT 2
    mov r0, #1
    str r0, [r7, #0x48]        @ MTX_POP port, parameter 1
    REC_GXSTAT 3
    mov r0, #0x34              @ SHININESS: 32 parameters follow
    str r0, [r7]
    mov r0, #0x11              @ the parameter and, last, a real push
    mov r3, #32
shininess:
    str r0, [r7]
    subs r3, r3, #1
    bne shininess
    str r0, [r7]
    REC_GXSTAT 4

    @ ---- Band 2: the coordinate pointer passes entry 31.
    ldr r0, =0x11111111
    mov r3, #8
pushes:
    str r0, [r7]
    subs r3, r3, #1
    bne pushes
    REC_GXSTAT 5
    mov r0, #0x8000
    str r0, [r9]               @ write 1 to bit 15: clear the error
    REC_GXSTAT 6

    @ ---- Band 3: the one-entry projection stack.
    mov r0, #0x10
    mov r1, #0                 @ projection matrix mode
    stmia r7, {r0, r1}
    mov r0, #0x11
    str r0, [r7]
    REC_GXSTAT 7
    mov r0, #0x11              @ REC_GXSTAT used r0
    str r0, [r7]
    REC_GXSTAT 8
    mov r0, #0x8000
    str r0, [r9]
    REC_GXSTAT 9

    @ ---- Band 4: a display list by DMA in start mode 7. MTX_MODE 1, then 298 pushes.
    ldr r4, =LIST
    mov r0, #0x10
    str r0, [r4], #4
    mov r0, #1
    str r0, [r4], #4
    mov r0, #0x11
    ldr r3, =298
build_list:
    str r0, [r4], #4
    subs r3, r3, #1
    bne build_list
    ldr r0, =DMA0SAD
    ldr r1, =LIST
    str r1, [r0]               @ DMA0SAD
    str r7, [r0, #4]           @ DMA0DAD = GXFIFO
    ldr r1, =DMA_GX
    str r1, [r0, #8]           @ DMA0CNT: enable
    @ Wait until the channel clears its enable bit; bounded, so a channel that never starts fails
    @ the check instead of hanging the ROM.
    ldr r3, =0x10000
wait_dma:
    ldr r1, [r0, #8]
    tst r1, #0x80000000
    beq dma_done
    subs r3, r3, #1
    bne wait_dma
dma_done:
    REC_GXSTAT 10
    ldr r1, [r0, #8]
    mov r1, r1, lsr #31
    str r1, [r8, #(11 * 4)]
    REC_IF 12, IF_DMA0

    @ ---- Band 5: the FIFO IRQ is level driven.
    ldr r0, =0x40000000        @ FIFO IRQ mode 1: less than half full
    str r0, [r9]
    REC_IF 13, IF_GXFIFO
    ldr r0, =IF_GXFIFO
    str r0, [r10]              @ an acknowledge has no effect while the condition holds
    REC_IF 14, IF_GXFIFO
    mov r0, #0
    str r0, [r9]               @ mode 0
    ldr r0, =IF_GXFIFO
    str r0, [r10]              @ now the acknowledge counts
    REC_IF 15, IF_GXFIFO

    @ ---- Judge the record.
    ldr r5, =0x03E0
    ldr r6, =0x001F

    FIRST 0, 0x06000000
    NEXT 1, 0x06000300
    PAINT 0
    FIRST 2, 0x06000400
    NEXT 3, 0x06000300
    NEXT 4, 0x06000400
    PAINT 1
    FIRST 5, 0x06008400
    NEXT 6, 0x06000400
    PAINT 2
    FIRST 7, 0x06002400
    NEXT 8, 0x06008400
    NEXT 9, 0x06000400
    PAINT 3
    FIRST 10, 0x06008E00
    NEXT 11, 0
    NEXT 12, 0x100
    PAINT 4
    FIRST 13, 0x00200000
    NEXT 14, 0x00200000
    NEXT 15, 0
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
    ldr r0, =0x54535847        @ "GXST"
    str r0, [r8, #(31 * 4)]
spin:
    b spin
    .ltorg
