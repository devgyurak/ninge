@ SPDX-License-Identifier: Apache-2.0
@ Affine-object ROM, ARM9 side: shows rotated, scaled and sheared objects on both screens
@ (docs/hw/obj.md, "An affine object"). Engine A (the upper screen) uses 1D tile mapping with a
@ 32-byte boundary, engine B (the lower screen) 2D mapping. No libnds, no crt0, no image asset:
@ every byte is computed here. Loaded at 0x02000000, entered at 0x02000100. The expected
@ pictures are worked out from the note in crates/runner/tests/objaffine.rs, which holds its own
@ copy of the object and parameter tables below.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the upper screen, engine B on), VRAMCNT_A = 0x82 (bank A,
@ MST 2: engine A OBJ VRAM), VRAMCNT_D = 0x84 (bank D, MST 4: engine B OBJ VRAM), DISPCNT of
@ engine A = 0x00011010 (display mode 1, sprites on, 1D mapping, boundary 32 bytes), of engine B =
@ 0x00011000 (display mode 1, sprites on, 2D mapping).
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
@ at the end of this file are written to entry 0 and up, and the parameter groups to the fourth
@ halfword of entries 4g to 4g+3 (group g's PA, PB, PC, PD), with halfword stores.
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
    .equ AFF, 1 << 8
    .equ DBL, 1 << 9
    .equ OFF, 1 << 9
    .equ GRP, 9

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

    @ The parameter groups: records of (group, PA, PB, PC, PD) halfwords, ended by 0xFFFF; group
    @ g's parameter n goes to byte 6 of OAM entry 4g + n, that is OAM + 32g + 8n + 6.
    ldr r10, =OAM_A
    adr r9, groups_a
    bl write_groups
    ldr r10, =OAM_B
    adr r9, groups_b
    bl write_groups

    ldr r0, =DISPCNT_A
    ldr r1, =0x00011010
    str r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00011000
    str r1, [r0]
spin:
    b spin

write_groups:
    ldrh r0, [r9], #2
    ldr r1, =0xFFFF
    cmp r0, r1
    bxeq lr
    add r2, r10, r0, lsl #5
    add r2, r2, #6
    mov r3, #4
write_param:
    ldrh r1, [r9], #2
    strh r1, [r2], #8
    subs r3, r3, #1
    bne write_param
    b write_groups

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

    @ Engine A, 1D mapping, boundary 32 bytes (a tile number counts 32 bytes), 16 colours unless
    @ C256. Entries 0-7:
    @   0  32x32, group 2          (20, 20)    tile 8     palette 3
    @   1  32x32, group 2, double  (80, 20)    tile 8     palette 3 (a 64x64 box)
    @   2  16x16, group 3          (160, 20)   tile 40    palette 5
    @   3  64x64, 256, group 31, double  (20, 60)  tile 64 (a 128x128 box)
    @   4  16x32, group 1          (170, 70)   tile 100   palette 9
    @   5  8x8, group 4, double    (200, 150)  tile 2     palette 1
    @   6  plain, disable bit      (230, 20)   tile 1
    @   7  16x16, group 0, double  (226, 100)  tile 12    palette 2 (a 32x32 box cut at the right)
    .balign 4
oam_a:
    obj 20, 20, 8, AFF, 2 << GRP | SIZE2, 3 << 12
    obj 20, 80, 8, AFF | DBL, 2 << GRP | SIZE2, 3 << 12
    obj 20, 160, 40, AFF, 3 << GRP | SIZE1, 5 << 12
    obj 60, 20, 64, AFF | DBL | C256, 31 << GRP | SIZE3, 0
    obj 70, 170, 100, AFF | SHAPE_TALL, 1 << GRP | SIZE2, 9 << 12
    obj 150, 200, 2, AFF | DBL, 4 << GRP, 1 << 12
    obj 20, 230, 1, OFF, 0, 0
    obj 100, 226, 12, AFF | DBL, 0 << GRP | SIZE1, 2 << 12
oam_a_end:

    @ Engine B, 2D mapping (tile numbers in a matrix 32 wide). Entries 0-7:
    @   0  32x16, group 1          (10, 10)    tile 0x20  palette 0
    @   1  16x16, 256, group 5, double  (60, 10)  tile 0x40
    @   2  64x32, group 2, double  (100, 50)   tile 0x60  palette 4 (a 128x64 box)
    @   3  8x32, group 3           (10, 100)   tile 0x08  palette 6
    @   4  32x32, group 31         (50, 120)   tile 0x80  palette 7
    @   5  plain, disable bit      (200, 20)   tile 1
    @   6  16x16, group 0          (230, 150)  tile 0x10  palette 12
    @   7  16x16, group 0, double, Y = 240 (150, 240)  tile 0x14  palette 1 (lines 0-7 show it)
    .balign 4
oam_b:
    obj 10, 10, 0x20, AFF | SHAPE_WIDE, 1 << GRP | SIZE2, 0
    obj 10, 60, 0x40, AFF | DBL | C256, 5 << GRP | SIZE1, 0
    obj 50, 100, 0x60, AFF | DBL | SHAPE_WIDE, 2 << GRP | SIZE3, 4 << 12
    obj 100, 10, 0x08, AFF | SHAPE_TALL, 3 << GRP | SIZE1, 6 << 12
    obj 120, 50, 0x80, AFF, 31 << GRP | SIZE2, 7 << 12
    obj 20, 200, 1, OFF, 0, 0
    obj 150, 230, 0x10, AFF, 0 << GRP | SIZE1, 12 << 12
    obj 240, 150, 0x14, AFF | DBL, 0 << GRP | SIZE1, 1 << 12
oam_b_end:

    @ Parameter groups: (group, PA, PB, PC, PD), 8.8 fixed point, then 0xFFFF.
    @   0: identity   1: a quarter turn   2: about 30 degrees   3: magnify by 2   4: shrink by 2
    @   5: a shear   31: about 45 degrees and scale 0.7
    .balign 4
groups_a:
    .hword 0, 256, 0, 0, 256
    .hword 1, 0, 256, -256, 0
    .hword 2, 222, 128, -128, 222
    .hword 3, 128, 0, 0, 128
    .hword 4, 512, 0, 0, 512
    .hword 31, 181, -181, 181, 181
    .hword 0xFFFF
    .balign 4
groups_b:
    .hword 0, 256, 0, 0, 256
    .hword 1, 0, 256, -256, 0
    .hword 2, 222, 128, -128, 222
    .hword 3, 128, 0, 0, 128
    .hword 5, 256, 64, 0, 256
    .hword 31, 181, -181, 181, 181
    .hword 0xFFFF

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
