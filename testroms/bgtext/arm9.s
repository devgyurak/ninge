@ SPDX-License-Identifier: Apache-2.0
@ Text-background ROM, ARM9 side: builds tile data, maps and a palette in VRAM bank A and the
@ palette RAM, then shows four text layers of engine A in display mode 1 (docs/hw/bg-text.md).
@ No libnds, no crt0, no image asset: every byte is computed here or listed below. Loaded at
@ 0x02000000, entered at 0x02000100. The expected picture is worked out from the note in
@ crates/runner/tests/bgtext.rs, which holds its own copy of the data described here.
@
@ Setup: POWCNT1 = 0x8003 (engine A on the UPPER screen, written explicitly), VRAMCNT_A = 0x81
@ (bank A, MST 1: engine A BG VRAM, OFS 0), DISPCNT = 0x01010F00 (display mode 1, BG mode 0, BG0-BG3
@ on, character base 1 = 0x10000, screen base 0).
@
@ Palette (BG palette, 0x05000000): entry i = R i&31, G (3i)&31, B (5i)&31 as a 15-bit colour,
@ for i = 0..255; then entry 0 (the backdrop) = 0x03E0, green.
@
@ Layers (BGxCNT, scroll, what is in the map; the VRAM offsets count from 0x06000000):
@   BG0  0x0000  priority 0, 4bpp, 256x256, char block 0 (tiles at 0x10000), screen block 0.
@        Tile 1 "asym" (row r, column c holds 1 + ((3r + c) mod 8)) at cells (2,2) plain, (3,2)
@        flipped horizontally, (4,2) vertically, (5,2) both, (6,2) with palette number 3, (6,3)
@        with palette number 3 and flipped horizontally. Tile 2 (all pixels 9) at cells
@        (10..12, 8..10).
@   BG1  0x4285  priority 1, 8bpp, 512x256, char block 1 (0x14000), screen blocks 2 and 3,
@        HOFS 252, VOFS 3. Tile 1 "grad" (row r, column c holds 16 + 8r + c) on row 4 of both
@        blocks, flipped vertically at cell (31,6) of block 0 and horizontally at cell (0,6)
@        of block 1.
@   BG2  0x8409  priority 1, 4bpp, 256x512, char block 2 (0x18000), screen blocks 4 and 5,
@        HOFS 0, VOFS 250. Tile 1 "asym" with palette number 5 at cell (5,31) of block 0, cell
@        (5,0) of block 1 flipped vertically, and cell (5,3) of block 1.
@   BG3  0xC68E  priority 2, 8bpp, 512x512, char block 3 (0x1C000), screen blocks 6-9.
@        Tiles 1-4 are solid, with the values 100-103. In block b (0-3) the cells (0..15,
@        0..23) hold tile b + 1; the rest is tile 0. HOFS is 0, but from line 101 to the end
@        of the frame it is 256 (the ROM polls VCOUNT: it writes 256 once VCOUNT reaches 100 and
@        0 again once it reaches 200), so lines 0-100 show block 0 and lines 101-191 show block 1.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ BG_REGS, 0x04000008
    .equ BG1HOFS, 0x04000014
    .equ BG3HOFS, 0x0400001C
    .equ VCOUNT, 0x04000006
    .equ PALETTE, 0x05000000
    .equ BG_VRAM, 0x06000000

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
    ldr r1, =0x8003
    strh r1, [r0]
    ldr r0, =VRAMCNT_A
    mov r1, #0x81
    strb r1, [r0]

    @ The palette.
    ldr r8, =PALETTE
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
    ldr r5, =0x03E0
    strh r5, [r8]

    @ Tile data: records of (destination, source, words), ended by a zero destination.
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

    @ Map cells: records of (map base, first cell, count, entry), ended by a zero base. A run
    @ writes `count` consecutive halfwords starting `first cell` halfwords into the base.
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

    @ BG0CNT-BG3CNT and the scrolls of BG1, BG2 and BG3 (BG0 has none).
    ldr r0, =BG_REGS
    ldr r1, =0x42850000        @ BG1CNT high, BG0CNT 0x0000 low
    str r1, [r0]
    ldr r1, =0xC68E8409        @ BG3CNT high, BG2CNT low
    str r1, [r0, #4]
    ldr r1, =0x000300FC        @ BG1VOFS 3, BG1HOFS 252
    str r1, [r0, #12]
    ldr r1, =0x00FA0000        @ BG2VOFS 250, BG2HOFS 0
    str r1, [r0, #16]
    mov r1, #0                 @ BG3VOFS 0, BG3HOFS 0
    str r1, [r0, #20]
    ldr r0, =DISPCNT
    ldr r1, =0x01010F00
    str r1, [r0]

    @ The raster split: BG3's scroll is 256 from line 101 to the end of the frame. A write made
    @ after VCOUNT reads 100 lands after line 100 started, so line 100 still has the old scroll.
    ldr r6, =VCOUNT
    ldr r7, =BG3HOFS
    ldr r5, =256
frame_loop:
wait_100:
    ldrh r0, [r6]
    cmp r0, #100
    blo wait_100
    strh r5, [r7]
wait_200:
    ldrh r0, [r6]
    cmp r0, #200
    blo wait_200
    mov r1, #0
    strh r1, [r7]
wait_wrap:
    ldrh r0, [r6]
    cmp r0, #100
    bhs wait_wrap
    b frame_loop

    .ltorg

    .balign 4
copies:
    .word BG_VRAM + 0x10000 + 1 * 32, tile_asym, 8
    .word BG_VRAM + 0x10000 + 2 * 32, tile_nine, 8
    .word BG_VRAM + 0x14000 + 1 * 64, tile_grad, 16
    .word BG_VRAM + 0x18000 + 1 * 32, tile_asym, 8
    .word BG_VRAM + 0x1C000 + 1 * 64, tile_solid8, 64
    .word 0

runs:
    @ BG0: screen block 0, at 0x06000000.
    .word BG_VRAM, 2 * 32 + 2, 1, 0x0001
    .word BG_VRAM, 2 * 32 + 3, 1, 0x0401
    .word BG_VRAM, 2 * 32 + 4, 1, 0x0801
    .word BG_VRAM, 2 * 32 + 5, 1, 0x0C01
    .word BG_VRAM, 2 * 32 + 6, 1, 0x3001
    .word BG_VRAM, 3 * 32 + 6, 1, 0x3401
    .word BG_VRAM, 8 * 32 + 10, 3, 0x0002
    .word BG_VRAM, 9 * 32 + 10, 3, 0x0002
    .word BG_VRAM, 10 * 32 + 10, 3, 0x0002
    @ BG1: screen blocks 2 and 3, at 0x06001000 and 0x06001800.
    .word BG_VRAM + 0x1000, 4 * 32, 32, 0x0001
    .word BG_VRAM + 0x1800, 4 * 32, 32, 0x0001
    .word BG_VRAM + 0x1000, 6 * 32 + 31, 1, 0x0801
    .word BG_VRAM + 0x1800, 6 * 32, 1, 0x0401
    @ BG2: screen blocks 4 and 5, at 0x06002000 and 0x06002800.
    .word BG_VRAM + 0x2000, 31 * 32 + 5, 1, 0x5001
    .word BG_VRAM + 0x2800, 5, 1, 0x5801
    .word BG_VRAM + 0x2800, 3 * 32 + 5, 1, 0x5001
    @ BG3: screen blocks 6-9, at 0x06003000 + 0x800 * b; 24 runs of 16 cells in each.
    .macro bg3_block base, tile
    .set cy, 0
    .rept 24
    .word \base, cy * 32, 16, \tile
    .set cy, cy + 1
    .endr
    .endm
    bg3_block BG_VRAM + 0x3000, 1
    bg3_block BG_VRAM + 0x3800, 2
    bg3_block BG_VRAM + 0x4000, 3
    bg3_block BG_VRAM + 0x4800, 4
    .word 0

    @ "asym": 4 bits per pixel, row r column c = 1 + ((3r + c) mod 8); the left pixel is the low
    @ nibble.
tile_asym:
    .word 0x87654321, 0x32187654, 0x65432187, 0x18765432
    .word 0x43218765, 0x76543218, 0x21876543, 0x54321876
    @ A tile of 4 bits per pixel whose pixels are all 9.
tile_nine:
    .word 0x99999999, 0x99999999, 0x99999999, 0x99999999
    .word 0x99999999, 0x99999999, 0x99999999, 0x99999999
    @ "grad": 8 bits per pixel, row r column c = 16 + 8r + c.
tile_grad:
    .word 0x13121110, 0x17161514, 0x1B1A1918, 0x1F1E1D1C
    .word 0x23222120, 0x27262524, 0x2B2A2928, 0x2F2E2D2C
    .word 0x33323130, 0x37363534, 0x3B3A3938, 0x3F3E3D3C
    .word 0x43424140, 0x47464544, 0x4B4A4948, 0x4F4E4D4C
    @ Four solid tiles of 8 bits per pixel: 100, 101, 102, 103.
tile_solid8:
    .rept 16
    .word 0x64646464
    .endr
    .rept 16
    .word 0x65656565
    .endr
    .rept 16
    .word 0x66666666
    .endr
    .rept 16
    .word 0x67676767
    .endr
