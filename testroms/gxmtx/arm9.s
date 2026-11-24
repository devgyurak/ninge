@ SPDX-License-Identifier: Apache-2.0
@ Geometry matrix ROM, ARM9 side (docs/hw/geometry-engine.md, "Matrices"). No libnds, no crt0.
@ Loaded at 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ The ARM9 sends two display lists (unpacked commands, one GXFIFO word at a time) that build the
@ position, vector and projection matrices, waits for the geometry engine to be idle and copies the
@ result registers into main RAM. The expected words are in crates/runner/tests/gxmtx.rs, worked out
@ by hand from the note and not from running this ROM.
@
@ List 1: matrix mode 2, identity, a quarter turn about Z (MTX_MULT_3x3), a scale by 2 (which must
@ leave the vector matrix alone); mode 1, identity, a translation (2, 1, -1), a scale by 2; mode 0,
@ a 4x3 load (the identity with 0.5 in the bottom row's first place); mode 1, a push and an
@ identity. List 2: a pop (offset 1).
@
@ Record at 0x02008000 (word slots):
@   0-15   CLIPMTX_RESULT after list 1: position (the identity) times projection.
@   16-31  CLIPMTX_RESULT after list 2: the popped position (scale x translation) times projection.
@   32-40  VECMTX_RESULT after list 2: the quarter turn, not scaled.
@   62     progress (1 when done).   63  this ROM's sentinel "GXMT".
@
@ TODO(hw-unverified): the matrix and stack behaviour follows the model of docs/hw/geometry-engine.md
@ ("Model", "Unverified"); a pass says the emulator follows the note, not that hardware was
@ measured. The ROM relies on the documented POWCNT1 reset value 820Fh having bit 3 (the geometry
@ engine) set (docs/hw/display-mode-vram.md, "Unverified").
    .equ REC, 0x02008000
    .equ GXFIFO, 0x04000400
    .equ GXSTAT, 0x04000600
    .equ CLIPMTX, 0x04000640
    .equ VECMTX, 0x04000680

    @ Wait until GXSTAT bit 27 (the engine is busy) is clear.
    .macro WAIT_IDLE
1:  ldr r0, [r9]
    tst r0, #0x08000000
    bne 1b
    .endm

    @ Copy \count words from the address in \src to the record at slot \slot.
    .macro COPY src, slot, count
    ldr r1, =\src
    add r2, r8, #(\slot * 4)
    mov r3, #\count
2:  ldr r0, [r1], #4
    str r0, [r2], #4
    subs r3, r3, #1
    bne 2b
    .endm

    @ Send the words from \start up to \end to GXFIFO.
    .macro SEND start, end
    ldr r1, =\start
    ldr r2, =\end
3:  ldr r0, [r1], #4
    str r0, [r7]
    cmp r1, r2
    blo 3b
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

    SEND list1, list1_end
    WAIT_IDLE
    COPY CLIPMTX, 0, 16
    SEND list2, list2_end
    WAIT_IDLE
    COPY CLIPMTX, 16, 16
    COPY VECMTX, 32, 9

    mov r0, #1
    str r0, [r8, #(62 * 4)]
    ldr r0, =0x544D5847        @ "GXMT"
    str r0, [r8, #(63 * 4)]
spin:
    b spin
    .ltorg

    .balign 4
list1:
    .word 0x10, 2              @ MTX_MODE 2: position and vector
    .word 0x15                 @ MTX_IDENTITY
    .word 0x1A                 @ MTX_MULT_3x3: a quarter turn about Z
    .word 0x0000, 0x1000, 0x0000
    .word 0xFFFFF000, 0x0000, 0x0000
    .word 0x0000, 0x0000, 0x1000
    .word 0x1B, 0x2000, 0x2000, 0x2000   @ MTX_SCALE by 2: position only
    .word 0x10, 1              @ MTX_MODE 1: position
    .word 0x15                 @ MTX_IDENTITY
    .word 0x1C, 0x2000, 0x1000, 0xFFFFF000   @ MTX_TRANS (2, 1, -1)
    .word 0x1B, 0x2000, 0x2000, 0x2000   @ MTX_SCALE by 2
    .word 0x10, 0              @ MTX_MODE 0: projection
    .word 0x17                 @ MTX_LOAD_4x3: the identity with 0.5 in the bottom row
    .word 0x1000, 0, 0
    .word 0, 0x1000, 0
    .word 0, 0, 0x1000
    .word 0x0800, 0, 0
    .word 0x10, 1              @ MTX_MODE 1: position
    .word 0x11                 @ MTX_PUSH
    .word 0x15                 @ MTX_IDENTITY
list1_end:
list2:
    .word 0x12, 1              @ MTX_POP, offset 1
list2_end:
