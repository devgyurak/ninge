@ SPDX-License-Identifier: Apache-2.0
@ Rasterizer fill ROM, ARM9 side (docs/hw/rasterizer.md, "From clip coordinates to the screen", "Which
@ pixels a polygon covers", "Colour along a polygon", "Depth"). No libnds, no crt0. Loaded at
@ 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 (DISPCNT 0x00010108: display mode 1, BG mode 0, BG0 on and 3D).
@ The rear plane is opaque blue at the far depth. With the matrices at their reset (the identity) a
@ vertex's clip coordinates are its screen position: pixel = (x + 1.0) x 128 across, and line =
@ 192 - (y + 1.0) x 96 down, in the full-screen viewport. The ROM sends seven polygons and
@ SWAP_BUFFERS; the swap waits for VBlank and the picture is drawn at line 214, so it shows from the
@ next frame. Every polygon has both faces on, no lights, no texture; DISP3DCNT is 0 (no edge
@ marking, anti-aliasing, fog or blending).
@
@ Polygons (the expected picture is in crates/runner/tests/gxedge.rs, worked out from the note, never
@ from running the ROM):
@   A  red quad, pixels 16-47 across, lines 24-47.
@   B  green right triangle (100,24) (100,72) (148,72): a slanted right edge.
@   C  quad 160-223 across, lines 24-47: red channel 0 at the left edge, 31 at the right.
@   D1 near green quad, then D2 far red quad overlapping it, drawn after it: depth decides.
@   E  a triangle of three equal vertices at (240,171): one dot.
@   F  wire frame (alpha 0) green quad (150,93)-(230,141).
@
@ TODO(hw-unverified): the coverage rule, the interpolation and the depth mapping follow the model of
@ docs/hw/rasterizer.md ("Unverified"); a pass says the emulator follows the note, not that hardware
@ was measured. The ROM relies on the documented POWCNT1 bits 2 and 3 for the 3D engine.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISP3DCNT, 0x04000060
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
    mov r1, #0
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
    @ A: red quad
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
    .word 0x00000023   @ VTX_16 (16, line 48, z 0)
    .word 0x0800F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (48, line 48, z 0)
    .word 0x0800F600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (48, line 24, z 0)
    .word 0x0C00F600
    .word 0x00000000
    @ B: green triangle with a slanted right edge
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x021F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (100, line 24, z 0)
    .word 0x0C00FC80
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (100, line 72, z 0)
    .word 0x0400FC80
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (148, line 72, z 0)
    .word 0x04000280
    .word 0x00000000
    @ C: red gradient quad, black at the left
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x031F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (160, line 24, z 0)
    .word 0x0C000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (160, line 48, z 0)
    .word 0x08000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (224, line 48, z 0)
    .word 0x08000C00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (224, line 24, z 0)
    .word 0x0C000C00
    .word 0x00000000
    @ D1: near green quad (z -0.5), drawn first
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x041F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (20, line 120, z -2048)
    .word 0xFC00F280
    .word 0x0000F800
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (20, line 168, z -2048)
    .word 0xF400F280
    .word 0x0000F800
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (84, line 168, z -2048)
    .word 0xF400FA80
    .word 0x0000F800
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (84, line 120, z -2048)
    .word 0xFC00FA80
    .word 0x0000F800
    @ D2: far red quad (z +0.5), drawn second
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x051F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (52, line 135, z 2048)
    .word 0xF980F680
    .word 0x00000800
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (52, line 183, z 2048)
    .word 0xF180F680
    .word 0x00000800
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (116, line 183, z 2048)
    .word 0xF180FE80
    .word 0x00000800
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (116, line 135, z 2048)
    .word 0xF980FE80
    .word 0x00000800
    @ E: a triangle of one point, white
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x061F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (240, line 171, z 0)
    .word 0xF3800E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (240, line 171, z 0)
    .word 0xF3800E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (240, line 171, z 0)
    .word 0xF3800E00
    .word 0x00000000
    @ F: wire frame quad, alpha 0, green
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x070000C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (150, line 93, z 0)
    .word 0x008002C0
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (150, line 141, z 0)
    .word 0xF88002C0
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (230, line 141, z 0)
    .word 0xF8800CC0
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (230, line 93, z 0)
    .word 0x00800CC0
    .word 0x00000000
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000000   @ manual sort off, Z-buffering
commands_end:
