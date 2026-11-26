@ SPDX-License-Identifier: Apache-2.0
@ Palette texture ROM, ARM9 side (docs/hw/textures.md, "A texel's address and colour",
@ "Blending the texel with the vertex colour"). No libnds, no crt0.
@ Loaded at 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over an opaque blue rear plane (as in gxalpha). VRAM bank A is
@ written through its LCDC window first and then mapped as texture slot 0; bank E the same as the
@ texture palette (docs/hw/vram-banks.md). DISP3DCNT is 01h: texture mapping on, nothing else.
@
@ Regions (the expected picture is in crates/runner/tests/gxtexpal.rs, worked out from the note).
@ Each is a 64x48 quad showing an 8x8 texture, one texel to 8x6 pixels, white vertex colour except R5:
@   R1  256-colour texture, index (3x + 5y) mod 7, palette base 0.
@   R2  16-colour texture, index (x + 2y) mod 16, palette base 20h (a 16-byte unit).
@   R3  4-colour texture, index (x + y) mod 4, palette base 48h (an 8-byte unit: the same register
@       value would be byte 480h in another format).
@   R4  the R1 texture with TEXIMAGE_PARAM bit 29 set: texels of index 0 are transparent.
@   R5  the R2 texture modulated by the grey vertex colour 4210h.
@   R6  the R3 texture with bit 29 set.
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
    @ R1 256-colour, PLTT_BASE 0
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
    @ R2 16-colour, PLTT_BASE 20h (byte 200h)
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x0C000008
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000020
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
    @ R3 4-colour, PLTT_BASE 48h (8-byte units: byte 240h)
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x0800000C
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000048
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
    @ R4 256-colour, colour 0 transparent
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x30000000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (16, line 96)
    .word 0x0000F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (16, line 144)
    .word 0xF800F200
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (80, line 144)
    .word 0xF800FA00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (80, line 96)
    .word 0x0000FA00
    .word 0x00000000
    @ R5 16-colour modulated by a grey vertex colour
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x0C000008
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000020
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00004210
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (96, line 96)
    .word 0x0000FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00004210
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (96, line 144)
    .word 0xF800FC00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00004210
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (160, line 144)
    .word 0xF8000400
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00004210
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (160, line 96)
    .word 0x00000400
    .word 0x00000000
    @ R6 4-colour, colour 0 transparent
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x2800000C
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000048
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (176, line 96)
    .word 0x00000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (176, line 144)
    .word 0xF8000600
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (240, line 144)
    .word 0xF8000E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (240, line 96)
    .word 0x00000E00
    .word 0x00000000
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000000   @ auto-sort, Z-buffering
commands_end:
texdata:
    .word 0x02060300
    .word 0x00040105
    .word 0x00040105
    .word 0x05020603
    .word 0x05020603
    .word 0x03000401
    .word 0x03000401
    .word 0x01050206
    .word 0x01050206
    .word 0x06030004
    .word 0x06030004
    .word 0x04010502
    .word 0x04010502
    .word 0x02060300
    .word 0x02060300
    .word 0x00040105
    .word 0x76543210
    .word 0x98765432
    .word 0xBA987654
    .word 0xDCBA9876
    .word 0xFEDCBA98
    .word 0x10FEDCBA
    .word 0x3210FEDC
    .word 0x543210FE
    .word 0x3939E4E4
    .word 0x93934E4E
    .word 0x3939E4E4
    .word 0x93934E4E
texdata_end:
paldata:
    .word 0x342B1BCA
    .word 0x6CED508C
    .word 0x25AF094E
    .word 0x5E714210
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x7F696308
    .word 0x342B1BCA
    .word 0x6CED508C
    .word 0x25AF094E
    .word 0x5E714210
    .word 0x17337AD2
    .word 0x4FF53394
    .word 0x04B76856
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x00000000
    .word 0x5E714210
    .word 0x17337AD2
paldata_end:

