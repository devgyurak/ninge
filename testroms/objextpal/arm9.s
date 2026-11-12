@ SPDX-License-Identifier: Apache-2.0
@ OBJ extended-palette ROM, ARM9 side: shows 256-colour and 16-colour objects with DISPCNT bit 31
@ set on both screens (docs/hw/extended-palettes.md, "Objects"). No libnds, no crt0, no image
@ asset: every byte is computed here. Loaded at 0x02000000, entered at 0x02000100. The expected
@ pictures are worked out from the note in crates/runner/tests/objextpal.rs, which holds its own
@ copy of the formulas and object tables below.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the upper screen, engine B on), VRAMCNT_A = 0x82 (bank A,
@ MST 2: engine A OBJ VRAM), VRAMCNT_D = 0x84 (bank D, MST 4: engine B OBJ VRAM). Banks F, G and I
@ are filled while in LCDC mode (MST 0), then F is mapped as engine A's OBJ extended palette
@ (VRAMCNT_F = 0x85, MST 5) and I as engine B's (VRAMCNT_I = 0x83, MST 3); G stays in LCDC mode
@ until the split. DISPCNT of both engines = 0x80011000 (display mode 1, sprites on, 2D mapping,
@ bit 31: OBJ extended palettes).
@
@ OBJ extended palette colours: the halfword of palette p, entry i (one 8 KiB slot) is
@   (p * 0xB3 + i * 0x47 + K) & 0x7FFF
@ with K = 0x1A3 in bank F, 0x3C5 in bank G and 0x2E1 in bank I.
@
@ Standard OBJ palettes: entry i = R i&31, G (3i)&31, B (5i)&31 as a 15-bit colour, for i = 0..255.
@ The backdrop (BG palette entry 0) is 0x03E0 (green) on engine A and 0x7C00 (blue) on engine B.
@
@ Tile data: the first 8 KiB of each OBJ VRAM holds 256 units of 32 bytes. In unit u, byte j of
@ row r (4 bytes a row) holds the 4-bit pixels c = 2j (low nibble) and 2j+1 (high nibble) with the
@ value (5u + 3r + c) & 15, so a pixel value of 0 appears in every tile. An 8 bits per pixel tile
@ reads the same bytes as 64 bytes.
@
@ The split, by polling VCOUNT (a write made once VCOUNT reaches 95 takes effect from line 96, and
@ the first state is written again once VCOUNT reaches 200):
@   lines 0-95    engine A reads bank F; engine B has DISPCNT bit 31 set (bank I).
@   lines 96-191  bank F is back in LCDC mode and G is mapped (MST 5), so engine A reads bank G;
@                 engine B's DISPCNT is 0x00011000, bit 31 clear: its 256-colour objects read the
@                 standard palette.
@
@ OAM: all 256 entries of both engines are first disabled (attr0 bit 9); then the entries listed
@ at the end of this file are written to entry 0 and up, and the parameter groups to the fourth
@ halfword of entries 4g to 4g+3, with halfword stores.
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
    .equ VCOUNT, 0x04000006
    .equ VRAMCNT_F, 0x04000245
    .equ VRAMCNT_G, 0x04000246
    .equ VRAMCNT_I, 0x04000249
    .equ LCDC_F, 0x06890000
    .equ LCDC_G, 0x06894000
    .equ LCDC_I, 0x068A0000
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

    @ Banks F, G and I in LCDC mode, filled with the extended palettes, then F and I mapped.
    mov r1, #0x80
    ldr r0, =VRAMCNT_F
    strb r1, [r0]
    ldr r0, =VRAMCNT_G
    strb r1, [r0]
    ldr r0, =VRAMCNT_I
    strb r1, [r0]
    ldr r0, =LCDC_F
    ldr r3, =0x1A3
    bl fill_ext
    ldr r0, =LCDC_G
    ldr r3, =0x3C5
    bl fill_ext
    ldr r0, =LCDC_I
    ldr r3, =0x2E1
    bl fill_ext
    ldr r0, =VRAMCNT_F
    mov r1, #0x85
    strb r1, [r0]
    ldr r0, =VRAMCNT_I
    mov r1, #0x83
    strb r1, [r0]

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
    ldr r1, =0x80011000
    str r1, [r0]
    ldr r0, =DISPCNT_B
    str r1, [r0]

    ldr r6, =VCOUNT
