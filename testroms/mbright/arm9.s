@ SPDX-License-Identifier: Apache-2.0
@ Master brightness ROM, ARM9 side: shows MASTER_BRIGHT on both engines (docs/hw/blending.md, "Master
@ brightness"). Engine A starts in display mode 2 (VRAM display) showing a gradient, and later
@ switches to display mode 1 (the layers); engine B shows the layers throughout: BG0 red in columns
@ 0-84, the white backdrop in 85-169 and BG2 blue from 170 on, cut by windows. The ROM then steps
@ through nine MASTER_BRIGHT settings, one for each frame: it waits for VBlank, writes the next row
@ of `steps`, and waits for the line to leave VBlank. No libnds, no crt0, no image asset: every byte
@ is computed here or listed below. Loaded at 0x02000000, entered at 0x02000100. The expected
@ pictures are worked out from the note in crates/runner/tests/mbright.rs, which holds its own copy of
@ the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A = 0x80 (bank
@ A, MST 0: the LCDC window 0x06800000, read by display mode 2), VRAMCNT_C = 0x84 (bank C, MST 4:
@ engine B BG VRAM, 0x06200000) and VRAMCNT_E = 0x81 (bank E, MST 1: engine A BG VRAM, 0x06000000).
@ The palette and the four solid layers are those of the `win` ROM.
@
@ The gradient: the halfword at (x, y) of bank A is (x >> 3) | (y >> 3) << 5 | ((x >> 3) ^ (y >> 3)) << 10,
@ red, green and blue each five bits, so it holds mid-tones as well as the extremes.
@
@ Windows, both engines: WIN0H 0x0055 (columns 0-84), WIN1H 0x55AA (columns 85-169), both rows 0-191
@ (WINxV 0x00C0), WININ 0x0001 (window 0 BG0, window 1 nothing), WINOUT 0x0004 (outside BG2).
@ Engine A DISPCNT starts as 0x00020000 (display mode 2); engine B's is 0x00016F00 (display mode 1,
@ BG0-BG3 on, windows 0 and 1 on).
@
@ Steps: nine rows of three words, written in VBlank: engine A's DISPCNT, engine A's MASTER_BRIGHT
@ and engine B's MASTER_BRIGHT.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ VCOUNT, 0x04000006
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_C, 0x04000242
    .equ VRAMCNT_E, 0x04000244
    .equ PAL_A, 0x05000000
    .equ PAL_B, 0x05000400
    .equ VRAM_A, 0x06000000
    .equ VRAM_B, 0x06200000
    .equ LCDC_A, 0x06800000
    .equ STEPS, 9

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
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =VRAMCNT_C
    mov r1, #0x84
    strb r1, [r0]
    ldr r0, =VRAMCNT_E
    mov r1, #0x81
    strb r1, [r0]

    @ The gradient, row by row.
    ldr r0, =LCDC_A
    mov r1, #0                   @ y
fill_row:
    mov r2, #0                   @ x
fill_px:
    mov r3, r2, lsr #3
    mov r4, r1, lsr #3
    eor r5, r3, r4
    orr r3, r3, r4, lsl #5
    orr r3, r3, r5, lsl #10
    strh r3, [r0], #2
    add r2, r2, #1
    cmp r2, #256
    bne fill_px
    add r1, r1, #1
    cmp r1, #192
    bne fill_row

    @ Both engines are set up inside one VBlank, so no frame is half drawn.
    ldr r6, =VCOUNT
setup_wait:
    ldrh r0, [r6]
    cmp r0, #192
    blo setup_wait
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

    @ The first frame after the setup shows no brightness. One row of `steps` for each frame after
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
    ldmia r4!, {r0-r2}
    ldr r7, =DISPCNT_A
    str r0, [r7]
    str r1, [r7, #0x6C]          @ MASTER_BRIGHT of engine A
    ldr r7, =DISPCNT_B
    str r2, [r7, #0x6C]          @ MASTER_BRIGHT of engine B
wait_visible:
    ldrh r0, [r6]
    cmp r0, #192
    bhs wait_visible
    subs r5, r5, #1
    bne step
done:
    b done

    .ltorg

    @ setup_engine: r8 = palette base, r9 = VRAM base, r10 = the engine's DISPCNT, r11 = four words:
    @ the registers at +0x40 (WIN0H, WIN1H), +0x44 (WIN0V, WIN1V), +0x48 (WININ, WINOUT) and DISPCNT.
setup_engine:
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
    .word 0x55AA0055, 0x00C000C0, 0x00040001, 0x00020000
regs_b:
    .word 0x55AA0055, 0x00C000C0, 0x00040001, 0x00016F00

    @ Engine A's DISPCNT, then MASTER_BRIGHT of engine A and of engine B, for each step.
steps:
    .word 0x00020000, 0x4008, 0x4008
    .word 0x00020000, 0x8008, 0x8008
    .word 0x00020000, 0x0008, 0x8008
    .word 0x00020000, 0xC008, 0xC008
    .word 0x00020000, 0x4000, 0x801F
    .word 0x00020000, 0x401F, 0x8010
    .word 0x00016F00, 0x4008, 0x0000
    .word 0x00016F00, 0x8004, 0x4004
    .word 0x00016F00, 0x0000, 0x8010
