@ SPDX-License-Identifier: Apache-2.0
@ Tile-object ROM, ARM9 side: shows plain (non-affine) tile objects on both screens
@ (docs/hw/obj.md). Engine A (the upper screen) uses 1D tile mapping with a 64-byte boundary,
@ engine B (the lower screen) uses 2D mapping. No libnds, no crt0, no image asset: every byte is
@ computed here. Loaded at 0x02000000, entered at 0x02000100. The expected pictures are worked
@ out from the note in crates/runner/tests/objtext.rs, which holds its own copy of the object
@ tables below.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the upper screen, engine B on), VRAMCNT_A = 0x82 (bank A,
@ MST 2: engine A OBJ VRAM, 0x06400000), VRAMCNT_D = 0x84 (bank D, MST 4: engine B OBJ VRAM,
@ 0x06600000), DISPCNT of engine A = 0x00111010 (display mode 1, sprites on, 1D mapping,
@ boundary field 1 = 64 bytes), of engine B = 0x00011000 (display mode 1, sprites on, 2D mapping).
@
@ Palettes: OBJ palette entry i of both engines = R i&31, G (3i)&31, B (5i)&31 as a 15-bit
@ colour, for i = 0..255. The backdrop (BG palette entry 0) is 0x03E0 (green) on engine A and
@ 0x7C00 (blue) on engine B.
@
@ Tile data: the first 8 KiB of each OBJ VRAM holds 256 units of 32 bytes. In unit u, byte j of
@ row r (4 bytes a row) holds the 4-bit pixels c = 2j (low nibble) and 2j+1 (high nibble) with the
@ value (5u + 3r + c) & 15, so a pixel value of 0 appears in every tile. An object's 8 bits per
@ pixel tile reads the same bytes as 64 bytes, a pixel a byte.
@
@ OAM: all 256 entries of both engines are first disabled (attr0 bit 9); then the entries listed
@ at the end of this file are written to entry 0 and up. Every other field is described there.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_D, 0x04000243
    .equ BG_PALETTE_A, 0x05000000
    .equ BG_PALETTE_B, 0x05000400
    .equ OBJ_PALETTE_A, 0x05000200
    .equ OBJ_PALETTE_B, 0x05000600
    .equ OBJ_VRAM_A, 0x06400000
    .equ OBJ_VRAM_B, 0x06600000
    .equ OAM_A, 0x07000000
    .equ OAM_B, 0x07000400

    @ Bits of attr0 and attr1.
    .equ C256, 1 << 13
    .equ SHAPE_WIDE, 1 << 14
    .equ SHAPE_TALL, 2 << 14
    .equ HFLIP, 1 << 12
    .equ VFLIP, 1 << 13
    .equ SIZE1, 1 << 14
    .equ SIZE2, 2 << 14
    .equ SIZE3, 3 << 14

    @ The linker script places .head at 0x02000000 and .text at the entry, 0x02000100.
    .section .head, "ax"
    .arm
    .space 0x100, 0

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ The protection unit and both caches are switched off, so the stores below reach VRAM, the
    @ palette RAM and OAM through the bus whatever state the loader left them in.
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
    mov r1, #0x82
    strb r1, [r0]
    ldr r0, =VRAMCNT_D
    mov r1, #0x84
    strb r1, [r0]

    @ The backdrops, and both OBJ palettes.
    ldr r0, =BG_PALETTE_A
    ldr r1, =0x03E0
    strh r1, [r0]
    ldr r0, =BG_PALETTE_B
    ldr r1, =0x7C00
    strh r1, [r0]
    ldr r8, =OBJ_PALETTE_A
    ldr r9, =OBJ_PALETTE_B
    mov r4, #0
pal_loop:
    and r5, r4, #31
    add r6, r4, r4, lsl #1
    and r6, r6, #31
    orr r5, r5, r6, lsl #5
    add r6, r4, r4, lsl #2
    and r6, r6, #31
    orr r5, r5, r6, lsl #10
    mov r7, r4, lsl #1
    strh r5, [r8, r7]
    strh r5, [r9, r7]
    add r4, r4, #1
    cmp r4, #256
    bne pal_loop

    @ Every entry of both OAM halves disabled: attr0 = 0x0200, attr1 = 0, then attr2 and the
    @ fourth halfword 0.
    ldr r0, =OAM_A
    mov r1, #0x200
    mov r2, #0
    mov r4, #256
oam_off:
    str r1, [r0], #4
    str r2, [r0], #4
    subs r4, r4, #1
    bne oam_off

    @ The tile units, to both OBJ VRAMs: 2048 words.
    adr r1, tiles
    ldr r0, =OBJ_VRAM_A
    ldr r2, =OBJ_VRAM_B
    ldr r4, =2048
