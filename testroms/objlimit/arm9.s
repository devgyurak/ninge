@ SPDX-License-Identifier: Apache-2.0
@ Object-limit ROM, ARM9 side: crowds a line with objects that are off the screen but cost
@ rendering cycles, so that the visible objects behind them in OAM are dropped
@ (docs/hw/obj.md, "The line limit"). Engine A (the upper screen) has the default budget of 1210
@ cycles a line, engine B (the lower screen) 954 (DISPCNT bit 23). No libnds, no crt0, no image
@ asset: every byte is computed here. Loaded at 0x02000000, entered at 0x02000100. The expected
@ pictures are worked out from the note in crates/runner/tests/objlimit.rs, which holds its own
@ copy of the object tables below.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the upper screen, engine B on), VRAMCNT_A = 0x82 (bank A,
@ MST 2: engine A OBJ VRAM), VRAMCNT_D = 0x84 (bank D, MST 4: engine B OBJ VRAM), DISPCNT of
@ engine A = 0x00011010 (display mode 1, sprites on, 1D mapping, boundary 32 bytes), of engine B =
@ 0x00811000 (display mode 1, sprites on, 2D mapping, bit 23: the budget is 954).
@
@ Palettes: OBJ palette entry i of both engines = R i&31, G (3i)&31, B (5i)&31 as a 15-bit
@ colour, for i = 0..255. The backdrop (BG palette entry 0) is 0x03E0 (green) on engine A and
@ 0x7C00 (blue) on engine B.
@
@ Tile data: the first 8 KiB of each OBJ VRAM holds 256 units of 32 bytes. In unit u, byte j of
@ row r (4 bytes a row) holds the 4-bit pixels c = 2j (low nibble) and 2j+1 (high nibble) with the
@ value (5u + 3r + c) & 15, so a pixel value of 0 appears in every tile.
@
@ OAM: all 256 entries of both engines are first disabled (attr0 bit 9); then the entries listed
@ at the end of this file are written to entry 0 and up, and affine group 0 (the identity) to the
@ fourth halfword of entries 0-3, with halfword stores.
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
    ldr r1, =0x00811000
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

    @ Engine A, 1D mapping, boundary 32 bytes (a tile number counts 32 bytes), budget 1210.
    @ Entries 0-11: 64x32 plain objects at (60, -256), entirely off the screen: 64 cycles each,
    @ 768. Entries 12-14: 32x32 affine objects with double-size at (60, -256), a 64x64 box: 138
    @ cycles each, 414; 1182 in all, 28 left. Entries 15 and 16: 8x32 objects at (40, 10) and
    @ (40, 24), tiles 1 and 5: 8 cycles each, 12 left. Entry 17: a 64x32 object at (40, 40),
    @ tile 20: 64 cycles, does not fit, so it and everything after it is dropped. Entries 18-20:
    @ 8x32 objects at x = 120, 134, 148 (tiles 30, 34, 38), dropped though 8 cycles would fit. On
    @ lines 40-59 the off-screen objects are not on the line and cost nothing, so all of entries
    @ 15-20 show; on lines 60-71 only entries 15 and 16 do.
    .balign 4
oam_a:
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, AFF | DBL, 0 << GRP | SIZE2, 0
    obj 60, -256, 0, AFF | DBL, 0 << GRP | SIZE2, 0
    obj 60, -256, 0, AFF | DBL, 0 << GRP | SIZE2, 0
    obj 40, 10, 1, SHAPE_TALL, SIZE1, 1 << 12
    obj 40, 24, 5, SHAPE_TALL, SIZE1, 2 << 12
    obj 40, 40, 20, SHAPE_WIDE, SIZE3, 3 << 12
    obj 40, 120, 30, SHAPE_TALL, SIZE1, 4 << 12
    obj 40, 134, 34, SHAPE_TALL, SIZE1, 5 << 12
    obj 40, 148, 38, SHAPE_TALL, SIZE1, 6 << 12
oam_a_end:

    @ Engine B, 2D mapping, budget 954. Entries 0-13: 64x32 plain objects at (60, -256): 64 cycles
    @ each, 896, 58 left. Entries 14-22: 8x32 objects at y = 40, x = 10 + 14k, tile k + 1 (k =
    @ 0..8), 8 cycles each: on lines 60-71 the first seven (56 cycles) show and the last two are
    @ dropped; on lines 40-59 all nine show.
    .balign 4
oam_b:
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 60, -256, 0, SHAPE_WIDE, SIZE3, 0
    obj 40, 10, 1, SHAPE_TALL, SIZE1, 1 << 12
    obj 40, 24, 2, SHAPE_TALL, SIZE1, 2 << 12
    obj 40, 38, 3, SHAPE_TALL, SIZE1, 3 << 12
    obj 40, 52, 4, SHAPE_TALL, SIZE1, 4 << 12
    obj 40, 66, 5, SHAPE_TALL, SIZE1, 5 << 12
    obj 40, 80, 6, SHAPE_TALL, SIZE1, 6 << 12
    obj 40, 94, 7, SHAPE_TALL, SIZE1, 7 << 12
    obj 40, 108, 8, SHAPE_TALL, SIZE1, 8 << 12
    obj 40, 122, 9, SHAPE_TALL, SIZE1, 9 << 12
oam_b_end:

    @ Parameter groups: (group, PA, PB, PC, PD), 8.8 fixed point, then 0xFFFF. Group 0 is the
    @ identity.
    .balign 4
groups_a:
    .hword 0, 256, 0, 0, 256
    .hword 0xFFFF
    .balign 4
groups_b:
    .hword 0, 256, 0, 0, 256
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
