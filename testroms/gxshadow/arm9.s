@ SPDX-License-Identifier: Apache-2.0
@ Rasterizer shadow ROM, ARM9 side (docs/hw/rasterizer.md, "Shadow polygons"). No libnds, no crt0. Loaded at
@ 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over an opaque blue rear plane (as in gxedge). DISP3DCNT is 08h
@ (blending); SWAP_BUFFERS asks for manual sort, which shadow volumes need. Each region is an opaque
@ red floor with a mask pass (polygon mode 3, ID 0, back faces) and a render pass (mode 3, ID not 0,
@ front faces, alpha 15, black).
@
@ Regions (the expected picture is in crates/runner/tests/gxshadow.rs, worked out from the note):
@   S1  the mask is behind the floor, the render pass in front: the floor is shadowed.
@   S2  both in front of the floor: the stencil is set where the render pass looks, and neither its
@       front face nor its back face draws a shadow.
@   S3  the render pass has the floor's own ID: no shadow.
@   S4  a volume in front of the floor draws nothing; the next volume's mask pass clears its flags, and
@       its render pass then shadows.
@
@ TODO(hw-unverified): the stencil's lifetime and the depth rules of both passes follow the model of
@ docs/hw/rasterizer.md ("Unverified"); a pass says the emulator follows the note, not that hardware
@ was measured.
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
    mov r1, #0x08
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
    @ S1 floor, ID 1
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
    .word 0x00000023   @ VTX_16 (112, line 72, z 0)
    .word 0x0400FE00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (112, line 24, z 0)
    .word 0x0C00FE00
    .word 0x00000000
    @ S1 mask pass (ID 0, back faces) behind the floor
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x000F0070
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 36, z 1000)
    .word 0x0A00F400
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 36, z 1000)
    .word 0x0A00FA00
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 60, z 1000)
    .word 0x0600FA00
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 60, z 1000)
    .word 0x0600F400
    .word 0x000003E8
    @ S1 render pass (ID 2, front faces) in front: shadowed
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x020F00B0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 36, z -1000)
    .word 0x0A00F400
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 60, z -1000)
    .word 0x0600F400
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 60, z -1000)
    .word 0x0600FA00
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 36, z -1000)
    .word 0x0A00FA00
    .word 0x0000FC18
    @ S2 floor, ID 3
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x031F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (128, line 24, z 0)
    .word 0x0C000000
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (128, line 72, z 0)
    .word 0x04000000
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (224, line 72, z 0)
    .word 0x04000C00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (224, line 24, z 0)
    .word 0x0C000C00
    .word 0x00000000
    @ S2 mask pass in front of the floor
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x000F0070
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 36, z -1000)
    .word 0x0A000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 36, z -1000)
    .word 0x0A000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 60, z -1000)
    .word 0x06000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 60, z -1000)
    .word 0x06000200
    .word 0x0000FC18
    @ S2 render pass, front face, in front: the stencil is set, so no shadow
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x040F00B0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 36, z -1000)
    .word 0x0A000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 60, z -1000)
    .word 0x06000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 60, z -1000)
    .word 0x06000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 36, z -1000)
    .word 0x0A000800
    .word 0x0000FC18
    @ S2 render pass, back face, in front: the same flag, no shadow either
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x040F0070
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 36, z -1000)
    .word 0x0A000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 36, z -1000)
    .word 0x0A000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 60, z -1000)
    .word 0x06000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 60, z -1000)
    .word 0x06000200
    .word 0x0000FC18
    @ S3 floor, ID 5
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x051F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (16, line 96, z 0)
    .word 0x0000F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (16, line 144, z 0)
    .word 0xF800F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (112, line 144, z 0)
    .word 0xF800FE00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (112, line 96, z 0)
    .word 0x0000FE00
    .word 0x00000000
    @ S3 mask pass behind the floor
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x000F0070
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 108, z 1000)
    .word 0xFE00F400
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 108, z 1000)
    .word 0xFE00FA00
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 132, z 1000)
    .word 0xFA00FA00
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 132, z 1000)
    .word 0xFA00F400
    .word 0x000003E8
    @ S3 render pass of the floor's own ID: no shadow
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x050F00B0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 108, z -1000)
    .word 0xFE00F400
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (32, line 132, z -1000)
    .word 0xFA00F400
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 132, z -1000)
    .word 0xFA00FA00
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (80, line 108, z -1000)
    .word 0xFE00FA00
    .word 0x0000FC18
    @ S4 floor, ID 6
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x061F00C0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (128, line 96, z 0)
    .word 0x00000000
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (128, line 144, z 0)
    .word 0xF8000000
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (224, line 144, z 0)
    .word 0xF8000C00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000023   @ VTX_16 (224, line 96, z 0)
    .word 0x00000C00
    .word 0x00000000
    @ S4 mask pass in front of the floor
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x000F0070
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 108, z -1000)
    .word 0xFE000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 108, z -1000)
    .word 0xFE000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 132, z -1000)
    .word 0xFA000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 132, z -1000)
    .word 0xFA000200
    .word 0x0000FC18
    @ S4 first render pass (ID 7): the stencil is set, draws nothing
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x070F00B0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 108, z -1000)
    .word 0xFE000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 132, z -1000)
    .word 0xFA000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 132, z -1000)
    .word 0xFA000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 108, z -1000)
    .word 0xFE000800
    .word 0x0000FC18
    @ S4 second mask pass, behind the floor: a new volume, clears the old flags
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x000F0070
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 108, z 1000)
    .word 0xFE000200
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 108, z 1000)
    .word 0xFE000800
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 132, z 1000)
    .word 0xFA000800
    .word 0x000003E8
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 132, z 1000)
    .word 0xFA000200
    .word 0x000003E8
    @ S4 second render pass (ID 8): the stencil is clear, shadowed
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x080F00B0
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 108, z -1000)
    .word 0xFE000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (144, line 132, z -1000)
    .word 0xFA000200
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 132, z -1000)
    .word 0xFA000800
    .word 0x0000FC18
    .word 0x00000020   @ COLOR
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (192, line 108, z -1000)
    .word 0xFE000800
    .word 0x0000FC18
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000001   @ manual sort, Z-buffering
commands_end:
