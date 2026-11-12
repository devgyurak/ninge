@ SPDX-License-Identifier: Apache-2.0
@ Extended-background ROM, ARM9 side: shows the three extended layer kinds (docs/hw/bg-modes.md,
@ "Extended layers"): engine A (the upper screen) in BG mode 5 with a direct-colour bitmap (BG2)
@ in front of a 256-colour bitmap (BG3), engine B (the lower screen) in BG mode 3 with a tile
@ layer with 16-bit map entries (BG3). No libnds, no crt0, no image asset: every byte is computed
@ here. Loaded at 0x02000000, entered at 0x02000100. The expected picture is worked out from the
@ note in crates/runner/tests/bgext.rs, which holds its own copy of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A = 0x81
@ (bank A, MST 1: engine A BG VRAM, 0x06000000, 128 KiB: the 256-colour bitmap is 64 KiB and the
@ direct-colour one 32 KiB), VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM, 0x06200000).
@ DISPCNT of engine A = 0x00010C05 (display mode 1, BG mode 5, BG2 and BG3 on), engine B =
@ 0x00010803 (display mode 1, BG mode 3, BG3 on). Every VRAM offset counts from the start of the
@ engine's window.
@
@ Palette (both engines' BG palette, 0x05000000 and 0x05000400): entry i = R i&31, G (3i)&31,
@ B (5i)&31 as a 15-bit colour, for i = 0..255; then entry 0 (the backdrop) = 0x03E0, green, on
@ engine A and 0x7C00, blue, on engine B.
@
@ Layers (BGxCNT; PA, PB, PC, PD; BGxX, BGxY as signed 20.8):
@   A BG2  0x0484  priority 0, direct-colour bitmap (bits 7 and 2), size 0 (128x128), wrap off,
@          screen block 4 (0x10000). PA 0x0100, PB 0, PC 0, PD 0x0100, X -32.0, Y -16.0.
@          Pixel (tx, ty) = (37 tx + 91 ty) & 0x7FFF, with bit 15 set unless (tx + ty) mod 3 = 0.
@   A BG3  0x6081  priority 1, 256-colour bitmap (bit 7), size 1 (256x256), wrap on, screen block
@          0. PA 0x00F0, PB 0x0040, PC 0xFFC0, PD 0x00F0, X 0x00002000, Y 0.
@          Pixel (tx, ty) = 1 + ((5 tx + 3 ty) mod 255).
@   B BG3  0x6014  priority 0, tile layer with 16-bit entries (bit 7 clear; bit 2 set is the
@          character base, block 5 = 0x14000), size 1 (256x256), wrap on, screen block 0.
@          PA 0x00C0, PB 0xFFD0, PC 0x0030, PD 0x00C0, X 0x00001000, Y 0x00000800.
@          Tiles 1-4: row r, column c of tile t holds 16 t + ((3 r + c) mod 16). Cell (cx, cy)
@          holds tile 1 + ((cx + 2 cy) mod 4), flipped horizontally if cx is odd and vertically
@          if cy is odd, palette number 0.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_C, 0x04000242
    .equ BG2CNT_A, 0x0400000C
    .equ AFFINE_A, 0x04000020
    .equ BG3CNT_B, 0x0400100E
    .equ AFFINE_B, 0x04001030
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

    @ Everything is made in main RAM (a byte store to VRAM is dropped) and copied by words.
    @ The 256-colour bitmap, 64 KiB at BUF: byte = 1 + ((5 tx + 3 ty) mod 255), with v the running
    @ (5 tx + 3 ty) mod 255: the row starts at (3 ty) mod 255 and each pixel adds 5.
    ldr r0, =BUF
    mov r2, #0                   @ (3 ty) mod 255
    mov r1, #0                   @ ty
bmp_row:
    mov r3, r2                   @ v
    mov r4, #0                   @ tx
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

    @ The direct-colour bitmap, 32 KiB at BUF + 0x10000: halfword = (37 tx + 91 ty) & 0x7FFF, bit
    @ 15 set unless (tx + ty) mod 3 = 0. r6 is (tx + ty) mod 3.
    ldr r0, =BUF + 0x10000
    mov r1, #0                   @ ty
    ldr r7, =0x7FFF
dir_row:
    mov r4, #0                   @ tx
    mov r6, #0
    mov r5, r1
dir_mod:                         @ r6 = ty mod 3
    cmp r5, #3
    blo dir_mod_done
    sub r5, r5, #3
    b dir_mod
dir_mod_done:
    mov r6, r5
dir_pixel:
    mov r2, #37
    mul r3, r4, r2               @ 37 tx
    mov r2, #91
    mla r3, r1, r2, r3           @ + 91 ty
    and r3, r3, r7
    cmp r6, #0
    orrne r3, r3, #0x8000
    strh r3, [r0], #2
    add r6, r6, #1
    cmp r6, #3
    moveq r6, #0
    add r4, r4, #1
    cmp r4, #128
    bne dir_pixel
    add r1, r1, #1
    cmp r1, #128
    bne dir_row

    @ Tiles 1-4 at BUF + 0x20000 (320 bytes, tile 0 stays zero).
    ldr r0, =BUF + 0x20000
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

    @ The 32x32 map of 16-bit entries at BUF + 0x21000 (2 KiB).
    ldr r0, =BUF + 0x21000
    mov r2, #0                   @ cy
map_row:
    mov r3, #0                   @ cx
map_cell:
    add r4, r3, r2, lsl #1
    and r4, r4, #3
    add r4, r4, #1               @ tile
    tst r3, #1
    orrne r4, r4, #0x0400        @ horizontal flip
    tst r2, #1
    orrne r4, r4, #0x0800        @ vertical flip
    strh r4, [r0], #2
    add r3, r3, #1
    cmp r3, #32
    bne map_cell
    add r2, r2, #1
    cmp r2, #32
    bne map_row

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

    @ Registers. Engine A: BG2CNT and BG3CNT, then BG2's and BG3's transform.
    ldr r0, =BG2CNT_A
    ldr r1, =0x60810484
    str r1, [r0]
    ldr r0, =AFFINE_A
    ldr r1, =0x00000100          @ BG2 PB:PA
    str r1, [r0]
    ldr r1, =0x01000000          @ BG2 PD:PC
    str r1, [r0, #4]
    ldr r1, =0xFFFFE000          @ BG2X
    str r1, [r0, #8]
    ldr r1, =0xFFFFF000          @ BG2Y
    str r1, [r0, #12]
    ldr r1, =0x004000F0          @ BG3 PB:PA
    str r1, [r0, #16]
    ldr r1, =0x00F0FFC0          @ BG3 PD:PC
    str r1, [r0, #20]
    ldr r1, =0x00002000          @ BG3X
    str r1, [r0, #24]
    mov r1, #0                   @ BG3Y
    str r1, [r0, #28]
    @ Engine B: BG3CNT and BG3's transform.
    ldr r0, =BG3CNT_B
    ldr r1, =0x6014
    strh r1, [r0]
    ldr r0, =AFFINE_B
    ldr r1, =0xFFD000C0          @ PB:PA
    str r1, [r0]
    ldr r1, =0x00C00030          @ PD:PC
    str r1, [r0, #4]
    ldr r1, =0x00001000          @ BG3X
    str r1, [r0, #8]
    ldr r1, =0x00000800          @ BG3Y
    str r1, [r0, #12]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00010803
    str r1, [r0]
    ldr r0, =DISPCNT_A
    ldr r1, =0x00010C05
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
    .word VRAM_A + 0x00000, BUF, 16384                   @ A BG3: the 256-colour bitmap (64 KiB)
    .word VRAM_A + 0x10000, BUF + 0x10000, 8192          @ A BG2: the direct-colour bitmap (32 KiB)
    .word VRAM_B + 0x14000, BUF + 0x20000, 80            @ B BG3: tiles
    .word VRAM_B + 0x00000, BUF + 0x21000, 512           @ B BG3: the map (2 KiB)
    .word 0
