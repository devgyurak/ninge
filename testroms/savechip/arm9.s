@ SPDX-License-Identifier: Apache-2.0
@ Save-chip ROM, ARM9 side (docs/hw/auxspi-saves.md). No libnds, no crt0. Loaded at 0x02000000
@ and entered there (docs/hw/cart-header.md).
@
@ The host fits a blank 8 KiB EEPROM before the ROM runs (the golden harness and the test in
@ crates/runner/tests/savechip.rs do; the ROM cannot tell the chip type). The ROM talks to the
@ chip over AUXSPI by hand: AUXSPICNT enable and SPI mode, a byte written to AUXSPIDATA, busy
@ polled, the reply read back. Hold (AUXSPICNT bit 6) is set on every byte of a command but its
@ last, which ends the session.
@
@ Four checks, each shown as a 48-row band on engine A's screen (the bottom one, display swap
@ clear): green passes, red fails.
@   0  WREN, write A1h B2h C3h D4h at 0040h, poll RDSR until WIP is clear, read the four bytes back.
@   1  A write at 0080h without WREN is ignored: two bytes read back there are still FFh.
@   2  Page wrap: WREN, 33 bytes (1..33) written from 0100h, a page-aligned address, so the 33rd
@      byte wraps to 0100h. Byte 0100h reads 33 and byte 011Fh reads 32 (the 8 KiB EEPROM's page is
@      32 bytes).
@   3  The write latch: RDSR reads 00h after the write above ended (the latch clears), 02h after
@      WREN, and 00h after WRDI.
@
@ Record at 0x02008000, one word per slot (the expected values are in
@ crates/runner/tests/savechip.rs):
@   0  the four bytes of check 0, first byte lowest     1  the two bytes of check 1
@   2  bytes 0100h and 011Fh of check 2 (low, then high byte)
@   3  the three RDSR values of check 3, first lowest
@   8..11  the band colours (halfwords 03E0h green, 001Fh red)
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02008000
    .equ AUXSPICNT, 0x040001A0
    .equ AUXSPIDATA, 0x040001A2
    .equ EXMEMCNT, 0x04000204
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000

    @ Sends the byte \value, with the chip held selected afterwards when \hold is 1; the reply is
    @ left in r0.
    .macro SPI value, hold
    mov r0, #\value
    mov r1, #\hold
    bl spi_byte
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    @ Protection unit and caches off, so every access reaches the bus.
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop

    ldr r8, =REC
    @ The ARM9 takes the slot: EXMEMCNT bit 11 clear.
    ldr r0, =EXMEMCNT
    ldrh r1, [r0]
    bic r1, r1, #0x800
    strh r1, [r0]

    @ ---- 0: write four bytes at 0040h, wait for the write, read them back.
    SPI 0x06, 0                @ WREN
    SPI 0x02, 1                @ WRITE
    SPI 0x00, 1                @ address 0040h, high byte
    SPI 0x40, 1
    SPI 0xA1, 1
    SPI 0xB2, 1
    SPI 0xC3, 1
    SPI 0xD4, 0
    bl wait_ready
    SPI 0x03, 1                @ READ at 0040h
    SPI 0x00, 1
    SPI 0x40, 1
    SPI 0x00, 1
    mov r4, r0
    SPI 0x00, 1
    orr r4, r4, r0, lsl #8
    SPI 0x00, 1
    orr r4, r4, r0, lsl #16
    SPI 0x00, 0
    orr r4, r4, r0, lsl #24
    str r4, [r8, #0]

    @ ---- 1: a write with no WREN is ignored.
    SPI 0x02, 1
    SPI 0x00, 1
    SPI 0x80, 1
    SPI 0x5A, 0
    SPI 0x03, 1                @ READ at 0080h
    SPI 0x00, 1
    SPI 0x80, 1
    SPI 0x00, 1
    mov r4, r0
    SPI 0x00, 0
    orr r4, r4, r0, lsl #8
    str r4, [r8, #4]

    @ ---- 2: 33 bytes from the page-aligned 0100h; the last wraps to the page's first byte.
    SPI 0x06, 0
    SPI 0x02, 1
    SPI 0x01, 1                @ address 0100h
    SPI 0x00, 1
    mov r9, #1
page_loop:
    mov r0, r9
    cmp r9, #33
    moveq r1, #0               @ the last byte ends the session
    movne r1, #1
    bl spi_byte
    add r9, r9, #1
    cmp r9, #34
    bne page_loop
    SPI 0x03, 1                @ READ at 0100h
    SPI 0x01, 1
    SPI 0x00, 1
    SPI 0x00, 0
    mov r4, r0
    SPI 0x03, 1                @ READ at 011Fh
    SPI 0x01, 1
    SPI 0x1F, 1
    SPI 0x00, 0
    orr r4, r4, r0, lsl #8
    str r4, [r8, #8]

    @ ---- 3: the write latch.
    SPI 0x05, 1                @ RDSR after the write ended
    SPI 0x00, 0
    mov r4, r0
    SPI 0x06, 0                @ WREN
    SPI 0x05, 1
    SPI 0x00, 0
    orr r4, r4, r0, lsl #8
    SPI 0x04, 0                @ WRDI
    SPI 0x05, 1
    SPI 0x00, 0
    orr r4, r4, r0, lsl #16
    str r4, [r8, #12]

    @ ---- The band colours: green passes, red fails.
    ldr r5, =0x03E0
    ldr r6, =0x001F
    ldr r0, [r8, #0]
    ldr r2, =0xD4C3B2A1
    cmp r0, r2
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(8 * 4)]
    ldr r0, [r8, #4]
    ldr r2, =0x0000FFFF
    cmp r0, r2
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(9 * 4)]
    ldr r0, [r8, #8]
    ldr r2, =0x00002021
    cmp r0, r2
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(10 * 4)]
    ldr r0, [r8, #12]
    ldr r2, =0x00000200
    cmp r0, r2
    moveq r1, r5
    movne r1, r6
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
    ldr r0, =0x53415645        @ "SAVE"
    str r0, [r8, #(31 * 4)]
spin:
    b spin

@ One SPI byte: r0 is the byte, r1 is 1 to keep the chip selected afterwards. AUXSPICNT is
@ written with the slot enabled in SPI mode (A000h), baud 0, and hold (bit 6) when asked; the byte
@ goes to AUXSPIDATA, AUXSPICNT bit 7 (busy) is polled, and the reply comes back in r0.
spi_byte:
    ldr r2, =AUXSPICNT
    ldr r3, =0xA000
    cmp r1, #0
    orrne r3, r3, #0x40
    strh r3, [r2]
    strh r0, [r2, #2]
sb_wait:
    ldrh r3, [r2]
    tst r3, #0x80
    bne sb_wait
    ldrh r0, [r2, #2]
    bx lr

@ Polls RDSR until the write-in-progress bit (bit 0) is clear.
wait_ready:
    mov r10, lr                @ no stack here: r10 keeps the return address, which spi_byte spares
wr_poll:
    SPI 0x05, 1
    SPI 0x00, 0
    tst r0, #1
    bne wr_poll
    bx r10
    .ltorg
