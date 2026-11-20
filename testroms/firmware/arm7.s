@ SPDX-License-Identifier: Apache-2.0
@ Firmware ROM, ARM7 side (docs/hw/spi.md, docs/hw/firmware.md). No libnds, no crt0. Loaded and
@ entered at 0x02380000 (docs/hw/cart-header.md).
@
@ The ARM7 talks to the firmware flash over the SPI bus by hand: SPICNT enable and firmware select
@ (device 1), a byte written to SPIDATA, SPICNT bit 7 (busy) polled, the reply read back. Hold
@ (SPICNT bit 11) is set on every byte of a command but its last, which ends the session.
@
@ It records, for the ARM9 to judge (see arm9.s):
@   0  RDID: the three ID bytes, first lowest.
@   1  The CRC-16 (init FFFFh, polynomial A001h reflected) the ARM7 computes over the first 70h
@      bytes of user-settings area 1 at 3FF00h (low half), and the CRC stored at +72h (high half).
@   2  The update counters: the halfword at 3FE70h (area 0) in the low byte, the one at 3FF70h
@      (area 1) in the second byte.
@ and leaves area 1's first 74h bytes at 0x02008100 for the ARM9 to compare with the boot RAM copy.
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02008000
    .equ BUF, 0x02008100
    .equ SPICNT, 0x040001C0
    .equ SPIDATA, 0x040001C2

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
    ldr r8, =REC

    @ ---- 0: RDID.
    SPI 0x9F, 1
    SPI 0x00, 1
    mov r4, r0
    SPI 0x00, 1
    orr r4, r4, r0, lsl #8
    SPI 0x00, 0
    orr r4, r4, r0, lsl #16
    str r4, [r8, #0]

    @ ---- 2 (first half): the counter of area 0, the halfword at 3FE70h.
    SPI 0x03, 1
    SPI 0x03, 1
    SPI 0xFE, 1
    SPI 0x70, 1
    SPI 0x00, 1
    mov r4, r0
    SPI 0x00, 0
    @ Only the low byte of each counter matters (it is 00h to 7Fh).
    str r4, [r8, #8]

    @ ---- 1 (reading): the first 74h bytes of area 1 at 3FF00h into BUF.
    SPI 0x03, 1
    SPI 0x03, 1
    SPI 0xFF, 1
    SPI 0x00, 1
    ldr r5, =BUF
    mov r6, #0x74
area_loop:
    subs r6, r6, #1            @ the last byte ends the session
    moveq r1, #0
    movne r1, #1
    mov r0, #0
    bl spi_byte
    strb r0, [r5], #1
    cmp r6, #0
    bne area_loop

    @ ---- 2 (second half): area 1's counter, the halfword at BUF+70h.
    ldr r5, =BUF
    ldrb r0, [r5, #0x70]
    ldr r4, [r8, #8]
    and r4, r4, #0xFF
    orr r4, r4, r0, lsl #8
    str r4, [r8, #8]

    @ ---- 1: the CRC over BUF[0..70h], and the stored one at BUF+72h.
    ldr r4, =0xFFFF
    ldr r9, =0xA001
    mov r6, #0x70
crc_byte:
    ldrb r0, [r5], #1
    eor r4, r4, r0
    mov r7, #8
crc_bit:
    movs r4, r4, lsr #1
    eorcs r4, r4, r9
    subs r7, r7, #1
    bne crc_bit
    subs r6, r6, #1
    bne crc_byte
    ldr r5, =BUF
    ldrh r0, [r5, #0x72]
    orr r4, r4, r0, lsl #16
    str r4, [r8, #4]

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4D525746        @ "FWRM"
    str r0, [r8, #(31 * 4)]
spin:
    b spin

@ One SPI byte: r0 is the byte, r1 is 1 to keep the chip selected afterwards. SPICNT is written
@ with the bus enabled and the firmware selected (8100h), baud 0, and hold (bit 11) when asked; the
@ byte goes to SPIDATA, SPICNT bit 7 (busy) is polled, and the reply comes back in r0.
spi_byte:
    ldr r2, =SPICNT
    ldr r3, =0x8100
    cmp r1, #0
    orrne r3, r3, #0x800
    strh r3, [r2]
    strh r0, [r2, #2]
sb_wait:
    ldrh r3, [r2]
    tst r3, #0x80
    bne sb_wait
    ldrh r0, [r2, #2]
    bx lr
    .ltorg
