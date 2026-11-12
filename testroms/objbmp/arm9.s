@ SPDX-License-Identifier: Apache-2.0
@ Bitmap-object ROM, ARM9 side: shows bitmap objects (attr0 mode 3, 15-bit direct colour, bit 15
@ set = opaque) on both screens (docs/hw/obj.md, "A bitmap object"). Engine A (the upper screen)
@ changes DISPCNT between line bands to show all three mappings in one frame; engine B (the lower
@ screen) uses 1D mapping with a 128-byte boundary. No libnds, no crt0, no image asset: every
@ byte is computed here. Loaded at 0x02000000, entered at 0x02000100. The expected pictures are
@ worked out from the note in crates/runner/tests/objbmp.rs, which holds its own copy of the
@ object tables below.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the upper screen, engine B on), VRAMCNT_A = 0x82 (bank A,
@ MST 2: engine A OBJ VRAM), VRAMCNT_D = 0x84 (bank D, MST 4: engine B OBJ VRAM). The backdrop
@ (BG palette entry 0) is 0x03E0 (green) on engine A and 0x7C00 (blue) on engine B.
@
@ Bitmap data: the first 64 KiB of each OBJ VRAM holds 32768 halfwords; halfword i is
@ ((i * 0x2F1) & 0x7FFF), with bit 15 set unless i is a multiple of 5 (so one pixel in five is
@ transparent).
@
@ DISPCNT of engine A (display mode 1, sprites on) by line band, written by polling VCOUNT (a
@ write made once VCOUNT reaches 63 or 127 takes effect from line 64 or 128, and the value for
@ line 0 is written again once VCOUNT reaches 200):
@   lines 0-63    0x00011020  bit 5: 2D mapping, source bitmap 256 wide
@   lines 64-127  0x00011000  2D mapping, source bitmap 128 wide
@   lines 128-191 0x00411040  bit 6: 1D mapping, bit 22: boundary 256 bytes
@ DISPCNT of engine B = 0x00011040 (1D mapping, boundary 128 bytes).
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
    .equ BMP, 3 << 10
    .equ VCOUNT, 0x04000006
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

    @ The backdrops.
    ldr r0, =BG_PALETTE_A
    ldr r1, =0x03E0
    strh r1, [r0]
    ldr r0, =BG_PALETTE_B
    ldr r1, =0x7C00
    strh r1, [r0]

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

    @ The bitmap data, to both OBJ VRAMs: halfword i is (i * 0x2F1) & 0x7FFF, plus 0x8000 unless
    @ i is a multiple of 5. r6 counts i modulo 5.
    ldr r0, =OBJ_VRAM_A
    ldr r1, =OBJ_VRAM_B
    ldr r4, =32768
    ldr r7, =0x2F1
    ldr r9, =0x7FFF
    mov r5, #0
    mov r6, #0
bitmap_fill:
    mul r8, r5, r7
    and r8, r8, r9
    cmp r6, #0
    orrne r8, r8, #0x8000
    strh r8, [r0], #2
    strh r8, [r1], #2
    add r5, r5, #1
    add r6, r6, #1
    cmp r6, #5
    moveq r6, #0
    cmp r5, r4
    bne bitmap_fill

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
    ldr r11, =OAM_A
    adr r9, groups_a
    bl write_groups
    ldr r11, =OAM_B
    adr r9, groups_b
    bl write_groups

    ldr r0, =DISPCNT_B
    ldr r1, =0x00011040
    str r1, [r0]
    ldr r10, =DISPCNT_A
    ldr r6, =VCOUNT
    ldr r1, =0x00011020
    str r1, [r10]

    @ The band changes. A write made after VCOUNT reads 63 lands after line 63 started, so line
    @ 63 still has the old DISPCNT; likewise at 127.
frame_loop:
wait_63:
    ldrh r0, [r6]
    cmp r0, #63
    blo wait_63
    ldr r1, =0x00011000
    str r1, [r10]
wait_127:
    ldrh r0, [r6]
    cmp r0, #127
    blo wait_127
    ldr r1, =0x00411040
    str r1, [r10]
wait_200:
    ldrh r0, [r6]
    cmp r0, #200
    blo wait_200
    ldr r1, =0x00011020
    str r1, [r10]
wait_wrap:
    ldrh r0, [r6]
    cmp r0, #63
    bhs wait_wrap
    b frame_loop

write_groups:
    ldrh r0, [r9], #2
    ldr r1, =0xFFFF
    cmp r0, r1
    bxeq lr
    add r2, r11, r0, lsl #5
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

    @ Engine A, by line band (see the top of the file). Entries 0-9:
    @   0  32x32            (10, 8)     tile 0x01   2D, 256 wide
    @   1  16x16, h flip    (60, 8)     tile 0x22
    @   2  64x32            (100, 16)   tile 0x40
    @   3  16x16 affine, group 2, double  (190, 4)  tile 0x05 (a 32x32 box)
    @   4  32x16            (10, 70)    tile 0x11   2D, 128 wide
    @   5  8x8, v flip      (60, 70)    tile 0x3F
    @   6  32x32            (100, 90)   tile 0x20
    @   7  32x32            (10, 130)   tile 3      1D, boundary 256
    @   8  16x32            (60, 130)   tile 40
    @   9  32x32 affine, group 3  (120, 140)  tile 80
    .balign 4
oam_a:
    obj 8, 10, 0x01, BMP, SIZE2, 0
    obj 8, 60, 0x22, BMP, HFLIP | SIZE1, 0
    obj 16, 100, 0x40, BMP | SHAPE_WIDE, SIZE3, 0
    obj 4, 190, 0x05, BMP | AFF | DBL, 2 << GRP | SIZE1, 0
    obj 70, 10, 0x11, BMP | SHAPE_WIDE, SIZE2, 0
    obj 70, 60, 0x3F, BMP, VFLIP, 0
    obj 90, 100, 0x20, BMP, SIZE2, 0
    obj 130, 10, 3, BMP, SIZE2, 0
    obj 130, 60, 40, BMP | SHAPE_TALL, SIZE2, 0
    obj 140, 120, 80, BMP | AFF, 3 << GRP | SIZE2, 0
oam_a_end:

    @ Engine B, 1D mapping, boundary 128 bytes. Entries 0-5:
    @   0  16x16            (10, 10)    tile 4
    @   1  64x64            (40, 20)    tile 20
    @   2  32x16, both flips (150, 20)  tile 100
    @   3  32x32 affine, group 1, double  (120, 100)  tile 120 (a 64x64 box)
    @   4  8x8, attr2 bits 12-15 set  (200, 150)  tile 7
    @   5  16x16, X = -8     (-8, 160)  tile 9
    .balign 4
oam_b:
    obj 10, 10, 4, BMP, SIZE1, 0
    obj 20, 40, 20, BMP, SIZE3, 0
    obj 20, 150, 100, BMP | SHAPE_WIDE, HFLIP | VFLIP | SIZE2, 0
    obj 100, 120, 120, BMP | AFF | DBL, 1 << GRP | SIZE2, 0
    obj 150, 200, 7, BMP, 0, 0xF << 12
    obj 160, -8, 9, BMP, SIZE1, 0
oam_b_end:

    @ Parameter groups: (group, PA, PB, PC, PD), 8.8 fixed point, then 0xFFFF.
    @   1: a quarter turn   2: about 30 degrees   3: magnify by 2
    .balign 4
groups_a:
    .hword 2, 222, 128, -128, 222
    .hword 3, 128, 0, 0, 128
    .hword 0xFFFF
    .balign 4
groups_b:
    .hword 1, 0, 256, -256, 0
    .hword 0xFFFF
