@ SPDX-License-Identifier: Apache-2.0
@ Extended-palette ROM, ARM9 side: shows BG layers that read extended palettes on both screens
@ (docs/hw/extended-palettes.md). No libnds, no crt0, no image asset: every byte is computed here.
@ Loaded at 0x02000000, entered at 0x02000100. The expected pictures are worked out from the note in
@ crates/runner/tests/bgextpal.rs, which holds its own copy of the formulas below.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the upper screen, engine B on); VRAMCNT_A = 0x81 (bank A, MST 1:
@ engine A BG VRAM at 0x06000000) and VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM at
@ 0x06200000). The banks of the extended palettes are filled while they are in LCDC mode (MST 0),
@ then mapped: E = 0x84 (MST 4: engine A BG slots 0-3), H = 0x82 (MST 2: engine B BG slots 0-3);
@ F and G stay in LCDC mode until the split below.
@
@ Extended palette colours: the halfword of slot s, palette p, index i is
@   (s * 0x1F1 + p * 0xB3 + i * 0x47 + K) & 0x7FFF
@ with K = 0x000 in bank E, 0x2A5 in banks F and G, 0x333 in bank H. E holds slots 0-3, F slots 0
@ and 1, G slots 2 and 3, H slots 0-3 (8 KiB a slot, 16 palettes of 256 colours).
@
@ Standard palettes (BG palette of each engine): entry i = R i&31, G (3i)&31, B (5i)&31 as a 15-bit
@ colour, entry 0 (the backdrop) 0x03E0 on engine A and 0x7C00 on engine B.
@
@ Tiles: tile 1 of 8 bits per pixel, row r column c = 1 + ((3r + c) & 7), at byte 64 of character
@ block 0 of both BG VRAMs; tile 1 of 4 bits per pixel with the same values, at byte 32 of character
@ block 1 (0x4000) of engine A.
@
@ Each layer fills one band of cell columns, rows 0-23 (all 192 lines): BG0 columns 0-7 (x 0-63), BG1
@ 8-15, BG2 16-23, BG3 24-31. The map entry of cell (column, row) is tile 1 with palette number
@ row & 15 (bits 12-15).
@   Engine A, BG mode 3, DISPCNT 0x40010F03 (extended palettes on):
@     BG0 0x0180  text, 256 colours, screen block 1: reads slot 0 (bit 13 clear).
@     BG1 0x2280  text, 256 colours, screen block 2, bit 13 set: reads slot 3 (not 1).
@     BG2 0x0304  text, 16 colours, screen block 3, character block 1: the standard palette.
@     BG3 0x4400  extended layer, 16-bit tile entries, 256x256, screen block 4: reads slot 3.
@   Engine B, BG mode 4, DISPCNT 0x40010F04:
@     BG0 0x0180, BG1 0x2280 (as engine A), BG2 0x4300 an affine layer with one-byte map entries (256x256,
@     screen block 3): the standard palette; BG3 0x4400 as engine A.
@ The affine layers (BG2 and BG3) have PA = PD = 0x100.
@
@ The split, by polling VCOUNT (a write made once VCOUNT reaches 95 takes effect from line 96, and
@ the first state is written again once VCOUNT reaches 200):
@   lines 0-95    engine A reads bank E; engine B has DISPCNT bit 30 set (bank H).
@   lines 96-191  bank E leaves the mapping (LCDC mode), F is mapped (MST 4, OFS 0) and G (MST 4,
@                 OFS 1), so engine A reads F and G; engine B's DISPCNT is 0x00010F04, bit 30 clear:
@                 its layers read the standard palette.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ BGCNT_A, 0x04000008
    .equ BGCNT_B, 0x04001008
    .equ AFFINE_A, 0x04000020
    .equ AFFINE_B, 0x04001020
    .equ VCOUNT, 0x04000006
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_C, 0x04000242
    .equ VRAMCNT_E, 0x04000244
    .equ VRAMCNT_F, 0x04000245
    .equ VRAMCNT_G, 0x04000246
    .equ VRAMCNT_H, 0x04000248
    .equ LCDC_E, 0x06880000
    .equ LCDC_F, 0x06890000
    .equ LCDC_G, 0x06894000
    .equ LCDC_H, 0x06898000
    .equ BG_PALETTE_A, 0x05000000
    .equ BG_PALETTE_B, 0x05000400
    .equ BG_VRAM_A, 0x06000000
    .equ BG_VRAM_B, 0x06200000

    @ The linker script places .head at 0x02000000 and .text at the entry, 0x02000100.
    .section .head, "ax"
    .arm
    .space 0x100, 0

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ The protection unit and both caches are switched off, so the stores below reach VRAM, the
    @ palette RAM and the registers through the bus whatever state the loader left them in.
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
    @ E, F, G and H in LCDC mode to be filled.
    mov r1, #0x80
    ldr r0, =VRAMCNT_E
    strb r1, [r0]
    ldr r0, =VRAMCNT_F
    strb r1, [r0]
    ldr r0, =VRAMCNT_G
    strb r1, [r0]
    ldr r0, =VRAMCNT_H
    strb r1, [r0]

    @ The standard palettes, then the backdrops.
    ldr r8, =BG_PALETTE_A
    ldr r9, =BG_PALETTE_B
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
    ldr r1, =0x03E0
    strh r1, [r8]
    ldr r1, =0x7C00
    strh r1, [r9]

    @ The extended palettes: destination, slots, first slot, K.
    ldr r0, =LCDC_E
    mov r1, #4
    mov r2, #0
    ldr r3, =0x000
    bl fill_ext
    ldr r0, =LCDC_F
    mov r1, #2
    mov r2, #0
    ldr r3, =0x2A5
    bl fill_ext
    ldr r0, =LCDC_G
    mov r1, #2
    mov r2, #2
    ldr r3, =0x2A5
    bl fill_ext
    ldr r0, =LCDC_H
    mov r1, #4
    mov r2, #0
    ldr r3, =0x333
    bl fill_ext
    @ E and H into their extended-palette mappings; F and G stay in LCDC mode.
    ldr r0, =VRAMCNT_E
    mov r1, #0x84
    strb r1, [r0]
    ldr r0, =VRAMCNT_H
    mov r1, #0x82
    strb r1, [r0]

    @ Tile data: 8 bits per pixel tile 1 in both BG VRAMs, 4 bits per pixel tile 1 in engine A's
    @ character block 1.
    ldr r0, =BG_VRAM_A + 64
    adr r1, tile8
    mov r2, #16
    bl copy_words
    ldr r0, =BG_VRAM_B + 64
    adr r1, tile8
    mov r2, #16
    bl copy_words
    ldr r0, =BG_VRAM_A + 0x4000 + 32
    adr r1, tile4
    mov r2, #8
    bl copy_words

    @ Maps of 16-bit entries: base, byte offset of the band in a row, row stride.
    ldr r0, =BG_VRAM_A + 0x800
    mov r1, #0
    mov r2, #64
    bl fill_map
    ldr r0, =BG_VRAM_A + 0x1000
    mov r1, #16
    mov r2, #64
    bl fill_map
    ldr r0, =BG_VRAM_A + 0x1800
    mov r1, #32
    mov r2, #64
    bl fill_map
    ldr r0, =BG_VRAM_A + 0x2000
    mov r1, #48
    mov r2, #64
    bl fill_map
    ldr r0, =BG_VRAM_B + 0x800
    mov r1, #0
    mov r2, #64
    bl fill_map
    ldr r0, =BG_VRAM_B + 0x1000
    mov r1, #16
    mov r2, #64
    bl fill_map
    ldr r0, =BG_VRAM_B + 0x2000
    mov r1, #48
    mov r2, #64
    bl fill_map
    @ Engine B's BG2: an affine map of one-byte entries (32 cells a row), tile 1 in columns 16-23.
    ldr r0, =BG_VRAM_B + 0x1800 + 16
    ldr r3, =0x01010101
    mov r4, #24
bg2_row:
    str r3, [r0]
    str r3, [r0, #4]
    add r0, r0, #32
    subs r4, r4, #1
    bne bg2_row

    @ BGxCNT of both engines, the affine parameters, DISPCNT.
    ldr r0, =BGCNT_A
    ldr r1, =0x22800180
    str r1, [r0]
    ldr r1, =0x44000304
    str r1, [r0, #4]
    ldr r0, =BGCNT_B
    ldr r1, =0x22800180
    str r1, [r0]
    ldr r1, =0x44004300
    str r1, [r0, #4]
    mov r2, #0x100
    ldr r0, =AFFINE_A
    strh r2, [r0]
    strh r2, [r0, #6]
    strh r2, [r0, #0x10]
    strh r2, [r0, #0x16]
    ldr r0, =AFFINE_B
    strh r2, [r0]
    strh r2, [r0, #6]
    strh r2, [r0, #0x10]
    strh r2, [r0, #0x16]
    ldr r0, =DISPCNT_A
    ldr r1, =0x40010F03
    str r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x40010F04
    str r1, [r0]

    @ The split.
    ldr r6, =VCOUNT
frame_loop:
wait_95:
    ldrh r0, [r6]
    cmp r0, #95
    blo wait_95
    ldr r0, =VRAMCNT_E
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =VRAMCNT_F
    mov r1, #0x84
    strb r1, [r0]
    ldr r0, =VRAMCNT_G
    mov r1, #0x8C
    strb r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00010F04
    str r1, [r0]
wait_200:
    ldrh r0, [r6]
    cmp r0, #200
    blo wait_200
    ldr r0, =VRAMCNT_E
    mov r1, #0x84
    strb r1, [r0]
    ldr r0, =VRAMCNT_F
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =VRAMCNT_G
    strb r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x40010F04
    str r1, [r0]
wait_wrap:
    ldrh r0, [r6]
    cmp r0, #95
    bhs wait_wrap
    b frame_loop

    @ fill_ext: writes `r1` slots (8 KiB each) of extended palette colours from `r0`; the first slot
    @ is numbered `r2` and K is `r3` (see the top of the file).
fill_ext:
    ldr r9, =0x7FFF
    ldr r10, =0x1F1
    ldr r11, =0xB3
fe_slot:
    mul r4, r2, r10
    add r4, r4, r3
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
    add r2, r2, #1
    subs r1, r1, #1
    bne fe_slot
    bx lr

    @ copy_words: copies `r2` words from `r1` to `r0`.
copy_words:
    ldr r3, [r1], #4
    str r3, [r0], #4
    subs r2, r2, #1
    bne copy_words
    bx lr

    @ fill_map: rows 0-23 of a map of 16-bit entries at `r0` with a row stride of `r2` bytes; each
    @ row writes 8 cells from byte `r1` of the row: tile 1 with palette number row & 15.
fill_map:
    mov r4, #0
fm_row:
    and r5, r4, #15
    mov r5, r5, lsl #12
    orr r5, r5, #1
    orr r5, r5, r5, lsl #16
    mla r7, r4, r2, r0
    add r7, r7, r1
    str r5, [r7], #4
    str r5, [r7], #4
    str r5, [r7], #4
    str r5, [r7], #4
    add r4, r4, #1
    cmp r4, #24
    bne fm_row
    bx lr

    .ltorg

    .balign 4
    @ Tile 1 of 8 bits per pixel: row r column c = 1 + ((3r + c) & 7).
tile8:
    .set r, 0
    .rept 8
    .set lo, 0
    .set hi, 0
    .set c, 0
    .rept 4
    .set lo, lo | ((1 + ((3 * r + c) & 7)) << (8 * c))
    .set hi, hi | ((1 + ((3 * r + c + 4) & 7)) << (8 * c))
    .set c, c + 1
    .endr
    .word lo, hi
    .set r, r + 1
    .endr
    @ Tile 1 of 4 bits per pixel with the same values.
tile4:
    .set r, 0
    .rept 8
    .set w, 0
    .set c, 0
    .rept 8
    .set w, w | ((1 + ((3 * r + c) & 7)) << (4 * c))
    .set c, c + 1
    .endr
    .word w
    .set r, r + 1
    .endr