frame_loop:
wait_95:
    ldrh r0, [r6]
    cmp r0, #95
    blo wait_95
    ldr r0, =VRAMCNT_F
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =VRAMCNT_G
    mov r1, #0x85
    strb r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00011000
    str r1, [r0]
wait_200:
    ldrh r0, [r6]
    cmp r0, #200
    blo wait_200
    ldr r0, =VRAMCNT_F
    mov r1, #0x85
    strb r1, [r0]
    ldr r0, =VRAMCNT_G
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x80011000
    str r1, [r0]
wait_wrap:
    ldrh r0, [r6]
    cmp r0, #95
    bhs wait_wrap
    b frame_loop

fill_ext:
    @ r0 = destination, r3 = K: 16 palettes of 256 colours, (p * 0xB3 + i * 0x47 + K) & 0x7FFF.
    ldr r9, =0x7FFF
    ldr r11, =0xB3
    mov r4, r3
    mov r5, #16
fe_palette:
    mov r6, r4
    mov r7, #256
fe_index:
    and r8, r6, r9
    strh r8, [r0], #2
    add r6, r6, #0x47
    subs r7, r7, #1
    bne fe_index
    add r4, r4, r11
    subs r5, r5, #1
    bne fe_palette
    bx lr

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

    @ Both engines list the same eight objects, 2D mapping (tile numbers in a matrix 32 wide), 32x32
    @ (shape 0, size 2), in two bands: entries 0-3 at y = 10 (lines 10-41, the first half of the
    @ split) and entries 4-7 at y = 110 (the second half). In each band:
    @   - a 256-colour object of tile 0x40 with palette number 2 (band 1) or 11 (band 2) at x = 10;
    @   - the same with palette number 9 or 14 at x = 60;
    @   - a 16-colour object of tile 0x40 with palette number 3 or 6 at x = 110 (standard palette);
    @   - an affine 256-colour object (group 1, a quarter turn) with palette number 5 or 12 at
    @     x = 160.
    .balign 4
oam_a:
    obj 10, 10, 0x40, C256, SIZE2, 2 << 12
    obj 10, 60, 0x40, C256, SIZE2, 9 << 12
    obj 10, 110, 0x40, 0, SIZE2, 3 << 12
    obj 10, 160, 0x40, AFF | C256, 1 << GRP | SIZE2, 5 << 12
    obj 110, 10, 0x40, C256, SIZE2, 11 << 12
    obj 110, 60, 0x40, C256, SIZE2, 14 << 12
    obj 110, 110, 0x40, 0, SIZE2, 6 << 12
    obj 110, 160, 0x40, AFF | C256, 1 << GRP | SIZE2, 12 << 12
oam_a_end:

    .balign 4
oam_b:
    obj 10, 10, 0x40, C256, SIZE2, 2 << 12
    obj 10, 60, 0x40, C256, SIZE2, 9 << 12
    obj 10, 110, 0x40, 0, SIZE2, 3 << 12
    obj 10, 160, 0x40, AFF | C256, 1 << GRP | SIZE2, 5 << 12
    obj 110, 10, 0x40, C256, SIZE2, 11 << 12
    obj 110, 60, 0x40, C256, SIZE2, 14 << 12
    obj 110, 110, 0x40, 0, SIZE2, 6 << 12
    obj 110, 160, 0x40, AFF | C256, 1 << GRP | SIZE2, 12 << 12
oam_b_end:

    @ Parameter group 1: a quarter turn, PA = 0, PB = 256, PC = -256, PD = 0 (8.8 fixed point).
    .balign 4
groups_a:
    .hword 1, 0, 256, -256, 0
    .hword 0xFFFF
    .balign 4
groups_b:
    .hword 1, 0, 256, -256, 0
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