tile_copy:
    ldr r3, [r1], #4
    str r3, [r0], #4
    str r3, [r2], #4
    subs r4, r4, #1
    bne tile_copy

    @ The object entries: records of (destination, source, words), ended by a zero destination.
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

    ldr r0, =DISPCNT_A
    ldr r1, =0x00111010
    str r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00011000
    str r1, [r0]
spin:
    b spin

    .ltorg

    .balign 4
copies:
    .word OAM_A, oam_a, (oam_a_end - oam_a) / 4
    .word OAM_B, oam_b, (oam_b_end - oam_b) / 4
    .word 0

    @ One OAM entry: Y, X, tile number, and the extra bits of attr0, attr1 and attr2. The fourth
    @ halfword is 0.
    .macro obj y, x, tile, a0, a1, a2
    .word (\y) | (\a0) | (((\x) & 0x1FF | (\a1)) << 16), (\tile) | (\a2)
    .endm

    @ Engine A, 1D mapping, boundary 64 bytes (a tile number counts 64 bytes), 16 colours unless
    @ C256. Entries 0-9:
    @   0  8x8                  (8, 8)      tile 1       palette 0
    @   1  16x16, h flip        (24, 8)     tile 4       palette 3
    @   2  32x16, 256, v flip   (48, 8)     tile 8
    @   3  64x64                (100, 20)   tile 16      palette 5
    @   4  8x32, both flips     (180, 8)    tile 3       palette 9
    @   5  64x32, 256, both     (10, 100)   tile 30
    @   6  32x32, X = -16       (-16, 150)  tile 40      palette 7
    @   7  16x16, at the right  (248, 90)   tile 12      palette 1
    @   8  32x32, Y = 240       (150, 240)  tile 20      palette 2 (lines 0-15 show its lower half)
    @   9  16x32, 256           (200, 100)  tile 50
    .balign 4
oam_a:
    obj 8, 8, 1, 0, 0, 0
    obj 8, 24, 4, 0, HFLIP | SIZE1, 3 << 12
    obj 8, 48, 8, C256 | SHAPE_WIDE, VFLIP | SIZE2, 0
    obj 20, 100, 16, 0, SIZE3, 5 << 12
    obj 8, 180, 3, SHAPE_TALL, HFLIP | VFLIP | SIZE1, 9 << 12
    obj 100, 10, 30, C256 | SHAPE_WIDE, HFLIP | VFLIP | SIZE3, 0
    obj 150, -16, 40, 0, SIZE2, 7 << 12
    obj 90, 248, 12, 0, SIZE1, 1 << 12
    obj 240, 150, 20, 0, SIZE2, 2 << 12
    obj 100, 200, 50, C256 | SHAPE_TALL, SIZE2, 0
oam_a_end:

    @ Engine B, 2D mapping (tile numbers in a matrix 32 wide). Entries 0-7:
    @   0  16x16                (10, 10)    tile 0x22    palette 0
    @   1  32x32, 256           (40, 10)    tile 0x40
    @   2  16x16, 256           (90, 10)    tile 0x61    (bit 0 of the tile number is ignored)
    @   3  64x32, both flips    (120, 40)   tile 0x80    palette 4
    @   4  8x16                 (252, 10)   tile 0x10    palette 6 (only 4 columns fit)
    @   5  32x8, 256, X = -4    (-4, 100)   tile 0x20
    @   6  16x16, Y = 188       (100, 188)  tile 0x30    palette 12 (only 4 lines fit)
    @   7  16x16                (120, 0)    tile 0x05    palette 15
    .balign 4
oam_b:
    obj 10, 10, 0x22, 0, SIZE1, 0
    obj 10, 40, 0x40, C256, SIZE2, 0
    obj 10, 90, 0x61, C256, SIZE1, 0
    obj 40, 120, 0x80, SHAPE_WIDE, HFLIP | VFLIP | SIZE3, 4 << 12
    obj 10, 252, 0x10, SHAPE_TALL, 0, 6 << 12
    obj 100, -4, 0x20, C256 | SHAPE_WIDE, SIZE1, 0
    obj 188, 100, 0x30, 0, SIZE1, 12 << 12
    obj 0, 120, 0x05, 0, SIZE1, 15 << 12
oam_b_end:

    @ 256 units of tile data (see the top of the file).
    .macro units from, count
    .set u, \from
    .rept \count
    .set r, 0
    .rept 8
    .set w, 0
    .set c, 0
    .rept 8
    .set w, w | (((u * 5 + 3 * r + c) & 15) << (4 * c))
    .set c, c + 1
    .endr
    .word w
    .set r, r + 1
    .endr
    .set u, u + 1
    .endr
    .endm
    .balign 4
tiles:
    units 0, 256
