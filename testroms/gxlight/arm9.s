@ SPDX-License-Identifier: Apache-2.0
@ Geometry lighting ROM, ARM9 side (docs/hw/geometry-engine.md, "Lighting"). No libnds, no crt0.
@ Loaded at 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ The ARM9 sets one white light (direction (0, 0, -511)), a material (diffuse white, ambient red 4,
@ emission blue 2) and sends two triangles, each after its own NORMAL: one facing the light and one
@ turned away. The vertex colours end in Vertex RAM; the runner test reads them through the
@ inspector (crates/runner/tests/gxlight.rs), with expected values worked out by hand from the note
@ and not from running this ROM. The ROM leaves 1 at 0x02008000 and "GXLI" at 0x0200807C when done.
@
@ TODO(hw-unverified): the lighting equation is GBATEK's, with the fixed-point widths of the model of
@ docs/hw/geometry-engine.md ("Model", "Unverified"); a pass says the emulator follows the note, not
@ that hardware was measured. The ROM relies on the documented POWCNT1 reset value 820Fh having bit 3 (the geometry engine) set
@ (docs/hw/display-mode-vram.md, "Unverified").
    .equ REC, 0x02008000
    .equ GXFIFO, 0x04000400

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
    ldr r1, =list
    ldr r2, =list_end
send:
    ldr r0, [r1], #4
    str r0, [r7]
    cmp r1, r2
    blo send

    mov r0, #1
    str r0, [r8]
    ldr r0, =0x494C5847        @ "GXLI"
    str r0, [r8, #0x7C]
spin:
    b spin
    .ltorg

    .balign 4
list:
    .word 0x29, 1                  @ POLYGON_ATTR: light 0 enabled
    .word 0x30, 0x00047FFF         @ DIF_AMB: diffuse white, ambient red 4
    .word 0x31, 0x08000000         @ SPE_EMI: no specular, emission blue 2
    .word 0x33, 0x00007FFF         @ LIGHT_COLOR: white, light 0
    .word 0x32, 0x20100000         @ LIGHT_VECTOR: (0, 0, -511), light 0
    .word 0x40, 0                  @ BEGIN_VTXS: separate triangles
    .word 0x21, 0x1FF00000         @ NORMAL (0, 0, 511): facing the light
    .word 0x23, 0x00000000, 0      @ VTX_16 (0, 0, 0)
    .word 0x23, 0x00000800, 0      @ VTX_16 (0x800, 0, 0)
    .word 0x23, 0x08000000, 0      @ VTX_16 (0, 0x800, 0)
    .word 0x21, 0x20100000         @ NORMAL (0, 0, -511): turned away
    .word 0x23, 0x00000000, 0
    .word 0x23, 0x00000800, 0
    .word 0x23, 0x08000000, 0
list_end:
