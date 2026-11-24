@ SPDX-License-Identifier: Apache-2.0
@ Geometry polygon ROM, ARM9 side (docs/hw/geometry-engine.md, "Vertices", "Clipping", "Polygon
@ RAM, Vertex RAM and the double buffer"). No libnds, no crt0. Loaded at 0x02000000 and entered
@ there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ With the matrices at their reset (the identity) the ARM9 sends separate triangles, a triangle strip
@ and a triangle with a corner beyond the right plane, waits for the geometry engine to be idle and
@ reads RAM_COUNT and DISP3DCNT; then it fills Polygon RAM past its 2048 polygons, reads the
@ overflow flag (DISP3DCNT bit 13), acknowledges it, sends SWAP_BUFFERS and reads RAM_COUNT again.
@ The expected words are in crates/runner/tests/gxpoly.rs, worked out by hand from the note and not
@ from running this ROM.
@
@ Record at 0x02008000 (word slots):
@   0  RAM_COUNT after a triangle and a four-vertex strip (1 + 2 polygons, 3 + 4 vertices).
@   1  DISP3DCNT then: no overflow.
@   2  RAM_COUNT after a triangle with one corner beyond the right plane: it is clipped to four
@      corners, so +1 polygon and +4 vertices.
@   3  RAM_COUNT after 2045 more triangles: Polygon RAM holds 2048, the last triangle was dropped.
@   4  DISP3DCNT then: bit 13, the overflow flag.
@   5  DISP3DCNT after a write of 1 to bit 13.
@   6  RAM_COUNT after SWAP_BUFFERS: the set the geometry engine fills is empty again.
@ Slot 30: progress (1 when done). Slot 31: this ROM's sentinel "GXPO".
@
@ TODO(hw-unverified): the clipping and the overflow behaviour follow the model of
@ docs/hw/geometry-engine.md ("Model", "Unverified"); a pass says the emulator follows the note, not
@ that hardware was measured. The ROM relies on the documented POWCNT1 reset value 820Fh having
@ bit 3 (the geometry engine) set (docs/hw/display-mode-vram.md, "Unverified").
    .equ REC, 0x02008000
    .equ GXFIFO, 0x04000400
    .equ GXSTAT, 0x04000600
    .equ RAM_COUNT, 0x04000604
    .equ DISP3DCNT, 0x04000060

    @ Wait until GXSTAT bit 27 (the engine is busy) is clear.
    .macro WAIT_IDLE
1:  ldr r0, [r9]
    tst r0, #0x08000000
    bne 1b
    .endm

    @ BEGIN_VTXS with primitive \p.
    .macro BEGIN p
    mov r0, #0x40
    str r0, [r7]
    mov r0, #\p
    str r0, [r7]
    .endm

    @ VTX_16 at (\x, \y, \z), each 0 to 0x7FFF.
    .macro VTX x, y, z
    mov r0, #0x23
    str r0, [r7]
    ldr r0, =((\y << 16) | \x)
    str r0, [r7]
    ldr r0, =\z
    str r0, [r7]
    .endm

    @ Record the word at \addr in slot \slot.
    .macro REC_WORD addr, slot
    ldr r1, =\addr
    ldr r0, [r1]
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

    @ A triangle, then a triangle strip of four vertices (two triangles sharing two).
    BEGIN 0
    VTX 0, 0, 0
    VTX 0x800, 0, 0
    VTX 0, 0x800, 0
    BEGIN 2
    VTX 0, 0, 0
    VTX 0x100, 0, 0
    VTX 0, 0x100, 0
    VTX 0x100, 0x100, 0
    WAIT_IDLE
    REC_WORD RAM_COUNT, 0
    REC_WORD DISP3DCNT, 1

    @ A triangle whose second corner is at x = 2.0, beyond the right plane.
    BEGIN 0
    VTX 0, 0, 0
    VTX 0x2000, 0, 0
    VTX 0, 0x800, 0
    WAIT_IDLE
    REC_WORD RAM_COUNT, 2

    @ 2045 more triangles: the 2049th polygon does not fit in Polygon RAM.
    BEGIN 0
    ldr r3, =2045
fill:
    VTX 0, 0, 0
    VTX 0x800, 0, 0
    VTX 0, 0x800, 0
    subs r3, r3, #1
    bne fill
    WAIT_IDLE
    REC_WORD RAM_COUNT, 3
    REC_WORD DISP3DCNT, 4
    ldr r1, =DISP3DCNT
    mov r0, #0x2000
    strh r0, [r1]              @ write 1 to bit 13: acknowledge
    REC_WORD DISP3DCNT, 5

    @ SWAP_BUFFERS (it waits for VBlank), then the set it hands back is empty.
    mov r0, #0x50
    str r0, [r7]
    mov r0, #0
    str r0, [r7]
    WAIT_IDLE
    REC_WORD RAM_COUNT, 6

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4F505847        @ "GXPO"
    str r0, [r8, #(31 * 4)]
spin:
    b spin
    .ltorg
