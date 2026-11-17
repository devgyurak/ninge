@ SPDX-License-Identifier: Apache-2.0
@ Cartridge-read ROM, ARM9 side (docs/hw/cart-protocol.md). No libnds, no crt0. Loaded at
@ 0x02000000 and entered there (docs/hw/cart-header.md).
@
@ The ROM reads its own data blob back through the cartridge slot. The blob is 400h bytes of a
@ computed pattern at the start of the .blob section; the linker script puts it 4000h bytes into
@ the ARM9 binary, which the header places at ROM offset 4000h, so it sits at ROM offset 8000h,
@ the first offset the B7 command can read. Its copy in main RAM (0x02004000) is the expected
@ data: the same bytes were loaded there from the same image.
@
@ Four checks, each shown as a 48-row band on engine A's screen (the bottom one, display swap
@ clear): green passes, red fails.
@   0  Polled read: B7 at 8000h, 200h bytes, each word taken from the data port when ROMCTRL
@      bit 23 says one is ready; the 128 words must equal the blob's first 200h bytes.
@   1  DMA read: B7 at 8200h, 200h bytes, taken by DMA channel 0 (ARM9 start mode 5, source the
@      data port fixed, length 1, 32-bit, repeat on: the recipe GBATEK gives); the 128 words must
@      equal the blob's second 200h bytes.
@   2  Chip ID: B8 returns a word that is neither FFFFFFFFh (no card) nor zero.
@   3  Transfer-complete IRQ: with AUXSPICNT bit 14 set, a transfer ends with IF bit 19 set;
@      with it clear, IF bit 19 stays clear. IF is polled: the interrupt is never taken.
@      TODO(hw-unverified): the expectation is the model's: when busy clears and which CPU gets
@      the IRQ are Unverified (docs/hw/cart-protocol.md).
@
@ Record at 0x02008000, one word per slot (the expected values are in
@ crates/runner/tests/cartread.rs):
@   0  words of the polled read that differ from the blob     1  the same for the DMA read
@   2  the chip ID word
@   3  IF bit 19 after a B8 transfer with the IRQ bit set     4  the same with the bit clear
@   8..11  the band colours (halfwords 03E0h green, 001Fh red)
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02008000
    .equ BUF_A, 0x02100000
    .equ BUF_B, 0x02101000
    .equ BLOB_RAM, 0x02004000
    .equ AUXSPICNT, 0x040001A0
    .equ ROMCTRL, 0x040001A4
    .equ CMD, 0x040001A8
    .equ DATA, 0x04100010
    .equ IF, 0x04000214
    .equ EXMEMCNT, 0x04000204
    .equ DMA0SAD, 0x040000B0
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000
    @ ROMCTRL: start (31), RESB (29), gap 2 of 18h, no key bits, block size in bits 24-26.
    .equ READ_200H, 0xA1180000
    .equ READ_4, 0xA7180000

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ Protection unit and caches off, so every access reaches the bus (as the vramdisp ROM does).
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop

    ldr r8, =REC
    @ The ARM9 takes the slot: EXMEMCNT bit 11 clear. Enable the slot with the IRQ bit set.
    ldr r0, =EXMEMCNT
    ldrh r1, [r0]
    bic r1, r1, #0x800
    strh r1, [r0]
    ldr r0, =AUXSPICNT
    ldr r1, =0xC000
    strh r1, [r0]

    @ ---- 0: polled read of the blob's first 200h bytes.
    ldr r0, =0x800000B7        @ B7, address 00008000h, three zero bytes
    mov r1, #0
    ldr r2, =READ_200H
    ldr r4, =BUF_A
    bl read_block
    ldr r0, =BUF_A
    ldr r1, =BLOB_RAM
    bl count_diffs
    str r0, [r8, #0]

    @ ---- 1: DMA read of the second 200h bytes. Channel 0 is armed before the transfer starts.
    ldr r6, =DMA0SAD
    ldr r0, =DATA
    str r0, [r6, #0]
    ldr r0, =BUF_B
    str r0, [r6, #4]
    ldr r0, =0xAF000001        @ enable, mode 5, 32-bit, source fixed, repeat, length 1
    str r0, [r6, #8]
    ldr r2, =CMD
    ldr r0, =0x820000B7        @ B7, address 00008200h
    str r0, [r2, #0]
    mov r0, #0
    str r0, [r2, #4]
    ldr r3, =ROMCTRL
    ldr r0, =READ_200H
    str r0, [r3]
dma_wait:
    ldr r0, [r3]
    tst r0, #(1 << 31)
    bne dma_wait
    mov r0, #0
    str r0, [r6, #8]           @ stop the repeating channel
    ldr r0, =BUF_B
    ldr r1, =(BLOB_RAM + 0x200)
    bl count_diffs
    str r0, [r8, #4]

    @ ---- 2 and 3: the chip ID, and IF bit 19 with the IRQ bit set.
    ldr r0, =IF
    ldr r1, =(1 << 19)
    str r1, [r0]               @ acknowledge
    bl chip_id
    str r0, [r8, #8]
    ldr r0, =IF
    ldr r1, [r0]
    and r1, r1, #(1 << 19)
    str r1, [r8, #12]

    @ ---- 3: the same with the IRQ bit clear.
    ldr r0, =IF
    ldr r1, =(1 << 19)
    str r1, [r0]
    ldr r0, =AUXSPICNT
    ldr r1, =0x8000
    strh r1, [r0]
    bl chip_id
    ldr r0, =IF
    ldr r1, [r0]
    and r1, r1, #(1 << 19)
    str r1, [r8, #16]

    @ ---- The band colours: green passes, red fails.
    ldr r5, =0x03E0
    ldr r6, =0x001F
    ldr r0, [r8, #0]
    cmp r0, #0
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(8 * 4)]
    ldr r0, [r8, #4]
    cmp r0, #0
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(9 * 4)]
    ldr r0, [r8, #8]
    mvn r2, #0
    cmp r0, r2
    cmpne r0, #0
    moveq r1, r6
    movne r1, r5
    strh r1, [r8, #(10 * 4)]
    ldr r0, [r8, #12]
    ldr r2, [r8, #16]
    cmp r0, #0
    moveq r1, r6               @ IF bit 19 must be set with the IRQ bit on
    movne r1, r5
    cmp r2, #0
    movne r1, r6               @ and clear with it off
    strh r1, [r8, #(11 * 4)]

    @ ---- Show the bands: engine A, display mode 2 (VRAM display), bank A as LCDC.
    ldr r0, =POWCNT1
    mov r1, #3
    strh r1, [r0]
    ldr r0, =VRAMCNT_A
    mov r1, #0x80
    strb r1, [r0]
    ldr r0, =DISPCNT
    mov r1, #0x00020000
    str r1, [r0]
    ldr r9, =VRAM_A
    mov r4, #0                 @ y
band_row:
    mov r1, #48
    mov r2, #0                 @ band = y / 48 by repeated subtraction
    mov r0, r4
band_div:
    cmp r0, r1
    blo band_done
    sub r0, r0, r1
    add r2, r2, #1
    b band_div
band_done:
    add r1, r8, #(8 * 4)
    add r1, r1, r2, lsl #2
    ldrh r6, [r1]
    mov r5, #0                 @ x
band_pixel:
    strh r6, [r9], #2
    add r5, r5, #1
    cmp r5, #256
    blo band_pixel
    add r4, r4, #1
    cmp r4, #192
    blo band_row

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x43415244        @ "CARD"
    str r0, [r8, #(31 * 4)]
spin:
    b spin

@ Sends the command words r0, r1 with ROMCTRL value r2, and stores each word the card makes
@ ready at [r4], ascending, until the transfer is done.
read_block:
    ldr r5, =CMD
    str r0, [r5, #0]
    str r1, [r5, #4]
    ldr r3, =ROMCTRL
    str r2, [r3]
rb_loop:
    ldr r0, [r3]
    tst r0, #(1 << 23)
    beq rb_busy
    ldr r1, =DATA
    ldr r1, [r1]
    str r1, [r4], #4
rb_busy:
    tst r0, #(1 << 31)
    bne rb_loop
    bx lr

@ Runs a B8 (chip ID) transfer of one word and returns the word in r0.
chip_id:
    ldr r5, =CMD
    mov r0, #0xB8
    str r0, [r5, #0]
    mov r0, #0
    str r0, [r5, #4]
    ldr r3, =ROMCTRL
    ldr r0, =READ_4
    str r0, [r3]
    mov r2, #0
ci_loop:
    ldr r0, [r3]
    tst r0, #(1 << 23)
    beq ci_busy
    ldr r1, =DATA
    ldr r2, [r1]
ci_busy:
    tst r0, #(1 << 31)
    bne ci_loop
    mov r0, r2
    bx lr

@ Returns in r0 how many of the 128 words at r0 differ from the words at r1.
count_diffs:
    mov r2, #0
    mov r3, #128
cd_loop:
    ldr r4, [r0], #4
    ldr r5, [r1], #4
    cmp r4, r5
    addne r2, r2, #1
    subs r3, r3, #1
    bne cd_loop
    mov r0, r2
    bx lr
    .ltorg

    @ 100h words of a computed pattern: each is the last times 33 plus 12345, from a fixed seed.
    .section .blob, "a"
blob:
    .set V, 0x01234567
    .rept 256
    .word V
    .set V, (V * 33 + 12345) & 0xFFFFFFFF
    .endr
