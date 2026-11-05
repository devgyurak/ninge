@ SPDX-License-Identifier: Apache-2.0
@ VRAM-display ROM, ARM9 side: draws a computed test picture into VRAM bank A and shows it with
@ engine A in display mode 2 (docs/hw/display-mode-vram.md). No libnds, no crt0, no image
@ asset: every pixel is computed here. Loaded at 0x02000000, entered at 0x02000100.
@
@ Order: POWCNT1, VRAMCNT_A, DISPCNT, then the pixels.
@   POWCNT1 = 0x0003: LCD and engine A on, bit 15 (display swap) CLEAR, so engine A is on the
@     LOWER screen and the upper screen is engine B's black placeholder. Written explicitly: the
@     power-on value of POWCNT1 is unverified and has bit 15 set, so a golden that depends on
@     it would not prove the write works.
@   VRAMCNT_A = 0x80: enabled, MST 0 (LCDC at 0x06800000).
@   DISPCNT   = 0x00020000: display mode 2, VRAM block 0 (bank A).
@
@ The picture, 256 x 192 halfwords of 15-bit colour (bits 0-4 R, 5-9 G, 10-14 B), row by row:
@   rows   0..63   eight 32-pixel bars, full-intensity channels (colour table `bars` below):
@                  white, red, green, blue, cyan, magenta, yellow, black. Odd rows also set
@                  bit 15, which is not part of the colour and must not change the pixel.
@   rows  64..95   red ramp:   channel = x >> 3 (0..31)
@   rows  96..127  green ramp: channel = x >> 3
@   rows 128..159  blue ramp:  channel = x >> 3
@   rows 160..191  grey ramp:  all three channels = x >> 3
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000

    @ The linker script places .head at 0x02000000 and .text at the entry, 0x02000100; the
    @ padding keeps the loaded binary's first byte at the load address.
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
    mov r1, #3
    strh r1, [r0]
    ldr r0, =VRAMCNT_A
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =DISPCNT
    mov r1, #0x00020000
    str r1, [r0]

    ldr r8, =VRAM_A            @ write pointer
    adr r9, bars
    mov r4, #0                 @ y
row:
    mov r5, #0                 @ x
pixel:
    mov r0, r5, lsr #5         @ bar index (rows < 64)
    mov r2, r5, lsr #3         @ ramp channel 0..31 (rows >= 64)
    cmp r4, #64
    bhs ramps
    add r1, r9, r0, lsl #1
    ldrh r6, [r1]
    tst r4, #1
    orrne r6, r6, #0x8000
    b store
ramps:
    sub r1, r4, #64
    mov r1, r1, lsr #5         @ band 0..3: red, green, blue, grey
    cmp r1, #0
    moveq r6, r2
    cmp r1, #1
    moveq r6, r2, lsl #5
    cmp r1, #2
    moveq r6, r2, lsl #10
    cmp r1, #3
    orreq r6, r2, r2, lsl #5
    orreq r6, r6, r2, lsl #10
store:
    strh r6, [r8], #2
    add r5, r5, #1
    cmp r5, #256
    blo pixel
    add r4, r4, #1
    cmp r4, #192
    blo row
spin:
    b spin

    .ltorg
    .balign 2
bars:
    .hword 0x7FFF              @ white
    .hword 0x001F              @ red
    .hword 0x03E0              @ green
    .hword 0x7C00              @ blue
    .hword 0x7FE0              @ cyan (green + blue)
    .hword 0x7C1F              @ magenta (red + blue)
    .hword 0x03FF              @ yellow (red + green)
    .hword 0x0000              @ black
