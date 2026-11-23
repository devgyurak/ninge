@ SPDX-License-Identifier: Apache-2.0
@ Firmware ROM, ARM9 side (docs/hw/firmware.md, docs/hw/direct-boot.md). No libnds, no crt0. Loaded
@ at 0x02000000 and entered there (docs/hw/cart-header.md).
@
@ The ARM7 reads the firmware over the SPI bus and records what it found (arm7.s). This side waits
@ for the ARM7's sentinel, then compares that with what direct boot left in main RAM (the boot-time
@ values, docs/hw/direct-boot.md) and shows four checks as 48-row bands on engine A's screen (the
@ bottom one, display swap clear): green passes, red fails.
@   0  The firmware chip answered RDID with 20h 40h 12h.
@   1  The CRC the ARM7 computed over user-settings area 1 equals the CRC stored in the area.
@   2  The update counters follow the newest-area rule: counter 1 == (counter 0 + 1) & 7Fh.
@   3  The boot indicator at 027FFC40h is 1, and the first 70h bytes of area 1 equal the copy at
@      027FFC80h (the newest area, by check 2).
@
@ Record at 0x02008000 (the ARM7's slots 0-2 and the sentinel are described in arm7.s; the expected
@ values are in crates/runner/tests/firmware.rs):
@   3  the boot indicator, a halfword at 027FFC40h
@   4  the count of words of the 70h-byte copy that differ from the ARM7's read
@   8..11  the band colours (halfwords 03E0h green, 001Fh red)
@ Slot 30: progress (1 when done). Slot 31: the ARM7's sentinel.
    .equ REC, 0x02008000
    .equ BUF, 0x02008100
    .equ BOOT_INDICATOR, 0x027FFC40
    .equ USER_COPY, 0x027FFC80
    .equ POWCNT1, 0x04000304
    .equ DISPCNT, 0x04000000
    .equ VRAMCNT_A, 0x04000240
    .equ VRAM_A, 0x06800000

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
    @ Wait for the ARM7's sentinel "FWRM".
    ldr r2, =0x4D525746
wait_arm7:
    ldr r0, [r8, #(31 * 4)]
    cmp r0, r2
    bne wait_arm7

    @ ---- 3: the boot indicator, and the copy against the ARM7's read, word by word.
    ldr r0, =BOOT_INDICATOR
    ldrh r1, [r0]
    str r1, [r8, #(3 * 4)]
    ldr r0, =USER_COPY
    ldr r1, =BUF
    mov r4, #0                 @ differing words
    mov r5, #28                @ 70h bytes
copy_loop:
    ldr r2, [r0], #4
    ldr r3, [r1], #4
    cmp r2, r3
    addne r4, r4, #1
    subs r5, r5, #1
    bne copy_loop
    str r4, [r8, #(4 * 4)]

    @ ---- The band colours: green passes, red fails.
    ldr r5, =0x03E0
    ldr r6, =0x001F
    @ 0: RDID.
    ldr r0, [r8, #0]
    ldr r2, =0x00124020
    cmp r0, r2
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(8 * 4)]
    @ 1: computed CRC (low half) equals the stored one (high half).
    ldr r0, [r8, #4]
    mov r1, r0, lsr #16
    mov r0, r0, lsl #16
    mov r0, r0, lsr #16
    cmp r0, r1
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(9 * 4)]
    @ 2: counter 1 == (counter 0 + 1) & 7Fh.
    ldr r0, [r8, #8]
    and r1, r0, #0xFF
    mov r2, r0, lsr #8
    and r2, r2, #0xFF
    add r1, r1, #1
    and r1, r1, #0x7F
    cmp r1, r2
    moveq r1, r5
    movne r1, r6
    strh r1, [r8, #(10 * 4)]
    @ 3: boot indicator 1 and no differing word.
    ldr r0, [r8, #(3 * 4)]
    ldr r2, [r8, #(4 * 4)]
    cmp r0, #1
    cmpeq r2, #0
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
spin:
    b spin
    .ltorg
