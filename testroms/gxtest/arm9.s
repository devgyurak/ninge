@ SPDX-License-Identifier: Apache-2.0
@ Geometry test-command ROM, ARM9 side (docs/hw/geometry-engine.md, "POS_TEST, VEC_TEST and
@ BOX_TEST"). No libnds, no crt0. Loaded at 0x02000000 and entered there
@ (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ The ARM9 sets a translation (0x100, 0x200, 0x300) in the position matrix, runs POS_TEST,
@ VEC_TEST and two BOX_TESTs, and records the results. The expected words are in
@ crates/runner/tests/gxtest.rs, worked out by hand from the note and not from running this ROM.
@
@ Record at 0x02008000 (word slots):
@   0-3  POS_RESULT x, y, z, w for the point (1, 2, 3).
@   4    VEC_RESULT x and y (one word read from 0x04000630) for the vector (256, -256, 0).
@   5    VEC_RESULT z (a halfword read from 0x04000634).
@   6    GXSTAT bit 1 after a BOX_TEST of a box that is inside the view volume.
@   7    GXSTAT bit 1 after a BOX_TEST of a box beyond the right plane.
@   8    GXSTAT bit 0 read right after the last BOX_TEST was sent: the test is still busy.
@ Slot 62: progress (1 when done). Slot 63: this ROM's sentinel "GXTE".
@
@ TODO(hw-unverified): the tests follow the model of docs/hw/geometry-engine.md ("Model",
@ "Unverified"); a pass says the emulator follows the note, not that hardware was measured. The ROM
@ relies on the documented POWCNT1 reset value 820Fh having bit 3 (the geometry engine) set
@ (docs/hw/display-mode-vram.md, "Unverified").
    .equ REC, 0x02008000
    .equ GXFIFO, 0x04000400
    .equ GXSTAT, 0x04000600
    .equ POS_RESULT, 0x04000620
    .equ VEC_RESULT, 0x04000630

    @ Wait until GXSTAT bit 27 (the engine is busy) is clear.
    .macro WAIT_IDLE
1:  ldr r0, [r9]
    tst r0, #0x08000000
    bne 1b
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

    SEND list_pos, list_pos_end
    WAIT_IDLE
    ldr r1, =POS_RESULT
    ldr r0, [r1]
    str r0, [r8, #(0 * 4)]
    ldr r0, [r1, #4]
    str r0, [r8, #(1 * 4)]
    ldr r0, [r1, #8]
    str r0, [r8, #(2 * 4)]
    ldr r0, [r1, #12]
    str r0, [r8, #(3 * 4)]

    SEND list_vec, list_vec_end
    WAIT_IDLE
    ldr r1, =VEC_RESULT
    ldr r0, [r1]
    str r0, [r8, #(4 * 4)]
    ldrh r0, [r1, #4]
    str r0, [r8, #(5 * 4)]

    SEND list_box_in, list_box_in_end
    ldr r0, [r9]
    and r0, r0, #1
    str r0, [r8, #(8 * 4)]     @ right after the last word: BOX_TEST is still busy
    WAIT_IDLE
    ldr r0, [r9]
    and r0, r0, #2
    str r0, [r8, #(6 * 4)]

    SEND list_box_out, list_box_out_end
    WAIT_IDLE
    ldr r0, [r9]
    and r0, r0, #2
    str r0, [r8, #(7 * 4)]

    mov r0, #1
    str r0, [r8, #(62 * 4)]
    ldr r0, =0x45545847        @ "GXTE"
    str r0, [r8, #(63 * 4)]
spin:
    b spin
    .ltorg

    .balign 4
list_pos:
    .word 0x10, 1                  @ MTX_MODE 1: position
    .word 0x1C, 0x100, 0x200, 0x300   @ MTX_TRANS
    .word 0x71, 0x00020001, 0x00000003   @ POS_TEST (1, 2, 3)
list_pos_end:
list_vec:
    .word 0x10, 2                  @ MTX_MODE 2
    .word 0x72, 0x000C0100         @ VEC_TEST (256, -256, 0)
list_vec_end:
list_box_in:
    .word 0x70, 0xFF00FF00, 0x0200FF00, 0x02000200   @ BOX_TEST: corner (-0x100) size 0x200
list_box_in_end:
list_box_out:
    .word 0x70, 0x00003000, 0x01000000, 0x01000100   @ BOX_TEST: corner (0x3000, 0, 0) size 0x100
list_box_out_end:
