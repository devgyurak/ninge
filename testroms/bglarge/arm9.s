@ SPDX-License-Identifier: Apache-2.0
@ Large-bitmap ROM, ARM9 side: shows the large 256-colour bitmap of BG mode 6 on engine A (the upper
@ screen), rotated and scaled, without wrap (docs/hw/bg-modes.md, "Large bitmap"). Engine B keeps its
@ power-on DISPCNT (display mode 0, white). No libnds, no crt0, no image asset: every byte is
@ computed here. Loaded at 0x02000000, entered at 0x02000100. The expected picture is worked out
@ from the note in crates/runner/tests/bglarge.rs, which holds its own copy of the data described
@ here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A..D = 0x81,
@ 0x89, 0x91, 0x99 (banks A-D, MST 1, OFS 0-3: the whole 512 KiB of engine A's BG VRAM at
@ 0x06000000). DISPCNT of engine A = 0x00010406 (display mode 1, BG mode 6, BG2 on).
@
@ Palette (engine A BG palette, 0x05000000): entry i = R i&31, G (3i)&31, B (5i)&31 as a 15-bit
@ colour, for i = 0..255; then entry 0 (the backdrop) = 0x03E0, green.
@
@ Bitmap: 1024x512, one byte a pixel, rows in order from the start of BG VRAM. Texel (tx, ty) holds
@ 1 + (((tx >> 2) + 2 ty) & 127): four neighbouring pixels share a value (a word at a time).
@ BG2: BG2CNT = 0x4080 (priority 0, bit 7, size 1 = 1024x512, wrap off). PA 0x00A0, PB 0x0050,
@ PC 0xFFB0, PD 0x00A0, X 0xFFFFD800 (-40.0), Y 0x00001E00 (30.0).
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ BG2CNT_A, 0x0400000C
    .equ AFFINE_A, 0x04000020
    .equ PAL_A, 0x05000000
    .equ VRAM_A, 0x06000000

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
    mov r1, #0x89
    strb r1, [r0, #1]
    mov r1, #0x91
    strb r1, [r0, #2]
    mov r1, #0x99
    strb r1, [r0, #3]

    @ The palette.
    ldr r8, =PAL_A
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
    ldr r5, =0x03E0
    strh r5, [r8]

    @ The bitmap, a word (four pixels of one value) at a time, straight into VRAM.
    ldr r0, =VRAM_A
    mov r1, #0                   @ ty
bmp_row:
    mov r2, #0                   @ tx >> 2
bmp_word:
    add r3, r2, r1, lsl #1
    and r3, r3, #127
    add r3, r3, #1
    orr r3, r3, r3, lsl #8
    orr r3, r3, r3, lsl #16
    str r3, [r0], #4
    add r2, r2, #1
    cmp r2, #256
    bne bmp_word
    add r1, r1, #1
    cmp r1, #512
    bne bmp_row

    @ Registers.
    ldr r0, =BG2CNT_A
    ldr r1, =0x4080
    strh r1, [r0]
    ldr r0, =AFFINE_A
    ldr r1, =0x005000A0          @ PB:PA
    str r1, [r0]
    ldr r1, =0x00A0FFB0          @ PD:PC
    str r1, [r0, #4]
    ldr r1, =0xFFFFD800          @ BG2X
    str r1, [r0, #8]
    ldr r1, =0x00001E00          @ BG2Y
    str r1, [r0, #12]
    ldr r0, =DISPCNT_A
    ldr r1, =0x00010406
    str r1, [r0]
done:
    b done

    .ltorg
