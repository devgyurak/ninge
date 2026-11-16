@ SPDX-License-Identifier: Apache-2.0
@ Colour effects ROM, ARM9 side: shows BLDCNT, BLDALPHA and BLDY on both engines
@ (docs/hw/blending.md). The four solid text layers of the `win` ROM (BG0 red in front to BG3 yellow
@ behind, over a white backdrop), windows that cut the screen into three bands with different layers
@ and effect bits, and on engine A two objects. The ROM then steps through nine register settings,
@ one for each frame: it waits for VBlank, writes the next row of `steps`, and waits for the line to
@ leave VBlank. No libnds, no crt0, no image asset: every byte is computed here or listed below.
@ Loaded at 0x02000000, entered at 0x02000100. The expected pictures are worked out from the note in
@ crates/runner/tests/blend.rs, which holds its own copy of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on), VRAMCNT_A = 0x81 (bank
@ A, MST 1: engine A BG VRAM), VRAMCNT_B = 0x82 (bank B, MST 2: engine A OBJ VRAM, 0x06400000) and
@ VRAMCNT_C = 0x84 (bank C, MST 4: engine B BG VRAM). The palette and the four layers are those of the
@ `win` ROM; the OBJ palette (0x05000200) has entry 1 = 0x7C1F, magenta, and OBJ tile 0 is solid 1.
@
@ Engine A: DISPCNT 0x00017F00 (display mode 1, BG0-BG3 and OBJ on, windows 0 and 1 on).
@   WIN0H 0x0050 (columns 0-79), WIN1H 0x50A0 (columns 80-159), both rows 0-191 (WINxV 0x00C0).
@   WININ 0x3D3F: window 0 BG0-BG3, OBJ and the effect; window 1 BG0, BG2, BG3, OBJ and the effect.
@   WINOUT 0x0013: outside (columns 160-255) BG0, BG1 and OBJ, no effect.
@   Entry 0  mode 0, 8x8, at (20, 20), tile 0: a normal object in window 0.
@   Entry 1  mode 1 (semi-transparent), 8x8, at (100, 20): in window 1. Entries 2-127 are disabled.
@ Engine B: DISPCNT 0x00016F00 (display mode 1, BG0-BG3 on, windows 0 and 1 on, no objects).
@   WIN0H 0x0055 (columns 0-84), WIN1H 0x55AA (columns 85-169), both rows 0-191.
@   WININ 0x2820: window 0 the effect only (so the backdrop is the top pixel), window 1 BG3 and the
@   effect. WINOUT 0x002C: outside BG2, BG3 and the effect.
@
@ Steps: nine rows of four words, written in VBlank: BLDCNT | BLDALPHA << 16 and BLDY of engine A,
@ then the same of engine B.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ DISPCNT_B, 0x04001000
    .equ VCOUNT, 0x04000006
    .equ VRAMCNT_A, 0x04000240
    .equ VRAMCNT_B, 0x04000241
    .equ VRAMCNT_C, 0x04000242
    .equ PAL_A, 0x05000000
    .equ PAL_B, 0x05000400
    .equ VRAM_A, 0x06000000
    .equ VRAM_B, 0x06200000
    .equ OBJ_A, 0x06400000
    .equ OAM_A, 0x07000000
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
    mov r1, #0x81
    strb r1, [r0]
    ldr r0, =VRAMCNT_B
    mov r1, #0x82
    strb r1, [r0]
    ldr r0, =VRAMCNT_C
    mov r1, #0x84
    strb r1, [r0]

    @ The objects of engine A: OBJ palette entry 1 magenta, tile 0 solid 1, every OAM entry
    @ disabled, then entries 0 and 1.
    ldr r0, =PAL_A + 0x202
    ldr r1, =0x7C1F
    strh r1, [r0]
    ldr r0, =OBJ_A
    ldr r1, =0x11111111
    mov r2, #8
obj_tile:
    str r1, [r0], #4
    subs r2, r2, #1
    bne obj_tile
    ldr r0, =OAM_A
    mov r1, #0x200               @ attr0 bit 9: disabled
    mov r2, #128
oam_off:
    strh r1, [r0], #8
    subs r2, r2, #1
    bne oam_off
    ldr r0, =OAM_A
    adr r1, objs
    ldmia r1, {r2-r5}
    stmia r0, {r2-r3}            @ entry 0: attr0, attr1, attr2 and the affine slot
    add r0, r0, #8
    stmia r0, {r4-r5}            @ entry 1

    @ Both engines are set up inside one VBlank, so no frame is half drawn: the lines before DISPCNT
    @ is written would be white and the ones after it the layers.
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

    @ The first frame after the setup shows no effect. One row of `steps` for each frame after that:
    @ wait for VBlank, write, wait for the visible lines.
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
    str r0, [r7, #0x50]          @ BLDCNT and BLDALPHA
    str r1, [r7, #0x54]          @ BLDY
    ldr r7, =DISPCNT_B
    str r2, [r7, #0x50]
    str r3, [r7, #0x54]
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
    .word 0x50A00050, 0x00C000C0, 0x00133D3F, 0x00017F00
regs_b:
    .word 0x55AA0055, 0x00C000C0, 0x002C2820, 0x00016F00
    @ OAM entries 0 and 1, attr0 | attr1 << 16 and attr2: a normal 8x8 object at (20, 20) and a
    @ semi-transparent one (attr0 bit 10) at (100, 20).
objs:
    .word 0x00140014, 0x00000000, 0x00640414, 0x00000000

    @ BLDCNT | BLDALPHA << 16 and BLDY of engine A, then of engine B, for each step.
steps:
    .word 0x08080241, 0, 0x08080844, 0
    .word 0x0C040441, 0, 0x0C042068, 0
    .word 0x141F0241, 0, 0x000000E8, 8
    .word 0x00000091, 8, 0x00000084, 16
    .word 0x000000D1, 8, 0x1010204C, 0
    .word 0x08080181, 16, 0x00000000, 0
    .word 0x08080041, 0, 0x08080844, 0
    .word 0x08080150, 0, 0x0C042068, 0
    .word 0x00000100, 0, 0x000000E8, 8
