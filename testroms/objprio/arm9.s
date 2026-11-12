@ SPDX-License-Identifier: Apache-2.0
@ Object-priority ROM, ARM9 side: shows 8x8 objects of different priorities in front of and
@ behind text backgrounds, and in front of and behind each other, on both screens
@ (docs/hw/obj.md, "Priority"). No libnds, no crt0, no image asset. Loaded at 0x02000000, entered
@ at 0x02000100. The expected pictures are worked out from the note in
@ crates/runner/tests/objprio.rs, which holds its own copy of the object tables below.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the upper screen, engine B on); VRAMCNT_A = 0x82 (bank A,
@ MST 2: engine A OBJ VRAM), VRAMCNT_B = 0x81 (bank B, MST 1: engine A BG VRAM at 0x06000000),
@ VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM at 0x06200000), VRAMCNT_D = 0x84 (bank D,
@ MST 4: engine B OBJ VRAM). DISPCNT of engine A = 0x00011300 (display mode 1, BG0, BG1 and
@ sprites on, 2D mapping), of engine B = 0x00011100 (BG0 and sprites on).
@
@ Palettes: entry i of the BG palette and of the OBJ palette of both engines = R i&31, G (3i)&31,
@ B (5i)&31 as a 15-bit colour, for i = 0..255. The backdrop (BG palette entry 0) is 0x03E0
@ (green) on engine A and 0x7C00 (blue) on engine B.
@
@ Tile data, in the OBJ VRAMs: unit u (32 bytes, 4 bits per pixel) for u = 0..8 has every pixel
@ equal to u (unit 0 is transparent); unit 9 is a checker, pixel (r, c) = 9 when r + c is odd and
@ 0 otherwise. In the BG VRAMs, tile 1 is unit 5 and tile 2 is unit 6 (character block 0).
@
@ Backgrounds. Engine A: BG0 (priority 2, screen block 1, character block 0) is tile 1 (value 5)
@ in every cell; BG1 (priority 1, screen block 2) is tile 2 (value 6) in cells 8..15 of rows
@ 0..23, that is columns 64..127 of the screen. Engine B: BG0 (priority 1, screen block 1) is tile
@ 1 in every cell.
@
@ OAM: all 256 entries of both engines are first disabled (attr0 bit 9); the entries listed at
@ the end of the file are written from entry 0.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ BGCNT_A, 0x04000008
    .equ BGCNT_B, 0x04001008
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_B, 0x04000241
    .equ VRAMCNT_C, 0x04000242
    .equ VRAMCNT_D, 0x04000243
    .equ BG_PALETTE_A, 0x05000000
    .equ BG_PALETTE_B, 0x05000400
    .equ OBJ_PALETTE_A, 0x05000200
    .equ OBJ_PALETTE_B, 0x05000600
    .equ BG_VRAM_A, 0x06000000
    .equ BG_VRAM_B, 0x06200000
    .equ OBJ_VRAM_A, 0x06400000
    .equ OBJ_VRAM_B, 0x06600000
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
    ldr r0, =VRAMCNT_B
    mov r1, #0x81
    strb r1, [r0]
    ldr r0, =VRAMCNT_C
    mov r1, #0x84
    strb r1, [r0]
    ldr r0, =VRAMCNT_D
    mov r1, #0x84
    strb r1, [r0]

    @ The four palettes, then the backdrops.
    ldr r8, =BG_PALETTE_A
    ldr r9, =OBJ_PALETTE_A
    ldr r10, =BG_PALETTE_B
    ldr r11, =OBJ_PALETTE_B
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
    strh r5, [r10, r7]
    strh r5, [r11, r7]
    add r4, r4, #1
    cmp r4, #256
    bne pal_loop
    ldr r1, =0x03E0
    strh r1, [r8]
    ldr r1, =0x7C00
    strh r1, [r10]

    @ Every entry of both OAM halves disabled.
    ldr r0, =OAM_A
    mov r1, #0x200
    mov r2, #0
    mov r4, #256
oam_off:
    str r1, [r0], #4
    str r2, [r0], #4
    subs r4, r4, #1
    bne oam_off

    @ Tile data, entry tables and the first screen block of each BG: records of (destination,
    @ source, words), ended by a zero destination.
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

    @ Screen blocks: records of (destination, words, value), ended by a zero destination.
    adr r9, fills
fill_next:
    ldr r0, [r9], #4
    cmp r0, #0
    beq fills_done
    ldr r2, [r9], #4
    ldr r3, [r9], #4
fill_word:
    str r3, [r0], #4
    subs r2, r2, #1
    bne fill_word
    b fill_next
fills_done:

    @ BG1's bars: rows 0..23 of screen block 2, cells 8..15 (4 words) from byte 16 of each row.
    ldr r0, =BG_VRAM_A + 0x1000 + 16
    ldr r3, =0x00020002
    mov r4, #24
