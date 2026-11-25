@ SPDX-License-Identifier: Apache-2.0
@ Rasterizer depth ROM, ARM9 side (docs/hw/rasterizer.md, "Depth", and docs/hw/geometry-engine.md,
@ "Clipping"). No libnds, no crt0. Loaded at
@ 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over an opaque blue rear plane (as in gxedge). SWAP_BUFFERS
@ asks for W-buffering (bit 1), so a pixel's depth is its W. Each polygon first loads the position
@ matrix: the identity with the fourth diagonal element set to the polygon's W (1.0 = 0x1000), so a
@ vertex's W is that value and its screen position is its clip position divided by it; the ROM scales
@ the coordinates so that every vertex still lands on a whole pixel.
@
@ Regions (the expected picture is in crates/runner/tests/gxdepth.rs, worked out from the note):
@   R1  green quad of W 2.0, then red quad of W 1.0 and a larger Z: W-buffering lets the smaller W
@       cover it (Z-buffering would not).
@   R2  three quads of one W: a polygon that asks for equal depth covers the first, one that asks for
@       less does not.
@   R3  equal depth has a tolerance of 200h: a W 100h above draws, a W 300h above does not.
@   R4  quads with a right half beyond the far plane: POLYGON_ATTR bit 12 set draws the part inside,
@       clear hides the polygon.
@
@ TODO(hw-unverified): the depth mapping, the equal tolerance and the clipping follow the model of
@ docs/hw/rasterizer.md and geometry-engine.md ("Unverified"); a pass says the emulator follows the
@ note, not that hardware was measured.
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
    @ R1 A: green quad, W 2.0, drawn first
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x2000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00002000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (16, line 24, z 0)
    .word 0x1800E400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (16, line 72, z 0)
    .word 0x0800E400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (80, line 72, z 0)
    .word 0x0800F400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (80, line 24, z 0)
    .word 0x1800F400
    .word 0x00000000
    @ R1 B: red quad, W 1.0, z 0.5, drawn second: smaller W wins
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x021F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (48, line 24, z 2048)
    .word 0x0C00F600
    .word 0x00000800
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (48, line 72, z 2048)
    .word 0x0400F600
    .word 0x00000800
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (112, line 72, z 2048)
    .word 0x0400FE00
    .word 0x00000800
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (112, line 24, z 2048)
    .word 0x0C00FE00
    .word 0x00000800
    @ R2 P1: green quad
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x031F00C0
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
    .word 0x00000023   @ VTX_16 (192, line 72, z 0)
    .word 0x04000800
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (192, line 24, z 0)
    .word 0x0C000800
    .word 0x00000000
    @ R2 P2: red, depth equal
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x041F40C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (144, line 24, z 0)
    .word 0x0C000200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (144, line 72, z 0)
    .word 0x04000200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (208, line 72, z 0)
    .word 0x04000A00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (208, line 24, z 0)
    .word 0x0C000A00
    .word 0x00000000
    @ R2 P3: white, depth less
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x051F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (160, line 24, z 0)
    .word 0x0C000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (160, line 72, z 0)
    .word 0x04000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (224, line 72, z 0)
    .word 0x04000C00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (224, line 24, z 0)
    .word 0x0C000C00
    .word 0x00000000
    @ R3 Q1: green quad
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x061F00C0
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
    .word 0x00000023   @ VTX_16 (80, line 144, z 0)
    .word 0xF800FA00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000023   @ VTX_16 (80, line 96, z 0)
    .word 0x0000FA00
    .word 0x00000000
    @ R3 Q2: red, depth equal, W 100h above Q1's
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1100
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001100
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x071F40C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (16, line 96, z 0)
    .word 0x0000F120
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (16, line 144, z 0)
    .word 0xF780F120
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (80, line 144, z 0)
    .word 0xF780F9A0
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (80, line 96, z 0)
    .word 0x0000F9A0
    .word 0x00000000
    @ R3 Q4: yellow quad
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x081F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003FF
    .word 0x00000023   @ VTX_16 (96, line 96, z 0)
    .word 0x0000FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003FF
    .word 0x00000023   @ VTX_16 (96, line 144, z 0)
    .word 0xF800FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003FF
    .word 0x00000023   @ VTX_16 (160, line 144, z 0)
    .word 0xF8000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003FF
    .word 0x00000023   @ VTX_16 (160, line 96, z 0)
    .word 0x00000400
    .word 0x00000000
    @ R3 Q3: white, depth equal, W 300h above Q4's
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1300
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001300
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x091F40C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (96, line 96, z 0)
    .word 0x0000FB40
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (96, line 144, z 0)
    .word 0xF680FB40
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (160, line 144, z 0)
    .word 0xF68004C0
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000023   @ VTX_16 (160, line 96, z 0)
    .word 0x000004C0
    .word 0x00000000
    @ R4 far plane, bit 12 set: magenta quad, right half beyond the far plane
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x0A1F10C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (176, line 96, z 0)
    .word 0x00000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (176, line 144, z 0)
    .word 0xF8000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (240, line 144, z 8192)
    .word 0xF8000E00
    .word 0x00002000
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (240, line 96, z 8192)
    .word 0x00000E00
    .word 0x00002000
    @ R4 far plane, bit 12 clear: the same shape lower down
    .word 0x00000010   @ MTX_MODE
    .word 0x00000001   @ position
    .word 0x00000016   @ MTX_LOAD_4x4: identity with W = 0x1000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00001000
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x0B1F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (176, line 150, z 0)
    .word 0xF7000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (176, line 183, z 0)
    .word 0xF1800600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (240, line 183, z 8192)
    .word 0xF1800E00
    .word 0x00002000
    .word 0x00000020   @ COLOR
    .word 0x00007C1F
    .word 0x00000023   @ VTX_16 (240, line 150, z 8192)
    .word 0xF7000E00
    .word 0x00002000
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000002   @ Z-buffering off: W-buffering (bit 1)
commands_end:
