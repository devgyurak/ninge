@ SPDX-License-Identifier: Apache-2.0
@ Texture wrap ROM, ARM9 side (docs/hw/textures.md, "From S and T to a texel"). No libnds, no
@ crt0.
@ Loaded at 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over an opaque blue rear plane (as in gxalpha). VRAM bank A is
@ written through its LCDC window first and then mapped as texture slot 0; bank E the same as the
@ texture palette (docs/hw/vram-banks.md). DISP3DCNT is 01h: texture mapping on, nothing else.
@
@ Regions (the expected picture is in crates/runner/tests/gxtexwrap.rs, worked out from the note).
@ Each is a 64x48 quad showing the same 8x8 256-colour texture (index 8y + x, palette base 0) with S
@ running from -4 to 12 texels and T from -3 to 9, so every edge case of the wrap shows:
@   R1  clamp in S and T.
@   R2  repeat in S and T.
@   R3  repeat and flip in S and T.
@   R4  repeat and flip in S, clamp in T.
@   R5  flip bits without repeat: both axes clamp.
@   R6  repeat in T only: S clamps.
@
@ TODO(hw-unverified): the texel sampling and the modulation follow the model of docs/hw/textures.md
@ ("Unverified"); a pass says the emulator follows the note, not that hardware was measured.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISP3DCNT, 0x04000060
    .equ CLEAR_COLOR, 0x04000350
    .equ CLEAR_DEPTH, 0x04000354
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_E, 0x04000244
    .equ LCDC_A, 0x06800000
    .equ LCDC_E, 0x06880000
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

    @ Engine A on the upper screen, the 3D engine on, texture mapping on.
    ldr r0, =POWCNT1
    ldr r1, =0x800F
    strh r1, [r0]
    ldr r0, =DISP3DCNT
    mov r1, #0x01
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

    @ Banks A and E as LCDC memory (MST 0): write the texture and the palette.
    mov r2, #0x80
    ldr r0, =VRAMCNT_A
    strb r2, [r0]
    ldr r0, =VRAMCNT_E
    strb r2, [r0]
    ldr r0, =LCDC_A
    ldr r1, =texdata
    ldr r2, =texdata_end
copy_tex:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_tex
    ldr r0, =LCDC_E
    ldr r1, =paldata
    ldr r2, =paldata_end
