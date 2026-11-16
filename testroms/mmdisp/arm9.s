@ SPDX-License-Identifier: Apache-2.0
@ Main-memory display ROM, ARM9 side: a DMA channel in start mode 4 streams a bitmap from main RAM
@ into DISP_MMEM_FIFO, engine A shows it in display mode 3, and the capture unit stores it into
@ VRAM bank B (source B = the FIFO), which display mode 2 shows back (docs/hw/display-capture.md).
@ No libnds, no crt0, no image asset: every byte is computed here or listed below. Loaded at
@ 0x02000000, entered at 0x02000100. The expected pictures are worked out from the note in
@ crates/runner/tests/mmdisp.rs, which holds its own copy of the data described here.
@
@ Setup: POWCNT1 = 0x8203 (engine A on the UPPER screen, engine B powered on and left in display
@ mode 0) and VRAMCNT_B = 0x80 (bank B, MST 0: the LCDC window 0x06820000, which capture writes and
@ display mode 2 reads).
@
@ The bitmap, 256 x 192 halfwords at 0x02200000: the halfword at (x, y) is
@ (x >> 3) | (y >> 3) << 5 | ((x >> 3) ^ (y >> 3)) << 10, red, green and blue each five bits.
@
@ DMA channel 0 is armed in each VBlank before a frame that shows the bitmap: source the bitmap,
@ destination DISP_MMEM_FIFO (fixed), 32-bit units, count 4, repeat, start mode 4. Writing the
@ enable bit again reloads the source, so the next frame starts at the top of the bitmap.
@
@ Steps: six rows of four words, written in VBlank, one for each frame after the first: engine A's
@ DISPCNT, DISPCAPCNT, MASTER_BRIGHT and whether the channel is armed again before the frame.
    .equ POWCNT1, 0x04000304
    .equ DISPCNT_A, 0x04000000
    .equ VCOUNT, 0x04000006
    .equ VRAMCNT_B, 0x04000241
    .equ DMA0SAD, 0x040000B0
    .equ FIFO, 0x04000068
    .equ BITMAP, 0x02200000
    .equ STEPS, 6

    @ The linker script places .head at 0x02000000 and .text at the entry, 0x02000100.
    .section .head, "ax"
    .arm
    .space 0x100, 0

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ The protection unit and both caches are switched off, as in the memmap ROM, so the
    @ stores below reach RAM and I/O through the bus whatever state the loader left them in.
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

    @ The bitmap, row by row.
    ldr r0, =BITMAP
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

    @ Display mode 3 and the channel are set inside one VBlank, so the first frame after it shows the
    @ whole bitmap.
    ldr r6, =VCOUNT
setup_wait:
    ldrh r0, [r6]
    cmp r0, #192
    blo setup_wait
    ldr r0, =DISPCNT_A
    ldr r1, =0x00030000
    str r1, [r0]
    bl arm_dma

    @ One row of `steps` for each frame after the first: wait for VBlank, write, wait for the
    @ visible lines.
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
    cmp r3, #0
    blne arm_dma
wait_visible:
    ldrh r0, [r6]
    cmp r0, #192
    bhs wait_visible
    subs r5, r5, #1
    bne step
done:
    b done

    .ltorg

    @ arm_dma: disable channel 0, then enable it again with the bitmap as the source. Clobbers r0-r2.
arm_dma:
    ldr r0, =DMA0SAD
    mov r1, #0
    str r1, [r0, #8]             @ DMA0CNT: disable
    ldr r1, =BITMAP
    str r1, [r0]                 @ DMA0SAD
    ldr r1, =FIFO
    str r1, [r0, #4]             @ DMA0DAD
    ldr r1, =0xA6400004          @ enable, repeat, 32-bit, destination fixed, start mode 4, 4 words
    str r1, [r0, #8]
    bx lr

    .ltorg

    @ DISPCNT, DISPCAPCNT, MASTER_BRIGHT of engine A and "arm the channel", for each step.
    @ DISPCNT 0x00030000 is display mode 3; 0x00060000 is display mode 2 reading block 1 (B).
    @ DISPCAPCNT 0xA2310000: bit 31 enable, block B (bits 16-17 = 1), the whole screen (size 3),
    @ capture source B (bit 29) taken from the FIFO (bit 25).
    @ MASTER_BRIGHT 0x8008 is down by 8/16, 0x4008 up by 8/16.
    .balign 4
steps:
    .word 0x00030000, 0xA2310000, 0x0000, 1
    .word 0x00060000, 0x00000000, 0x0000, 1
    .word 0x00030000, 0x00000000, 0x8008, 1
    .word 0x00060000, 0x00000000, 0x4008, 1
    .word 0x00030000, 0x00000000, 0x0000, 0
    .word 0x00060000, 0x00000000, 0x0000, 0
