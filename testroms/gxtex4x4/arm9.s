@ SPDX-License-Identifier: Apache-2.0
@ 4x4 compressed texture ROM, ARM9 side (docs/hw/textures.md, "4x4 compressed texels"). No
@ libnds, no crt0.
@ Loaded at 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over an opaque blue rear plane (as in gxalpha). VRAM bank A is
@ written through its LCDC window first and then mapped as texture slot 0; banks B and C as slots 1 and 2
@ and bank E as the texture palette (docs/hw/vram-banks.md). DISP3DCNT is 01h:
@ texture mapping on, nothing else.
@
@ Regions (the expected picture is in crates/runner/tests/gxtex4x4.rs, worked out from the note).
@ Each is a 64x48 quad of an 8x8 compressed texture of 4 blocks (16x8 in R3), every block row 0E4h:
@ texels 0, 1, 2, 3 from left to right. Block palette halfwords sit in bank B (slot 1):
@   R1  T1, four blocks of modes 0, 1, 2, 3 and palette offset 0, PLTT_BASE 0: mode 0 and 1 hide
@       texel 3, mode 1 and 3 mix colours 0 and 1.
@   R2  T1 with PLTT_BASE 20h: the palette moves with the base.
@   R3  T2, 16x8: 8 blocks of texel 0, mode 2, block b at palette offset 16b bytes: the block index
@       is row-major with 4 blocks to a row.
@   R4  T3 in slot 2: its palette halfwords are at 10000h + offset / 2 in slot 1; four blocks of
@       mode 2 at palette offsets 0, 16, 32, 48 bytes.
@   R5  T4 in slot 1: no palette halfword is defined for it, the model reads 0 (mode 0, offset 0).
@   R6  T1 with TEXIMAGE_PARAM bit 29 set: the same picture as R1.
@
@ TODO(hw-unverified): the texel sampling and the modulation follow the model of docs/hw/textures.md
@ ("Unverified"); a pass says the emulator follows the note, not that hardware was measured.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISP3DCNT, 0x04000060
    .equ CLEAR_COLOR, 0x04000350
    .equ CLEAR_DEPTH, 0x04000354
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_B, 0x04000241
    .equ VRAMCNT_C, 0x04000242
    .equ VRAMCNT_E, 0x04000244
    .equ LCDC_A, 0x06800000
    .equ LCDC_B, 0x06820000
    .equ LCDC_C, 0x06840000
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

    @ The banks as LCDC memory (MST 0) first: write the texture data and the palette.
    mov r2, #0x80
    ldr r0, =VRAMCNT_A
    strb r2, [r0]
    ldr r0, =VRAMCNT_B
    strb r2, [r0]
    ldr r0, =VRAMCNT_C
    strb r2, [r0]
    ldr r0, =VRAMCNT_E
    strb r2, [r0]
    ldr r0, =LCDC_A
    ldr r1, =texdata
    ldr r2, =texdata_end
copy_texdata:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_texdata
    ldr r0, =LCDC_B
    ldr r1, =infoT1
    ldr r2, =infoT1_end
copy_infoT1:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_infoT1
    ldr r0, =LCDC_B+0x20
    ldr r1, =infoT2
    ldr r2, =infoT2_end
copy_infoT2:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_infoT2
    ldr r0, =LCDC_B+0x10000
    ldr r1, =infoT3
    ldr r2, =infoT3_end
copy_infoT3:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_infoT3
    ldr r0, =LCDC_B+0x4000
    ldr r1, =t4data
    ldr r2, =t4data_end
copy_t4data:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_t4data
    ldr r0, =LCDC_C
    ldr r1, =texC
    ldr r2, =texC_end
copy_texC:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_texC
    ldr r0, =LCDC_E
    ldr r1, =paldata
    ldr r2, =paldata_end
copy_paldata:
    ldr r3, [r1], #4
    str r3, [r0], #4
    cmp r1, r2
    bne copy_paldata
    @ Then each bank in its texture mapping (MST 3).
    mov r2, #0x83
    ldr r0, =VRAMCNT_A
    strb r2, [r0]
    mov r2, #0x8B
    ldr r0, =VRAMCNT_B
    strb r2, [r0]
    mov r2, #0x93
    ldr r0, =VRAMCNT_C
    strb r2, [r0]
    mov r2, #0x83
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
    @ R1 T1: 8x8, blocks of modes 0 to 3, palette base 0
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x14000000
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
    @ R2 T1 again with PLTT_BASE 20h (byte 200h)
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x14000000
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
    @ R3 T2: 16x8, 8 blocks, palette offset 16 bytes per block, PLTT_BASE 10h
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x14100008
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000010
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
    .word 0x00000022   @ TEXCOORD (16, 8 texels)
    .word 0x00800100
    .word 0x00000023   @ VTX_16 (240, line 72)
    .word 0x04000E00
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (16, 0 texels)
    .word 0x00000100
    .word 0x00000023   @ VTX_16 (240, line 24)
    .word 0x0C000E00
    .word 0x00000000
    @ R4 T3: 8x8 in slot 2, mode 2, palette offsets 0, 16, 32, 48 bytes, PLTT_BASE 40h
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x14008000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000040
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
    @ R5 T4: 8x8 in slot 1, whose palette halfword reads as 0
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x14004800
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
    @ R6 T1 with bit 29 set (no effect on this format)
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x34000000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
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
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
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
texdata_end:
paldata:
    .word 0x08947A8A
    .word 0x0C417FFF
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
    .word 0x4FF53394
    .word 0x04B76856
    .word 0x3D792118
    .word 0x763B59DA
    .word 0x2EFD129C
    .word 0x67BF4B5E
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
    .word 0x763B59DA
    .word 0x2EFD129C
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
    .word 0x00000000
    .word 0x00000000
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
    .word 0x4FF53394
    .word 0x04B76856
paldata_end:
infoT1:
    .word 0x40000000
    .word 0xC0008000
infoT1_end:
infoT2:
    .word 0x80048000
    .word 0x800C8008
    .word 0x80148010
    .word 0x801C8018
infoT2_end:
infoT3:
    .word 0x80048000
    .word 0x800C8008
infoT3_end:
t4data:
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
t4data_end:
texC:
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
    .word 0xE4E4E4E4
texC_end:

