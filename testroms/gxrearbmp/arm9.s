@ SPDX-License-Identifier: Apache-2.0
@ Rear bitmap ROM, ARM9 side (docs/hw/textures.md, "The rear bitmap"). No libnds, no crt0.
@ Loaded at 0x02000000 and entered there (docs/hw/cart-header.md). The ARM7 spins (arm7.s).
@
@ Engine A shows the 3D layer as BG0 over the rear bitmap (the rest of the setup is as in gxalpha; CLEAR_COLOR only gives the rear pixels
@ their polygon ID, 0).
@ VRAM banks C and D are filled through their LCDC windows first and then mapped as texture slots 2
@ and 3, where the rear bitmaps live (docs/hw/vram-banks.md). DISP3DCNT is 4000h:
@ the rear bitmap (bit 14) on; no texture mapping, no blending.
@
@ Regions (the expected picture is in crates/runner/tests/gxrearbmp.rs, worked out from the note).
@   R1  the whole screen is the rear bitmap: colour from slot 2 with the checker of solid and
@       transparent 32x32 tiles (a transparent tile shows the backdrop), scrolled by X 40 and Y 100,
@       so both axes wrap.
@   R2  a white untextured quad over the middle: the depth bitmap in slot 3 is far for bitmap
@       columns below 128 and nearest from 128 on, so the quad draws only over the far columns.
@
@ TODO(hw-unverified): the Y scroll width (7 bits), the modulo-256 wrap, and the depth bitmap applying
@ under a transparent rear pixel follow the model of docs/hw/textures.md ("Unverified"); a pass says
@ the emulator follows the note, not that hardware was measured.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISP3DCNT, 0x04000060
    .equ CLEAR_COLOR, 0x04000350
    .equ CLEAR_DEPTH, 0x04000354
    .equ CLRIMAGE_OFFSET, 0x04000356
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_B, 0x04000241
    .equ VRAMCNT_C, 0x04000242
    .equ VRAMCNT_D, 0x04000243
    .equ VRAMCNT_E, 0x04000244
    .equ LCDC_A, 0x06800000
    .equ LCDC_B, 0x06820000
    .equ LCDC_C, 0x06840000
    .equ LCDC_D, 0x06860000
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

    @ Engine A on the upper screen, the 3D engine on, the rear bitmap on.
    ldr r0, =POWCNT1
    ldr r1, =0x800F
    strh r1, [r0]
    ldr r0, =DISP3DCNT
    mov r1, #0x4000
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

    @ Banks C and D as LCDC memory (MST 0): fill the two 256x256 rear bitmaps.
    mov r2, #0x80
    ldr r0, =VRAMCNT_C
    strb r2, [r0]
    ldr r0, =VRAMCNT_D
    strb r2, [r0]
    @ Colour bitmap (bank C): entry (x, y) is red x / 8, green y / 8, blue 0, alpha bit 15 set when
    @ bit 5 of x and bit 5 of y differ (a 32x32 checker of solid and transparent tiles).
    ldr r0, =LCDC_C
    mov r4, #0
fill_c_row:
    mov r5, #0
fill_c_col:
    mov r6, r5, lsr #3
    mov r7, r4, lsr #3
    orr r6, r6, r7, lsl #5
    eor r7, r5, r4
    tst r7, #0x20
    orrne r6, r6, #0x8000
    strh r6, [r0], #2
    add r5, r5, #1
    cmp r5, #256
    bne fill_c_col
    add r4, r4, #1
    cmp r4, #256
    bne fill_c_row
    @ Depth bitmap (bank D): entry (x, y) is 7FFFh (far) for x below 128 and 0 (nearest) from 128 on,
    @ with the fog bit 15 set on odd rows.
    ldr r0, =LCDC_D
    mov r4, #0
fill_d_row:
    mov r5, #0
fill_d_col:
    mov r6, #0
    cmp r5, #128
    movlo r6, #0x7F00
    orrlo r6, r6, #0xFF
    tst r4, #1
    orrne r6, r6, #0x8000
    strh r6, [r0], #2
    add r5, r5, #1
    cmp r5, #256
    bne fill_d_col
    add r4, r4, #1
    cmp r4, #256
    bne fill_d_row
    @ Bank C as texture slot 2 (MST 3, OFS 2) and bank D as slot 3 (OFS 3).
    mov r2, #0x93
    ldr r0, =VRAMCNT_C
    strb r2, [r0]
    mov r2, #0x9B
    ldr r0, =VRAMCNT_D
    strb r2, [r0]
    @ Scroll the bitmaps: X offset 40, Y offset 100 (CLRIMAGE_OFFSET, the upper half of the word at
    @ CLEAR_DEPTH), so both axes wrap inside the visible 256x192.
    ldr r0, =CLRIMAGE_OFFSET
    ldr r1, =0x6428
    strh r1, [r0]

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
    @ A white quad over the middle: it draws where the rear depth is far and is hidden where it is nearest
    .word 0x00000029   @ POLYGON_ATTR
    .word 0x011F00C0
    .word 0x0000002A   @ TEXIMAGE_PARAM
    .word 0x00000000
    .word 0x0000002B   @ PLTT_BASE
    .word 0x00000000
    .word 0x00000040   @ BEGIN_VTXS
    .word 0x00000001
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 0 texels)
    .word 0x00000000
    .word 0x00000023   @ VTX_16 (64, line 48)
    .word 0x0800F800
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (0, 8 texels)
    .word 0x00800000
    .word 0x00000023   @ VTX_16 (64, line 144)
    .word 0xF800F800
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 8 texels)
    .word 0x00800080
    .word 0x00000023   @ VTX_16 (192, line 144)
    .word 0xF8000800
    .word 0x00000000
    .word 0x00000020   @ COLOR
    .word 0x00007FFF
    .word 0x00000022   @ TEXCOORD (8, 0 texels)
    .word 0x00000080
    .word 0x00000023   @ VTX_16 (192, line 48)
    .word 0x08000800
    .word 0x00000000
    .word 0x00000050   @ SWAP_BUFFERS
    .word 0x00000000   @ auto-sort, Z-buffering
commands_end:

