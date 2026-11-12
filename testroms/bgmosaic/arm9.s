@ SPDX-License-Identifier: Apache-2.0
@ Mosaic ROM, ARM9 side: shows mosaic on a text layer, an affine layer and a 256-colour bitmap
@ (docs/hw/bg-modes.md, "Mosaic"): engine A (the upper screen) in BG mode 4 with BG0 text, BG2
@ affine and BG3 a 256-colour bitmap, all three with mosaic; engine B (the lower screen) in BG mode
@ 0 with BG0 text with mosaic over BG1 text without it. No libnds, no crt0, no image asset: every
@ byte is computed here or listed below. Loaded at 0x02000000, entered at 0x02000100. The expected
@ picture is worked out from the note in crates/runner/tests/bgmosaic.rs, which holds its own copy
@ of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A = 0x81
@ (bank A, MST 1: engine A BG VRAM, 0x06000000), VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM,
@ 0x06200000). DISPCNT of engine A = 0x00010D04 (display mode 1, BG mode 4, BG0, BG2 and BG3 on),
@ engine B = 0x00010300 (display mode 1, BG mode 0, BG0 and BG1 on). MOSAIC of engine A = 0x0024
@ (blocks 5 pixels wide, 3 lines high), of engine B = 0x0031 (2 wide, 4 high). VRAM offsets count
@ from the start of the engine's window.
@
@ Palette (both engines' BG palette, 0x05000000 and 0x05000400): entry i = R i&31, G (3i)&31,
@ B (5i)&31 as a 15-bit colour, for i = 0..255; then entry 0 (the backdrop) = 0x03E0, green, on
@ engine A and 0x7C00, blue, on engine B.
@
@ Text tiles (4 bits per pixel, character block 1 = 0x4000 in both engines): tile 1 "asym" (row r,
@ column c holds 1 + ((3r + c) mod 8)), tile 2 (all pixels 9), tile 3 "diag" (row r, column c holds
@ (r + c) mod 8; value 0 is transparent, so every eighth pixel is a hole).
@
@ Engine A:
@   BG0  0x0044  priority 0, text 4bpp, 256x256, mosaic, char block 1, screen block 0; HOFS 3,
@        VOFS 5. Tile 1 at cells (3..8, 2), flipped horizontally at (3..8, 3); tile 3 at (3..8, 4)
@        and (3..8, 9); tile 2 at (10..13, 6), flipped vertically at (10..13, 7).
@   BG2  0x4249  priority 1, affine, 256x256, wrap off, mosaic, char block 2 (0x8000), screen block 2
@        (0x1000). Tiles 1-4: row r, column c of tile t holds 16 t + ((3r + c) mod 16); cell (cx,
@        cy) holds tile 1 + ((cx + 2 cy) mod 4). PA 0x00E0, PB 0xFF90, PC 0x0070, PD 0x00E0, X 0x1000
@        (16.0), Y 0x0800 (8.0). Where the transform leaves the layer BG2 is transparent and BG3 shows.
@   BG3  0x64C2  priority 2, 256-colour bitmap (bit 7), 256x256, wrap on, mosaic, screen block 4
@        (0x10000). PA 0x0100, PD 0x0100, PB, PC, X and Y 0. Texel (tx, ty) holds
@        1 + ((5 tx + 3 ty) mod 255).
@ Engine B:
@   BG0  0x0044  priority 0, text 4bpp, 256x256, mosaic, char block 1, screen block 0. Tile 1 at
@        cells (1..6, 1), tile 3 at (1..6, 2), tile 2 at (8..9, 4), flipped horizontally at
@        (8..9, 5).
@   BG1  0x0105  priority 1, text 4bpp, 256x256, no mosaic, char block 1, screen block 1 (0x0800).
@        Tile 1 on row 8 and tile 3 on row 9 (all 32 cells); tile 1 flipped horizontally at
@        (2..10, 1), vertically at (2..10, 2) and both at (2..10, 3).
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_C, 0x04000242
    .equ BG0CNT_A, 0x04000008
    .equ BG0HOFS_A, 0x04000010
    .equ BG2CNT_A, 0x0400000C
    .equ AFFINE_A, 0x04000020
    .equ MOSAIC_A, 0x0400004C
    .equ BG0CNT_B, 0x04001008
    .equ MOSAIC_B, 0x0400104C
    .equ PAL_A, 0x05000000
    .equ PAL_B, 0x05000400
    .equ VRAM_A, 0x06000000
    .equ VRAM_B, 0x06200000
    .equ BUF, 0x02100000

    @ The linker script places .head at 0x02000000 and .text at the entry, 0x02000100.
    .section .head, "ax"
    .arm
    .space 0x100, 0

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ The protection unit and both caches are switched off, as in the memmap ROM, so the
    @ stores below reach VRAM through the bus whatever state the loader left them in.
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop

    ldr r0, =POWCNT1
    ldr r1, =0x8203
    strh r1, [r0]
    ldr r0, =VRAMCNT_A
    mov r1, #0x81
    strb r1, [r0]
    ldr r0, =VRAMCNT_C
    mov r1, #0x84
    strb r1, [r0]

    ldr r8, =PAL_A
    ldr r9, =0x03E0
    bl make_palette
    ldr r8, =PAL_B
    ldr r9, =0x7C00
    bl make_palette

    @ Made in main RAM (a byte store to VRAM is dropped), copied to VRAM by words.
    @ Affine tiles 1-4 at BUF (320 bytes; tile 0 stays zero).
    ldr r0, =BUF
    mov r1, #64
tile_loop:
    mov r2, r1, lsr #6
    mov r3, r1, lsr #3
    and r3, r3, #7
    and r4, r1, #7
    add r3, r3, r3, lsl #1
    add r3, r3, r4
    and r3, r3, #15
    add r3, r3, r2, lsl #4
    strb r3, [r0, r1]
    add r1, r1, #1
    cmp r1, #320
    bne tile_loop

    @ The 32x32 affine map at BUF + 0x1000: cell (cx, cy) holds tile 1 + ((cx + 2 cy) mod 4).
    ldr r0, =BUF + 0x1000
    mov r2, #0
map_row:
    mov r3, #0
map_cell:
    add r4, r3, r2, lsl #1
    and r4, r4, #3
    add r4, r4, #1
    strb r4, [r0], #1
    add r3, r3, #1
    cmp r3, #32
    bne map_cell
    add r2, r2, #1
    cmp r2, #32
    bne map_row

    @ The 256-colour bitmap, 64 KiB at BUF + 0x10000: byte = 1 + ((5 tx + 3 ty) mod 255), with v the
    @ running (5 tx + 3 ty) mod 255.
    ldr r0, =BUF + 0x10000
    mov r2, #0                   @ (3 ty) mod 255
    mov r1, #0                   @ ty
bmp_row:
    mov r3, r2
    mov r4, #0
bmp_pixel:
    add r5, r3, #1
    strb r5, [r0], #1
    add r3, r3, #5
    cmp r3, #255
    subhs r3, r3, #255
    add r4, r4, #1
    cmp r4, #256
    bne bmp_pixel
    add r2, r2, #3
    cmp r2, #255
    subhs r2, r2, #255
    add r1, r1, #1
    cmp r1, #256
    bne bmp_row

    @ Copy records: (destination, source, words), ended by a zero destination.
    adr r9, copies
copy_next:
    ldr r0, [r9], #4
    cmp r0, #0
    beq copies_done
    ldr r1, [r9], #4
    ldr r2, [r9], #4
copy_word:
    ldr r3, [r1], #4
    str r3, [r0], #4
    subs r2, r2, #1
    bne copy_word
    b copy_next
copies_done:

    @ Map cells of the text layers: records of (map base, first cell, count, entry).
    adr r9, runs
run_next:
    ldr r0, [r9], #4
    cmp r0, #0
    beq runs_done
    ldr r1, [r9], #4
    ldr r2, [r9], #4
    ldr r3, [r9], #4
    add r0, r0, r1, lsl #1
run_cell:
    strh r3, [r0], #2
    subs r2, r2, #1
    bne run_cell
    b run_next
runs_done:

    @ Engine A: BG0CNT and the scroll, BG2CNT and BG3CNT, BG2's and BG3's transform, MOSAIC.
    ldr r0, =BG0CNT_A
    ldr r1, =0x0044
    strh r1, [r0]
    ldr r0, =BG0HOFS_A
    ldr r1, =0x00050003          @ VOFS 5, HOFS 3
    str r1, [r0]
    ldr r0, =BG2CNT_A
    ldr r1, =0x64C24249
    str r1, [r0]
    ldr r0, =AFFINE_A
    ldr r1, =0xFF9000E0          @ BG2 PB:PA
    str r1, [r0]
    ldr r1, =0x00E00070          @ BG2 PD:PC
    str r1, [r0, #4]
    ldr r1, =0x00001000          @ BG2X
    str r1, [r0, #8]
    ldr r1, =0x00000800          @ BG2Y
    str r1, [r0, #12]
    ldr r1, =0x00000100          @ BG3 PB:PA
    str r1, [r0, #16]
    ldr r1, =0x01000000          @ BG3 PD:PC
    str r1, [r0, #20]
    ldr r0, =MOSAIC_A
    ldr r1, =0x0024
    strh r1, [r0]
    @ Engine B: BG0CNT and BG1CNT, MOSAIC.
    ldr r0, =BG0CNT_B
    ldr r1, =0x01050044
    str r1, [r0]
    ldr r0, =MOSAIC_B
    ldr r1, =0x0031
    strh r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00010300
    str r1, [r0]
    ldr r0, =DISPCNT_A
    ldr r1, =0x00010D04
    str r1, [r0]
done:
    b done

    .ltorg

    @ make_palette: r8 = palette base, r9 = entry 0 (the backdrop).
make_palette:
    mov r4, #0
pal_loop:
    and r5, r4, #31
    add r6, r4, r4, lsl #1
    and r6, r6, #31
    orr r5, r5, r6, lsl #5
    add r6, r4, r4, lsl #2
    and r6, r6, #31
    orr r5, r5, r6, lsl #10
    add r7, r8, r4, lsl #1
    strh r5, [r7]
    add r4, r4, #1
    cmp r4, #256
    bne pal_loop
    strh r9, [r8]
    bx lr

    .balign 4
copies:
    @ Engine A.
    .word VRAM_A + 0x4000 + 1 * 32, tile_asym, 8
    .word VRAM_A + 0x4000 + 2 * 32, tile_nine, 8
    .word VRAM_A + 0x4000 + 3 * 32, tile_diag, 8
    .word VRAM_A + 0x8000, BUF, 80                     @ BG2's tiles
    .word VRAM_A + 0x1000, BUF + 0x1000, 256           @ BG2's map
    .word VRAM_A + 0x10000, BUF + 0x10000, 16384       @ BG3's bitmap
    @ Engine B.
    .word VRAM_B + 0x4000 + 1 * 32, tile_asym, 8
    .word VRAM_B + 0x4000 + 2 * 32, tile_nine, 8
    .word VRAM_B + 0x4000 + 3 * 32, tile_diag, 8
    .word 0

runs:
    @ A BG0: screen block 0, at 0x06000000.
    .word VRAM_A, 2 * 32 + 3, 6, 0x0001
    .word VRAM_A, 3 * 32 + 3, 6, 0x0401
    .word VRAM_A, 4 * 32 + 3, 6, 0x0003
    .word VRAM_A, 9 * 32 + 3, 6, 0x0003
    .word VRAM_A, 6 * 32 + 10, 4, 0x0002
    .word VRAM_A, 7 * 32 + 10, 4, 0x0802
    @ B BG0: screen block 0, at 0x06200000.
    .word VRAM_B, 1 * 32 + 1, 6, 0x0001
    .word VRAM_B, 2 * 32 + 1, 6, 0x0003
    .word VRAM_B, 4 * 32 + 8, 2, 0x0002
    .word VRAM_B, 5 * 32 + 8, 2, 0x0402
    @ B BG1: screen block 1, at 0x06200800.
    .word VRAM_B + 0x800, 8 * 32, 32, 0x0001
    .word VRAM_B + 0x800, 9 * 32, 32, 0x0003
    .word VRAM_B + 0x800, 1 * 32 + 2, 9, 0x0401
    .word VRAM_B + 0x800, 2 * 32 + 2, 9, 0x0801
    .word VRAM_B + 0x800, 3 * 32 + 2, 9, 0x0C01
    .word 0

    @ "asym": 4 bits per pixel, row r column c = 1 + ((3r + c) mod 8); the left pixel is the low
    @ nibble.
tile_asym:
    .word 0x87654321, 0x32187654, 0x65432187, 0x18765432
    .word 0x43218765, 0x76543218, 0x21876543, 0x54321876
    @ A tile whose pixels are all 9.
tile_nine:
    .word 0x99999999, 0x99999999, 0x99999999, 0x99999999
    .word 0x99999999, 0x99999999, 0x99999999, 0x99999999
    @ "diag": row r column c = (r + c) mod 8, so 0 (transparent) once a row.
tile_diag:
    .word 0x76543210, 0x07654321, 0x10765432, 0x21076543
    .word 0x32107654, 0x43210765, 0x54321076, 0x65432107
