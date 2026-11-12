@ SPDX-License-Identifier: Apache-2.0
@ Window ROM, ARM9 side: shows WIN0, WIN1 and the outside region on both engines
@ (docs/hw/windows.md): four solid text layers, BG0 red in front to BG3 yellow behind, over a white
@ backdrop, and windows that choose which of them a pixel may show. No libnds, no crt0, no image
@ asset: every byte is computed here or listed below. Loaded at 0x02000000, entered at 0x02000100.
@ The expected picture is worked out from the note in crates/runner/tests/win.rs, which holds its own
@ copy of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A = 0x81
@ (bank A, MST 1: engine A BG VRAM, 0x06000000), VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM,
@ 0x06200000). Both engines are set up alike by setup_engine, with their own window registers.
@
@ Palette (both engines, 0x05000000 and 0x05000400): entry 0 (the backdrop) 0x7FFF white, 1 = 0x001F
@ red, 2 = 0x03E0 green, 3 = 0x7C00 blue, 4 = 0x03FF yellow.
@
@ Text layers (both engines): BG0-BG3 are 4 bits per pixel, 256x256, priority 0, character block 1
@ (0x4000), screen block n (n * 0x800) for BGn. Tile t (t = 1-4, at 0x4000 + 32 t) has every pixel
@ t, and the map of BGn is all tile n + 1. So BG0 is red, BG1 green, BG2 blue and BG3 yellow, each
@ over the whole screen, and the front-most layer a region shows is the one that appears.
@
@ Engine A: DISPCNT 0x00016F00 (display mode 1, BG0-BG3 on, windows 0 and 1 on).
@   WIN0H 0x28A0 (columns 40-159), WIN0V 0x1E78 (rows 30-119),
@   WIN1H 0x64DC (columns 100-219), WIN1V 0x50AA (rows 80-169),
@   WININ 0x0402: inside window 0 BG1 only, inside window 1 BG2 only,
@   WINOUT 0x0008: outside BG3 only.
@   The two windows overlap in columns 100-159, rows 80-119, where window 0 wins.
@ Engine B: DISPCNT 0x00014F00 (display mode 1, BG0-BG3 on, window 1 on).
@   WIN1H 0xB43C (X1 = 180 is past X2 = 60: columns 180-255), WIN1V 0x6428 (Y1 = 100 is past Y2 =
@   40: rows 100-191), WIN0H and WIN0V 0 (window 0 is off),
@   WININ 0x0100: inside window 1 BG0 only, WINOUT 0x0006: outside BG1 and BG2.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_C, 0x04000242
    .equ PAL_A, 0x05000000
    .equ PAL_B, 0x05000400
    .equ VRAM_A, 0x06000000
    .equ VRAM_B, 0x06200000

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
    ldr r9, =VRAM_A
    ldr r10, =DISPCNT_A
    adr r11, regs_a
    bl setup_engine
    ldr r8, =PAL_B
    ldr r9, =VRAM_B
    ldr r10, =DISPCNT_B
    adr r11, regs_b
    bl setup_engine
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
    .word 0x64DC28A0, 0x50AA1E78, 0x00080402, 0x00016F00
regs_b:
    .word 0xB43C0000, 0x64280000, 0x00060100, 0x00014F00