copy_pal:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_pal
    @ Bank A as texture slot 0 (MST 3, OFS 0), bank E as the texture palette (MST 3).
    mov r2, #0x83
    ldr r0, =VRAMCNT_A
    strb r2, [r0]
    ldr r0, =VRAMCNT_E
    strb r2, [r0]

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
    @ R1 clamp in S and T
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x10000000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, -3 texels)
    .word 0xFFD0FFC0
    .word 0x00000023   @ VTX_16 (16, line 24)
    .word 0x0C00F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, 9 texels)
    .word 0x0090FFC0
    .word 0x00000023   @ VTX_16 (16, line 72)
    .word 0x0400F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, 9 texels)
    .word 0x009000C0
    .word 0x00000023   @ VTX_16 (80, line 72)
    .word 0x0400FA00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, -3 texels)
    .word 0xFFD000C0
    .word 0x00000023   @ VTX_16 (80, line 24)
    .word 0x0C00FA00
    .word 0x00000000
    @ R2 repeat in S and T
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x10030000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, -3 texels)
    .word 0xFFD0FFC0
    .word 0x00000023   @ VTX_16 (96, line 24)
    .word 0x0C00FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, 9 texels)
    .word 0x0090FFC0
    .word 0x00000023   @ VTX_16 (96, line 72)
    .word 0x0400FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, 9 texels)
    .word 0x009000C0
    .word 0x00000023   @ VTX_16 (160, line 72)
    .word 0x04000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, -3 texels)
    .word 0xFFD000C0
    .word 0x00000023   @ VTX_16 (160, line 24)
    .word 0x0C000400
    .word 0x00000000
    @ R3 repeat and flip in S and T
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x100F0000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, -3 texels)
    .word 0xFFD0FFC0
    .word 0x00000023   @ VTX_16 (176, line 24)
    .word 0x0C000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, 9 texels)
    .word 0x0090FFC0
    .word 0x00000023   @ VTX_16 (176, line 72)
    .word 0x04000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, 9 texels)
    .word 0x009000C0
    .word 0x00000023   @ VTX_16 (240, line 72)
    .word 0x04000E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, -3 texels)
    .word 0xFFD000C0
    .word 0x00000023   @ VTX_16 (240, line 24)
    .word 0x0C000E00
    .word 0x00000000
    @ R4 repeat and flip in S, clamp in T
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x10050000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, -3 texels)
    .word 0xFFD0FFC0
    .word 0x00000023   @ VTX_16 (16, line 96)
    .word 0x0000F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, 9 texels)
    .word 0x0090FFC0
    .word 0x00000023   @ VTX_16 (16, line 144)
    .word 0xF800F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, 9 texels)
    .word 0x009000C0
    .word 0x00000023   @ VTX_16 (80, line 144)
    .word 0xF800FA00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, -3 texels)
    .word 0xFFD000C0
    .word 0x00000023   @ VTX_16 (80, line 96)
    .word 0x0000FA00
    .word 0x00000000
    @ R5 flip without repeat (clamps)
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x100C0000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, -3 texels)
    .word 0xFFD0FFC0
    .word 0x00000023   @ VTX_16 (96, line 96)
    .word 0x0000FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, 9 texels)
    .word 0x0090FFC0
    .word 0x00000023   @ VTX_16 (96, line 144)
    .word 0xF800FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, 9 texels)
    .word 0x009000C0
    .word 0x00000023   @ VTX_16 (160, line 144)
    .word 0xF8000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, -3 texels)
    .word 0xFFD000C0
    .word 0x00000023   @ VTX_16 (160, line 96)
    .word 0x00000400
    .word 0x00000000
    @ R6 repeat in T only, clamp in S
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x10020000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, -3 texels)
    .word 0xFFD0FFC0
    .word 0x00000023   @ VTX_16 (176, line 96)
    .word 0x00000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (-4, 9 texels)
    .word 0x0090FFC0
    .word 0x00000023   @ VTX_16 (176, line 144)
    .word 0xF8000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, 9 texels)
    .word 0x009000C0
    .word 0x00000023   @ VTX_16 (240, line 144)
    .word 0xF8000E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (12, -3 texels)
    .word 0xFFD000C0
    .word 0x00000023   @ VTX_16 (240, line 96)
    .word 0x00000E00
    .word 0x00000000
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000000   @ auto-sort, Z-buffering
commands_end:
texdata:
    .word 0x03020100
    .word 0x07060504
    .word 0x0B0A0908
    .word 0x0F0E0D0C
    .word 0x13121110
    .word 0x17161514
    .word 0x1B1A1918
    .word 0x1F1E1D1C
    .word 0x23222120
    .word 0x27262524
    .word 0x2B2A2928
    .word 0x2F2E2D2C
    .word 0x33323130
    .word 0x37363534
    .word 0x3B3A3938
    .word 0x3F3E3D3C
texdata_end:
paldata:
    .word 0x2A460DE5
    .word 0x630846A7
    .word 0x1BCA7F69
    .word 0x508C342B
    .word 0x094E6CED
    .word 0x421025AF
    .word 0x7AD25E71
    .word 0x33941733
    .word 0x68564FF5
    .word 0x211804B7
    .word 0x59DA3D79
    .word 0x129C763B
    .word 0x4B5E2EFD
    .word 0x000067BF
    .word 0x38C21C61
    .word 0x71845523
    .word 0x2A460DE5
    .word 0x630846A7
    .word 0x1BCA7F69
    .word 0x508C342B
    .word 0x094E6CED
    .word 0x421025AF
    .word 0x7AD25E71
    .word 0x33941733
    .word 0x68564FF5
    .word 0x211804B7
    .word 0x59DA3D79
    .word 0x129C763B
    .word 0x4B5E2EFD
    .word 0x000067BF
    .word 0x38C21C61
    .word 0x71845523
paldata_end:

