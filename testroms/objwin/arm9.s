@ SPDX-License-Identifier: Apache-2.0
@ OBJ window ROM, ARM9 side: shows the OBJ window on both engines (docs/hw/windows.md, "The OBJ
@ window"): the four solid text layers of the `win` ROM (BG0 red in front to BG3 yellow behind, over a
@ white backdrop) and objects of mode 2, whose opaque pixels choose which layers a pixel may show. No
@ libnds, no crt0, no image asset: every byte is computed here or listed below. Loaded at 0x02000000,
@ entered at 0x02000100. The expected picture is worked out from the note in
@ crates/runner/tests/objwin.rs, which holds its own copy of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A = 0x81 (bank
@ A, MST 1: engine A BG VRAM), VRAMCNT_B = 0x82 (bank B, MST 2: engine A OBJ VRAM, 0x06400000),
@ VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM) and VRAMCNT_D = 0x84 (bank D, MST 4: engine B OBJ
@ VRAM, 0x06600000). Both engines are set up alike by setup_engine with their own registers and
@ objects. The background layers, the palette and the layer registers are those of the `win` ROM;
@ the OBJ palette (0x05000200 and 0x05000600) has entry 1 = 0x7C1F, magenta.
@
@ Object tiles (4 bits per pixel, 2D mapping): the tiles 32 r + c for r and c in 0-3 are solid 1, and
@ tile 8 has pixel value 1 in its left four columns and 0 (transparent) in the right four.
@
@ Engine A: DISPCNT 0x0001DF00 (display mode 1, BG0-BG3 and OBJ on, window 1 and the OBJ window on).
@   WIN1H 0x78B4 (columns 120-179), WIN1V 0x1450 (rows 20-79), WININ 0x0800 (inside window 1 BG3
@   only), WINOUT 0x0211 (outside BG0 and OBJ, inside the OBJ window BG1 only). Window 0 is off.
@   Entries 0-3 are disabled objects holding affine group 0: PA = PD = 0x200 (2.0), PB = PC = 0.
@   Entry 4  mode 2, 32x32, at (20, 20).
@   Entry 5  mode 2, affine, 32x32 box, at (80, 40): group 0 shows the middle 16 by 16.
@   Entry 6  mode 0, 16x16, at (40, 30): overlaps entry 4's mask.
@   Entry 7  mode 2, 32x32, at (150, 50): overlaps window 1's right edge and bottom edge.
@   Entry 8  mode 2, 8x8, tile 8 (half transparent), at (200, 100).
@   Entry 9  mode 0, 8x8, at (202, 102): overlaps entry 8's mask.
@ Engine B: DISPCNT 0x00019F00 (display mode 1, BG0-BG3 and OBJ on, the OBJ window on), WINOUT 0x0401
@   (outside BG0, inside the OBJ window BG2), WININ 0, MOSAIC 0x0300 (OBJ blocks 4 wide, 1 high).
@   Entry 0  mode 2, 32x32, at (100, 100).
@   Entry 1  mode 2, mosaic, 16x16, at (30, 150).
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ MOSAIC_B, 0x0400104C
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_B, 0x04000241
    .equ VRAMCNT_C, 0x04000242
    .equ VRAMCNT_D, 0x04000243
    .equ PAL_A, 0x05000000
    .equ PAL_B, 0x05000400
    .equ VRAM_A, 0x06000000
    .equ VRAM_B, 0x06200000
    .equ OBJ_A, 0x06400000
    .equ OBJ_B, 0x06600000
    .equ OAM_A, 0x07000000
    .equ OAM_B, 0x07000400
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
    ldr r0, =VRAMCNT_B
    mov r1, #0x82
    strb r1, [r0]
    ldr r0, =VRAMCNT_C
    mov r1, #0x84
    strb r1, [r0]
    ldr r0, =VRAMCNT_D
    mov r1, #0x84
    strb r1, [r0]

    ldr r8, =PAL_A
    ldr r9, =VRAM_A
    ldr r10, =DISPCNT_A
    adr r11, regs_a
    bl setup_engine
    ldr r8, =PAL_B
    ldr r9, =VRAM_B
    ldr r10, =DISPCNT_B
    adr r11, regs_b
    bl setup_engine
    ldr r0, =MOSAIC_B
    ldr r1, =0x0300
    strh r1, [r0]
done:
    b done

    .ltorg

    @ setup_engine: r8 = palette base, r9 = BG VRAM base, r10 = the engine's DISPCNT, r11 = seven
    @ words: the registers at +0x40 (WIN0H, WIN1H), +0x44 (WIN0V, WIN1V), +0x48 (WININ, WINOUT),
    @ DISPCNT, the OAM base, the OBJ VRAM base and the address of the object records.
setup_engine:
    add r7, r8, #0x200           @ the OBJ palette
    @ The five palette entries.
    adr r0, palette_data
    mov r1, #5
pal_loop:
    ldrh r2, [r0], #2
    strh r2, [r8], #2
    subs r1, r1, #1
    bne pal_loop

    @ Tile t = 1-4 at 0x4000 + 32 t: eight words of t * 0x11111111.
    ldr r5, =0x11111111
    mov r4, #1
tile_next:
    mul r6, r5, r4
    add r0, r9, #0x4000
    add r0, r0, r4, lsl #5
    mov r1, #8
tile_word:
    str r6, [r0], #4
    subs r1, r1, #1
    bne tile_word
    add r4, r4, #1
    cmp r4, #5
    bne tile_next

    @ The map of BGn, at screen block n: 512 words holding tile n + 1 twice.
    mov r4, #1
map_next:
    orr r6, r4, r4, lsl #16
    sub r0, r4, #1
    add r0, r9, r0, lsl #11
    mov r1, #512
map_word:
    str r6, [r0], #4
    subs r1, r1, #1
    bne map_word
    add r4, r4, #1
    cmp r4, #5
    bne map_next

    @ BGnCNT at +8 + 2n: character block 1 (bits 2-5), screen block n (bits 8-12).
    mov r4, #0
cnt_next:
    mov r6, r4, lsl #8
    orr r6, r6, #4
    add r0, r10, #8
    add r0, r0, r4, lsl #1
    strh r6, [r0]
    add r4, r4, #1
    cmp r4, #4
    bne cnt_next

    @ OBJ palette entry 1: magenta.
    ldr r0, =0x7C1F
    strh r0, [r7, #2]

    @ The solid object tiles 32 r + c (r and c in 0-3), eight words of 0x11111111 each.
    ldr r12, [r11, #20]
    ldr r5, =0x11111111
    mov r4, #0
obj_row:
    mov r3, #0
obj_col:
    add r0, r3, r4, lsl #5
    add r0, r12, r0, lsl #5
    mov r1, #8
obj_word:
    str r5, [r0], #4
    subs r1, r1, #1
    bne obj_word
    add r3, r3, #1
    cmp r3, #4
    bne obj_col
    add r4, r4, #1
    cmp r4, #4
    bne obj_row
    @ Tile 8: pixel value 1 in the left four columns (the low nibbles), 0 in the right four.
    add r0, r12, #8 * 32
    ldr r5, =0x00001111
    mov r1, #8
half_word:
    str r5, [r0], #4
    subs r1, r1, #1
    bne half_word

    @ Every OAM entry disabled, then the listed entries: a count, then four halfwords each.
    ldr r0, [r11, #16]
    mov r2, #0x200
    mov r1, #128
oam_off:
    strh r2, [r0], #8
    subs r1, r1, #1
    bne oam_off
    ldr r0, [r11, #16]
    ldr r1, [r11, #24]
    ldr r3, [r1], #4
oam_put:
    ldrh r2, [r1], #2
    strh r2, [r0], #2
    ldrh r2, [r1], #2
    strh r2, [r0], #2
    ldrh r2, [r1], #2
    strh r2, [r0], #2
    ldrh r2, [r1], #2
    strh r2, [r0], #2
    subs r3, r3, #1
    bne oam_put

    @ The window registers, then DISPCNT.
    ldmia r11, {r0-r3}
    str r0, [r10, #0x40]
    str r1, [r10, #0x44]
    str r2, [r10, #0x48]
    str r3, [r10]
    bx lr

    .balign 4
palette_data:
    .hword 0x7FFF, 0x001F, 0x03E0, 0x7C00, 0x03FF

    .balign 4
regs_a:
    .word 0x78B40000, 0x14500000, 0x02110800, 0x0001DF00, OAM_A, OBJ_A, objs_a
regs_b:
    .word 0x00000000, 0x00000000, 0x04010000, 0x00019F00, OAM_B, OBJ_B, objs_b

    @ Object records: the count, then attr0, attr1, attr2 and attr3 of each entry in turn.
objs_a:
    .word 10
    .hword 0x0200, 0, 0, 0x0200      @ entry 0, disabled; attr3 is PA
    .hword 0x0200, 0, 0, 0x0000      @ entry 1, disabled; attr3 is PB
    .hword 0x0200, 0, 0, 0x0000      @ entry 2, disabled; attr3 is PC
    .hword 0x0200, 0, 0, 0x0200      @ entry 3, disabled; attr3 is PD
    .hword 0x0814, 0x8014, 0, 0      @ entry 4: Y 20, mode 2, X 20, size 2 (32x32)
    .hword 0x0928, 0x8050, 0, 0      @ entry 5: Y 40, affine, mode 2, X 80, group 0, size 2
    .hword 0x001E, 0x4028, 0, 0      @ entry 6: Y 30, X 40, size 1 (16x16)
    .hword 0x0832, 0x8096, 0, 0      @ entry 7: Y 50, mode 2, X 150, size 2
    .hword 0x0864, 0x00C8, 8, 0      @ entry 8: Y 100, mode 2, X 200, size 0, tile 8
    .hword 0x0066, 0x00CA, 0, 0      @ entry 9: Y 102, X 202, size 0
objs_b:
    .word 2
    .hword 0x0864, 0x8064, 0, 0      @ entry 0: Y 100, mode 2, X 100, size 2
    .hword 0x1896, 0x401E, 0, 0      @ entry 1: Y 150, mode 2, mosaic, X 30, size 1
