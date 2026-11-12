@ SPDX-License-Identifier: Apache-2.0
@ Affine-background ROM, ARM9 side: shows affine layers on both engines (docs/hw/bg-modes.md,
@ "Affine layers"): engine A (the upper screen) in BG mode 2 with BG2 and BG3 affine, engine B
@ (the lower screen) in BG mode 1 with BG3 affine and BG0 text. No libnds, no crt0, no image
@ asset: every byte is computed here. Loaded at 0x02000000, entered at 0x02000100. The expected
@ picture is worked out from the note in crates/runner/tests/bgaffine.rs, which holds its own copy
@ of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A = 0x81
@ (bank A, MST 1: engine A BG VRAM, 0x06000000), VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM,
@ 0x06200000). DISPCNT of engine A = 0x00010C02 (display mode 1, BG mode 2, BG2 and BG3 on),
@ engine B = 0x00010901 (display mode 1, BG mode 1, BG0 and BG3 on). Every VRAM offset counts from
@ the start of the engine's window (engine B has no DISPCNT bases, engine A's are 0).
@
@ Palette (both engines' BG palette, 0x05000000 and 0x05000400): entry i = R i&31, G (3i)&31,
@ B (5i)&31 as a 15-bit colour, for i = 0..255; then entry 0 (the backdrop) = 0x03E0, green, on
@ engine A and 0x7C00, blue, on engine B.
@
@ Tile data (character block 4, 0x10000, of both engines): tiles 1-4 of 64 bytes; row r, column c
@ of tile t holds 16 t + ((3 r + c) mod 16). Tile 0 is zero (transparent).
@ Affine maps (one byte a cell, row by row): cell (cx, cy) holds tile 1 + ((cx + 2 cy) mod 4).
@
@ Layers (BGxCNT; PA, PB, PC, PD; BGxX, BGxY as signed 20.8):
@   A BG2  0x6011  priority 1, size 1 (256x256), wrap on, char block 4, screen block 0 (0x0000).
@          PA 0x00E0, PB 0xFF80, PC 0x0080, PD 0x00E0, X 0x00004000 (64.0), Y 0xFFFFE000 (-32.0).
@   A BG3  0x0110  priority 0, size 0 (128x128), wrap off, char block 4, screen block 1 (0x0800).
@          PA 0x0100, PB 0, PC 0, PD 0x0100, X 0xFFFFC000 (-64.0), Y 0xFFFFE000 (-32.0). The ROM
@          writes X = 0xFFFFA000 (-96.0) once VCOUNT reaches 100 and X = 0xFFFFC000 again once it
@          reaches 200, so lines 101-191 of each frame start from the new X.
@   B BG3  0xA211  priority 1, size 2 (512x512), wrap on, char block 4, screen block 2 (0x1000).
@          PA 0x00C0, PB 0x0060, PC 0xFFA0, PD 0x00C0, X 0, Y 0x00010000 (256.0).
@   B BG0  0x0408  priority 0, text 4bpp, 256x256, char block 2 (0x8000), screen block 4 (0x2000).
@          Tile 1 (all pixels 9) at cells (2..4, 2) and (2..4, 3); cell (8, 2) is tile 1 with
@          palette number 3.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_C, 0x04000242
    .equ BG2CNT_A, 0x0400000C
    .equ AFFINE_A, 0x04000020
    .equ BG3X_A, 0x04000038
    .equ BG0CNT_B, 0x04001008
    .equ BG3CNT_B, 0x0400100E
    .equ AFFINE_B, 0x04001030
    .equ VCOUNT, 0x04000006
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

    @ Both palettes.
    ldr r8, =PAL_A
    ldr r9, =0x03E0
    bl make_palette
    ldr r8, =PAL_B
    ldr r9, =0x7C00
    bl make_palette

    @ Tile data and the three affine maps are made in main RAM (a byte store to VRAM is dropped)
    @ and copied to VRAM by words.
    @ Tiles: 320 bytes at BUF, tile 0 stays zero.
    ldr r0, =BUF
    mov r1, #64
tile_loop:
    mov r2, r1, lsr #6          @ tile t
    mov r3, r1, lsr #3
    and r3, r3, #7             @ row r
    and r4, r1, #7             @ column c
    add r3, r3, r3, lsl #1     @ 3r
    add r3, r3, r4
    and r3, r3, #15
    add r3, r3, r2, lsl #4     @ 16 t + ((3r + c) mod 16)
    strb r3, [r0, r1]
    add r1, r1, #1
    cmp r1, #320
    bne tile_loop

    ldr r0, =BUF + 0x1000      @ the 32x32 map
    mov r1, #32
    bl make_map
    ldr r0, =BUF + 0x2000      @ the 16x16 map
    mov r1, #16
    bl make_map
    ldr r0, =BUF + 0x3000      @ the 64x64 map
    mov r1, #64
    bl make_map

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

    @ Engine B's text layer: records of (map base, first cell, count, entry) as in the bgtext ROM.
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

    @ Registers. Engine A: BG2CNT and BG3CNT, then BG2's and BG3's transform.
    ldr r0, =BG2CNT_A
    ldr r1, =0x01106011
    str r1, [r0]
    ldr r0, =AFFINE_A
    ldr r1, =0xFF8000E0          @ PB:PA
    str r1, [r0]
    ldr r1, =0x00E00080          @ PD:PC
    str r1, [r0, #4]
    ldr r1, =0x00004000          @ BG2X
    str r1, [r0, #8]
    ldr r1, =0xFFFFE000          @ BG2Y
    str r1, [r0, #12]
    ldr r1, =0x00000100          @ BG3 PB:PA
    str r1, [r0, #16]
    ldr r1, =0x01000000          @ BG3 PD:PC
    str r1, [r0, #20]
    ldr r1, =0xFFFFC000          @ BG3X
    str r1, [r0, #24]
    ldr r1, =0xFFFFE000          @ BG3Y
    str r1, [r0, #28]
    @ Engine B: BG0CNT, BG3CNT and BG3's transform.
    ldr r0, =BG0CNT_B
    ldr r1, =0x0408
    strh r1, [r0]
    ldr r0, =BG3CNT_B
    ldr r1, =0xA211
    strh r1, [r0]
    ldr r0, =AFFINE_B
    ldr r1, =0x006000C0          @ PB:PA
    str r1, [r0]
    ldr r1, =0x00C0FFA0          @ PD:PC
    str r1, [r0, #4]
    mov r1, #0                   @ BG3X
    str r1, [r0, #8]
    ldr r1, =0x00010000          @ BG3Y
    str r1, [r0, #12]
    ldr r0, =DISPCNT_B
    ldr r1, =0x00010901
    str r1, [r0]
    ldr r0, =DISPCNT_A
    ldr r1, =0x00010C02
    str r1, [r0]

    @ The write in mid-frame: engine A's BG3X changes after VCOUNT reaches 100 and is restored
    @ after it reaches 200. A write made after VCOUNT reads 100 lands after line 100 started, so
    @ line 101 is the first to start from the new value.
    ldr r6, =VCOUNT
    ldr r7, =BG3X_A
    ldr r5, =0xFFFFA000
    ldr r4, =0xFFFFC000
frame_loop:
wait_100:
    ldrh r0, [r6]
    cmp r0, #100
    blo wait_100
    str r5, [r7]
wait_200:
    ldrh r0, [r6]
    cmp r0, #200
    blo wait_200
    str r4, [r7]
wait_wrap:
    ldrh r0, [r6]
    cmp r0, #100
    bhs wait_wrap
    b frame_loop

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

    @ make_map: r0 = destination, r1 = cells per side (a power of two). Cell (cx, cy) holds
    @ tile 1 + ((cx + 2 cy) mod 4).
make_map:
    mov r2, #0                   @ cy
map_row:
    mov r3, #0                   @ cx
map_cell:
    add r4, r3, r2, lsl #1
    and r4, r4, #3
    add r4, r4, #1
    strb r4, [r0], #1
    add r3, r3, #1
    cmp r3, r1
    bne map_cell
    add r2, r2, #1
    cmp r2, r1
    bne map_row
    bx lr

    .balign 4
copies:
    .word VRAM_A + 0x10000, BUF, 80                  @ tiles
    .word VRAM_A + 0x0000, BUF + 0x1000, 256         @ A BG2 map (1 KiB)
    .word VRAM_A + 0x0800, BUF + 0x2000, 64          @ A BG3 map (256 bytes)
    .word VRAM_B + 0x10000, BUF, 80                  @ tiles
    .word VRAM_B + 0x1000, BUF + 0x3000, 1024        @ B BG3 map (4 KiB)
    .word VRAM_B + 0x8000 + 32, tile_nine, 8         @ B BG0's tile 1
    .word 0

runs:
    @ B BG0: screen block 4, at 0x06202000.
    .word VRAM_B + 0x2000, 2 * 32 + 2, 3, 0x0001
    .word VRAM_B + 0x2000, 3 * 32 + 2, 3, 0x0001
    .word VRAM_B + 0x2000, 2 * 32 + 8, 1, 0x3001
    .word 0

    @ A tile of 4 bits per pixel whose pixels are all 9.
tile_nine:
    .word 0x99999999, 0x99999999, 0x99999999, 0x99999999
    .word 0x99999999, 0x99999999, 0x99999999, 0x99999999