bar_row:
    str r3, [r0]
    str r3, [r0, #4]
    str r3, [r0, #8]
    str r3, [r0, #12]
    add r0, r0, #64
    subs r4, r4, #1
    bne bar_row

    ldr r0, =BGCNT_A
    ldr r1, =0x02010102        @ BG1CNT: priority 1, screen block 2; BG0CNT: priority 2, block 1
    str r1, [r0]
    ldr r0, =BGCNT_B
    ldr r1, =0x00000101        @ BG0CNT: priority 1, screen block 1
    str r1, [r0]
    ldr r0, =DISPCNT_A
    ldr r1, =0x00011300
    str r1, [r0]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00011100
    str r1, [r0]
spin:
    b spin

    .ltorg

    .balign 4
copies:
    .word OBJ_VRAM_A, tiles, 80
    .word OBJ_VRAM_B, tiles, 80
    .word BG_VRAM_A + 32, tiles + 5 * 32, 16
    .word BG_VRAM_B + 32, tiles + 5 * 32, 16
    .word OAM_A, oam_a, (oam_a_end - oam_a) / 4
    .word OAM_B, oam_b, (oam_b_end - oam_b) / 4
    .word 0
fills:
    @ BG0 of engine A, screen block 1 at +0x800; BG0 of engine B, screen block 1 at +0x800.
    .word BG_VRAM_A + 0x800, 512, 0x00010001
    .word BG_VRAM_B + 0x800, 512, 0x00010001
    .word 0

    @ One OAM entry: Y, X, tile number (an 8x8 object, 16 colours, palette 0, tile n at n * 32 in
    @ 2D mapping) and priority. The fourth halfword is 0.
    .macro obj y, x, tile, prio
    .word (\y) | (((\x) & 0x1FF) << 16), (\tile) | ((\prio) << 10)
    .endm

    @ Engine A. Entries 0-3: priorities 0-3, tiles 1-4, over BG0 alone (x 8, 22, 36, 50, y 16).
    @ Entries 4-7: the same over BG1's bars (x 72, 84, 96, 108). Entries 8-15: four pairs at
    @ y 64, the second object 4 right and 4 down of the first, (priority, tile) of (first,
    @ second): (0,1)(0,2) at x 16; (1,3)(0,4) at x 48; (0,5)(1,6) at x 80; (0,9)(0,7) at x 112
    @ (tile 9 is the checker). Entries 16-18: tiles 1, 2, 3 of priorities 2, 1, 1 at the same
    @ place, (16, 100). Entries 19-21: tile 9 priority 1 at (60, 100), tile 8 priority 1 at the
    @ same place, tile 3 priority 0 at (64, 104). Entries 22-23: the checker, priority 0, over
    @ BG1 at (100, 140) and over BG0 at (140, 140).
    .balign 4
oam_a:
    obj 16, 8, 1, 0
    obj 16, 22, 2, 1
    obj 16, 36, 3, 2
    obj 16, 50, 4, 3
    obj 16, 72, 1, 0
    obj 16, 84, 2, 1
    obj 16, 96, 3, 2
    obj 16, 108, 4, 3
    obj 64, 16, 1, 0
    obj 68, 20, 2, 0
    obj 64, 48, 3, 1
    obj 68, 52, 4, 0
    obj 64, 80, 5, 0
    obj 68, 84, 6, 1
    obj 64, 112, 9, 0
    obj 68, 116, 7, 0
    obj 100, 16, 1, 2
    obj 100, 16, 2, 1
    obj 100, 16, 3, 1
    obj 100, 60, 9, 1
    obj 100, 60, 8, 1
    obj 104, 64, 3, 0
    obj 140, 100, 9, 0
    obj 140, 140, 9, 0
oam_a_end:

    @ Engine B (BG0 of priority 1 behind everything). Entries 0-3: priorities 0-3, tiles 1-4 at
    @ x 8, 22, 36, 50, y 16. Entries 4-7: the pairs of engine A at y 64 (x 16, 48, 80, 112).
    @ Entries 8-10: tiles 1, 2, 3 of priorities 2, 1, 1 at (16, 100).
    .balign 4
oam_b:
    obj 16, 8, 1, 0
    obj 16, 22, 2, 1
    obj 16, 36, 3, 2
    obj 16, 50, 4, 3
    obj 64, 16, 1, 0
    obj 68, 20, 2, 0
    obj 64, 48, 3, 1
    obj 68, 52, 4, 0
    obj 64, 80, 5, 0
    obj 68, 84, 6, 1
    obj 64, 112, 9, 0
    obj 68, 116, 7, 0
    obj 100, 16, 1, 2
    obj 100, 16, 2, 1
    obj 100, 16, 3, 1
oam_b_end:

    @ Tile units 0..9 (see the top of the file).
    .macro solid v
    .rept 8
    .word (\v) * 0x11111111
    .endr
    .endm
    .balign 4
tiles:
    solid 0
    solid 1
    solid 2
    solid 3
    solid 4
    solid 5
    solid 6
    solid 7
    solid 8
    .rept 4
    .word 0x90909090, 0x09090909
    .endr
