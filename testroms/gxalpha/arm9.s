@ SPDX-License-Identifier: Apache-2.0
@ Rasterizer translucency ROM, ARM9 side (docs/hw/rasterizer.md, "Translucency, draw order and the
@ rear plane"). No libnds, no crt0. Loaded at
@ 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over an opaque blue rear plane (as in gxedge). DISP3DCNT is
@ 0Ch: the alpha test (bit 2) with ALPHA_TEST_REF 8 and blending (bit 3). Auto-sort, Z-buffering.
@
@ Regions (the expected picture is in crates/runner/tests/gxalpha.rs, worked out from the note):
@   R1  a translucent green quad listed before an opaque red one: the opaque one draws first and the
@       translucent one blends over it and over the rear plane.
@   R2  alpha 8 is not above the reference and is hidden; alpha 9 draws.
@   R3  two translucent quads of one polygon ID: the second does not blend over the first.
@   R4  a translucent quad that updates the depth hides a farther translucent one behind it.
@
@ TODO(hw-unverified): the blend, the sort key and the same-ID rule follow the model of
@ docs/hw/rasterizer.md ("Unverified"); a pass says the emulator follows the note, not that hardware
@ was measured.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISP3DCNT, 0x04000060
    .equ ALPHA_TEST_REF, 0x04000340
    .equ CLEAR_COLOR, 0x04000350
    .equ CLEAR_DEPTH, 0x04000354
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

    @ Engine A on the upper screen, the 3D engine on (bits 2 and 3).
    ldr r0, =POWCNT1
    ldr r1, =0x800F
    strh r1, [r0]
    ldr r0, =DISP3DCNT
    mov r1, #0x0C
    strh r1, [r0]
    ldr r0, =ALPHA_TEST_REF
    mov r1, #8
    strh r1, [r0]
    @ The rear plane: opaque blue, polygon ID 0, at the far depth.
    ldr r0, =CLEAR_COLOR
    ldr r1, =0x001F7C00
    str r1, [r0]
    ldr r0, =CLEAR_DEPTH
    ldr r1, =0x7FFF
    strh r1, [r0]
    ldr r0, =DISPCNT_A
    ldr r1, =0x00010108
    str r1, [r0]

    @ The commands, one word at a time.
    ldr r7, =GXFIFO
    ldr r1, =commands
    ldr r2, =commands_end
send:
    ldr r0, [r1], #4
    str r0, [r7]
    cmp r1, r2
    bne send
spin:
    b spin
    .ltorg
commands:
    .word 0x00000060   @ VIEWPORT
    .word 0xBFFF0000   @ X1 0, Y1 0, X2 255, Y2 191
    @ R1 translucent green (alpha 15), listed first and nearer than the red one
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x020F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (48, line 24, z -100)
    .word 0x0C00F600
    .word 0x0000FF9C
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (48, line 72, z -100)
    .word 0x0400F600
    .word 0x0000FF9C
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (112, line 72, z -100)
    .word 0x0400FE00
    .word 0x0000FF9C
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (112, line 24, z -100)
    .word 0x0C00FE00
    .word 0x0000FF9C
    @ R1 opaque red, listed second but drawn first
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (16, line 24, z 0)
    .word 0x0C00F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (16, line 72, z 0)
    .word 0x0400F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (80, line 72, z 0)
    .word 0x0400FA00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (80, line 24, z 0)
    .word 0x0C00FA00
    .word 0x00000000
    @ R2 alpha 8: not above ALPHA_TEST_REF 8, hidden
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x030800C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (128, line 24, z 0)
    .word 0x0C000000
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (128, line 72, z 0)
    .word 0x04000000
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (160, line 72, z 0)
    .word 0x04000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (160, line 24, z 0)
    .word 0x0C000400
    .word 0x00000000
    @ R2 alpha 9: above the reference, drawn
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x040900C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (176, line 24, z 0)
    .word 0x0C000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (176, line 72, z 0)
    .word 0x04000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (208, line 72, z 0)
    .word 0x04000A00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (208, line 24, z 0)
    .word 0x0C000A00
    .word 0x00000000
    @ R3 T1 translucent green, ID 5
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x050F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (16, line 96, z 0)
    .word 0x0000F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (16, line 144, z 0)
    .word 0xF800F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (64, line 144, z 0)
    .word 0xF800F800
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (64, line 96, z 0)
    .word 0x0000F800
    .word 0x00000000
    @ R3 T2 translucent red, same ID 5: no blend over T1
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x050F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (40, line 96, z 0)
    .word 0x0000F500
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (40, line 144, z 0)
    .word 0xF800F500
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (88, line 144, z 0)
    .word 0xF800FB00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (88, line 96, z 0)
    .word 0x0000FB00
    .word 0x00000000
    @ R4 T3 translucent green, updates depth, near
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x060F08C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (120, line 96, z -2000)
    .word 0x0000FF00
    .word 0x0000F830
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (120, line 144, z -2000)
    .word 0xF800FF00
    .word 0x0000F830
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (168, line 144, z -2000)
    .word 0xF8000500
    .word 0x0000F830
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (168, line 96, z -2000)
    .word 0x00000500
    .word 0x0000F830
    @ R4 T4 translucent red, farther: fails against T3's depth
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x070F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (144, line 96, z 2000)
    .word 0x00000200
    .word 0x000007D0
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (144, line 144, z 2000)
    .word 0xF8000200
    .word 0x000007D0
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (192, line 144, z 2000)
    .word 0xF8000800
    .word 0x000007D0
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (192, line 96, z 2000)
    .word 0x00000800
    .word 0x000007D0
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000000   @ auto-sort, Z-buffering
commands_end:
