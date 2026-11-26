@ SPDX-License-Identifier: Apache-2.0
@ Alpha texture ROM, ARM9 side (docs/hw/textures.md, "A texel's address and colour",
@ "Texture alpha, translucency and the buffers"). No libnds, no crt0.
@ Loaded at 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over an opaque blue rear plane (as in gxalpha). VRAM bank A is
@ written through its LCDC window first and then mapped as texture slot 0; bank E the same as the
@ texture palette (docs/hw/vram-banks.md). DISP3DCNT is 09h:
@ texture mapping (bit 0) and blending (bit 3) on.
@
@ Regions (the expected picture is in crates/runner/tests/gxtexalpha.rs, worked out from the note).
@ Each is a 64x48 quad showing an 8x8 texture, one texel to 8x6 pixels. The texture is A3I5 with
@ alpha y and index (4x + y) mod 32, A5I3 with alpha 4y and index x, or direct with colour
@ x + 8y + 3 and an alpha bit that is clear where (x + y) mod 3 is 0:
@   R1  A3I5, modulation, polygon alpha 31: alpha 0 hides a row, the others blend with the rear plane.
@   R2  A5I3, modulation, polygon alpha 31, PLTT_BASE 4.
@   R3  direct colour, modulation: the alpha bit shows or hides each texel.
@   R4  A5I3 in decal mode with a red vertex colour: the texel alpha is only the mix ratio and the
@       polygon stays opaque.
@   R5  A3I5, modulation, polygon alpha 15: the two alphas multiply.
@   R6  direct colour in decal mode with a green vertex colour: a clear alpha bit shows the vertex
@       colour.
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
    mov r1, #0x09
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
    @ R1 A3I5 modulated, polygon alpha 31
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x04000000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (16, line 24)
    .word 0x0C00F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (16, line 72)
    .word 0x0400F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (80, line 72)
    .word 0x0400FA00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (80, line 24)
    .word 0x0C00FA00
    .word 0x00000000
    @ R2 A5I3 modulated, polygon alpha 31 (PLTT_BASE 4: byte 40h)
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x18000008
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000004
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (96, line 24)
    .word 0x0C00FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (96, line 72)
    .word 0x0400FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (160, line 72)
    .word 0x04000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (160, line 24)
    .word 0x0C000400
    .word 0x00000000
    @ R3 direct colour modulated, polygon alpha 31
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x1C000010
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (176, line 24)
    .word 0x0C000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (176, line 72)
    .word 0x04000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (240, line 72)
    .word 0x04000E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (240, line 24)
    .word 0x0C000E00
    .word 0x00000000
    @ R4 A5I3 decal, red vertex colour
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00D0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x18000008
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000004
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (16, line 96)
    .word 0x0000F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (16, line 144)
    .word 0xF800F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (80, line 144)
    .word 0xF800FA00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x0000001F
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (80, line 96)
    .word 0x0000FA00
    .word 0x00000000
    @ R5 A3I5 modulated, polygon alpha 15
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x010F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x04000000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (96, line 96)
    .word 0x0000FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (96, line 144)
    .word 0xF800FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (160, line 144)
    .word 0xF8000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (160, line 96)
    .word 0x00000400
    .word 0x00000000
    @ R6 direct decal, green vertex colour
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00D0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x1C000010
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (176, line 96)
    .word 0x00000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (176, line 144)
    .word 0xF8000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (240, line 144)
    .word 0xF8000E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x000003E0
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (240, line 96)
    .word 0x00000E00
    .word 0x00000000
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000000   @ auto-sort, Z-buffering
commands_end:
texdata:
    .word 0x0C080400
    .word 0x1C181410
    .word 0x2D292521
    .word 0x3D393531
    .word 0x4E4A4642
    .word 0x5E5A5652
    .word 0x6F6B6763
    .word 0x7F7B7773
    .word 0x908C8884
    .word 0x809C9894
    .word 0xB1ADA9A5
    .word 0xA1BDB9B5
    .word 0xD2CECAC6
    .word 0xC2DEDAD6
    .word 0xF3EFEBE7
    .word 0xE3FFFBF7
    .word 0x03020100
    .word 0x07060504
    .word 0x23222120
    .word 0x27262524
    .word 0x43424140
    .word 0x47464544
    .word 0x63626160
    .word 0x67666564
    .word 0x83828180
    .word 0x87868584
    .word 0xA3A2A1A0
    .word 0xA7A6A5A4
    .word 0xC3C2C1C0
    .word 0xC7C6C5C4
    .word 0xE3E2E1E0
    .word 0xE7E6E5E4
    .word 0xF1845523
    .word 0x2A468DE5
    .word 0xE308C6A7
    .word 0x9BCA7F69
    .word 0xD08CB42B
    .word 0x894E6CED
    .word 0x4210A5AF
    .word 0xFAD2DE71
    .word 0x33949733
    .word 0xE856CFF5
    .word 0xA11804B7
    .word 0x59DABD79
    .word 0x929C763B
    .word 0x4B5EAEFD
    .word 0x8000E7BF
    .word 0xB8C21C61
    .word 0xF184D523
    .word 0xAA460DE5
    .word 0x6308C6A7
    .word 0x9BCAFF69
    .word 0x508CB42B
    .word 0x894EECED
    .word 0xC21025AF
    .word 0x7AD2DE71
    .word 0xB3941733
    .word 0x6856CFF5
    .word 0xA11884B7
    .word 0xD9DA3D79
    .word 0x929CF63B
    .word 0xCB5E2EFD
    .word 0x0000E7BF
    .word 0xB8C29C61
texdata_end:
paldata:
    .word 0x4FF53394
    .word 0x04B76856
    .word 0x3D792118
    .word 0x763B59DA
    .word 0x2EFD129C
    .word 0x67BF4B5E
    .word 0x1C610000
    .word 0x552338C2
    .word 0x0DE57184
    .word 0x46A72A46
    .word 0x7F696308
    .word 0x342B1BCA
    .word 0x6CED508C
    .word 0x25AF094E
    .word 0x5E714210
    .word 0x17337AD2
    .word 0x2EFD129C
    .word 0x67BF4B5E
    .word 0x1C610000
    .word 0x552338C2
paldata_end:

