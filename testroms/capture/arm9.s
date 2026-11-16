@ SPDX-License-Identifier: Apache-2.0
@ Display capture ROM, ARM9 side: captures engine A's graphics screen into VRAM bank B and shows the
@ bank back in display mode 2 (docs/hw/display-capture.md). No libnds, no crt0, no image asset:
@ every byte is computed here or listed below. Loaded at 0x02000000, entered at 0x02000100. The
@ expected pictures are worked out from the note in crates/runner/tests/capture.rs, which holds its
@ own copy of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on and left in display
@ mode 0), VRAMCNT_B = 0x80 (bank B, MST 0: the LCDC window 0x06820000, which capture writes and
@ display mode 2 reads) and VRAMCNT_E = 0x81 (bank E, MST 1: engine A BG VRAM, 0x06000000).
@
@ The picture of the layers (display mode 1, BG0 only): the backdrop is blue (palette entry 0). BG0
@ is a 4 bits-per-pixel text layer, character block 1, screen block 0, of two solid tiles: tile 1
@ is palette entry 1 and tile 2 is palette entry 2 (green). Map rows 0-15 hold tile 1 in columns
@ 0-15 and tile 2 in columns 16-31; the other rows hold tile 0 (transparent). So rows 0-127 are
@ palette entry 1 in columns 0-127 and green in columns 128-255, and rows 128-191 are the backdrop.
@ Palette entry 1 is changed by the steps.
@
@ Steps: eight rows of four words, written in VBlank, one for each frame after the first: engine A's
@ DISPCNT, DISPCAPCNT, MASTER_BRIGHT of engine A and palette entry 1.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCAPCNT, 0x04000064
    .equ MASTER_BRIGHT_A, 0x0400006C
    .equ VCOUNT, 0x04000006
    .equ VRAMCNT_B, 0x04000241
    .equ VRAMCNT_E, 0x04000244
    .equ PAL_A, 0x05000000
    .equ VRAM_A, 0x06000000
    .equ STEPS, 8

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
    ldr r0, =VRAMCNT_B
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =VRAMCNT_E
    mov r1, #0x81
    strb r1, [r0]

    @ Everything is set up inside one VBlank, so no frame is half drawn.
    ldr r6, =VCOUNT
setup_wait:
    ldrh r0, [r6]
    cmp r0, #192
    blo setup_wait

    @ The palette: blue backdrop, then palette entries 1 (red) and 2 (green).
    ldr r8, =PAL_A
    adr r0, palette_data
    mov r1, #3
pal_loop:
    ldrh r2, [r0], #2
    strh r2, [r8], #2
    subs r1, r1, #1
    bne pal_loop

    @ Tile t = 1 and 2 at 0x4000 + 32 t: eight words of t * 0x11111111.
    ldr r9, =VRAM_A
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
    cmp r4, #3
    bne tile_next

    @ The map, 16 rows of 32 entries: tile 1 in columns 0-15 and tile 2 in columns 16-31. The
    @ remaining rows stay tile 0 (VRAM starts as zeros).
    mov r0, r9
    mov r1, #16
map_row:
    mov r2, #32
    mov r3, #0
map_col:
    cmp r3, #16
    movlo r4, #1
    movhs r4, #2
    strh r4, [r0], #2
    add r3, r3, #1
    cmp r3, r2
    bne map_col
    subs r1, r1, #1
    bne map_row

    @ BG0CNT: character block 1 (bits 2-5), screen block 0. DISPCNT: display mode 1, BG0 on.
    ldr r0, =DISPCNT_A
    mov r1, #4
    strh r1, [r0, #8]
    ldr r1, =0x00010100
    str r1, [r0]

    @ The first frame after the setup shows the layers alone. One row of `steps` for each frame after
    @ that: wait for VBlank, write, wait for the visible lines.
    ldr r6, =VCOUNT
first_visible:
    ldrh r0, [r6]
    cmp r0, #192
    bhs first_visible
    adr r4, steps
    mov r5, #STEPS
step:
wait_vblank:
    ldrh r0, [r6]
    cmp r0, #192
    blo wait_vblank
    ldmia r4!, {r0-r3}
    ldr r7, =DISPCNT_A
    str r0, [r7]
    str r1, [r7, #0x64]          @ DISPCAPCNT
    strh r2, [r7, #0x6C]         @ MASTER_BRIGHT of engine A
    ldr r7, =PAL_A
    strh r3, [r7, #2]            @ palette entry 1
wait_visible:
    ldrh r0, [r6]
    cmp r0, #192
    bhs wait_visible
    subs r5, r5, #1
    bne step
done:
    b done

    .ltorg

    .balign 4
palette_data:
    .hword 0x7C00, 0x001F, 0x03E0

    @ DISPCNT, DISPCAPCNT, MASTER_BRIGHT of engine A and palette entry 1, for each step.
    @ DISPCNT 0x00010100 is display mode 1 (BG0); 0x00060100 is display mode 2 reading block 1 (B) with BG0 still on, so the graphics screen is still composed.
    @ DISPCAPCNT: bit 31 enable, block B (bits 16-17 = 1), source A, then the write offset (bits
    @ 18-19) and size (bits 20-21): 0x80310000 whole screen at 0, 0x80150000 256x64 at 0x8000,
    @ 0x80090000 128x128 at 0x10000, 0x803D0000 whole screen at 0x18000 (wraps).
    .balign 4
steps:
    .word 0x00010100, 0x80310000, 0x0000, 0x001F
    .word 0x00060100, 0x80150000, 0x0000, 0x03FF
    .word 0x00060100, 0x00000000, 0x0000, 0x03FF
    .word 0x00060100, 0x80090000, 0x0000, 0x03FF
    .word 0x00010100, 0x80310000, 0x4008, 0x03E0
    .word 0x00060100, 0x00000000, 0x0000, 0x03E0
    .word 0x00010100, 0x803D0000, 0x0000, 0x03E0
    .word 0x00060100, 0x00000000, 0x0000, 0x03E0
