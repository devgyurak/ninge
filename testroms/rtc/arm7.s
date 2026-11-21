@ SPDX-License-Identifier: Apache-2.0
@ RTC ROM, ARM7 side (docs/hw/rtc.md). No libnds, no crt0. Loaded and entered at 0x02380000
@ (docs/hw/cart-header.md).
@
@ The ARM7 talks to the clock chip by hand through RTCCNT (4000138h), as the note's transfer
@ flowchart says: select the chip, send the command byte, send or receive the parameter bytes,
@ deselect. The flowchart sends every byte least significant bit first; this ROM sends the command
@ MSB first as the usual bytes (such as 65h) are written, which puts the same bits on the wire.
@ Each bit is one clock cycle: the clock goes low (with the data bit on the line when sending), then
@ high, and a received bit is read after it. The runner's host time is the epoch, 2000-01-01
@ 00:00:00 (the Input default), so the clock the chip shows is that until the ROM sets it.
@
@ It records, for the ARM9 to judge (see arm9.s):
@   0  Status register 1 (61h).
@   1  The date and time (65h), bytes 0-3: year, month, day, day of week, first byte lowest.
@   2  The date and time, bytes 4-6: hour, minute, second.
@   3  A date and time written (64h: 2024-12-31, day of week 1, 23:59:58), read back, bytes 0-3.
@   4  The same read back, bytes 4-6.
@   5  The time register (67h) after status 1 is set to 12-hour mode (60h).
@   6  Status register 2 (written 55h) in the low byte, the free register (written A5h) in the second.
@   7  INT2 (6Ah/6Bh): written FFh 12h 34h, read back as three bytes.
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02008000
    .equ RTCCNT, 0x04000138

    .macro CMD value
    bl begin
    mov r0, #\value
    bl tx_msb
    .endm

    .macro TX value
    mov r0, #\value
    bl tx_lsb
    .endm

    @ Receives three bytes, first lowest, into r4.
    .macro RX3
    bl rx_lsb
    mov r4, r0
    bl rx_lsb
    orr r4, r4, r0, lsl #8
    bl rx_lsb
    orr r4, r4, r0, lsl #16
    .endm

    @ Receives seven bytes: the first four into r4, the last three into r5.
    .macro RX7
    bl rx_lsb
    mov r4, r0
    bl rx_lsb
    orr r4, r4, r0, lsl #8
    bl rx_lsb
    orr r4, r4, r0, lsl #16
    bl rx_lsb
    orr r4, r4, r0, lsl #24
    bl rx_lsb
    mov r5, r0
    bl rx_lsb
    orr r5, r5, r0, lsl #8
    bl rx_lsb
    orr r5, r5, r0, lsl #16
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC

    @ ---- 0: status register 1.
    CMD 0x61
    bl rx_lsb
    bl end
    str r0, [r8, #0]

    @ ---- 1, 2: the date and time as the chip starts.
    CMD 0x65
    RX7
    bl end
    str r4, [r8, #4]
    str r5, [r8, #8]

    @ ---- 3, 4: write 2024-12-31, day of week 1, 23:59:58 and read it back.
    CMD 0x64
    TX 0x24
    TX 0x12
    TX 0x31
    TX 0x01
    TX 0x23
    TX 0x59
    TX 0x58
    bl end
    CMD 0x65
    RX7
    bl end
    str r4, [r8, #12]
    str r5, [r8, #16]

    @ ---- 5: 12-hour mode, then the time register.
    CMD 0x60
    TX 0x00
    bl end
    CMD 0x67
    RX3
    bl end
    str r4, [r8, #20]

    @ ---- 6: status register 2 and the free register.
    CMD 0x62
    TX 0x55
    bl end
    CMD 0x6E
    TX 0xA5
    bl end
    CMD 0x63
    bl rx_lsb
    mov r4, r0
    bl end
    CMD 0x6F
    bl rx_lsb
    orr r4, r4, r0, lsl #8
    bl end
    str r4, [r8, #24]

    @ ---- 7: INT2.
    CMD 0x6A
    TX 0xFF
    TX 0x12
    TX 0x34
    bl end
    CMD 0x6B
    RX3
    bl end
    str r4, [r8, #28]

    mov r0, #1
    str r0, [r8, #(30 * 4)]
    ldr r0, =0x4D435452        @ "RTCM"
    str r0, [r8, #(31 * 4)]
spin:
    b spin

@ RTCCNT levels (docs/hw/rtc.md, "RTCCNT bits"): bit 0 data, bit 1 clock, bit 2 select, bits 4-6 the
@ directions of data, clock and select (1 = the program drives the pin).
@   idle      select low, clock high, all three driven, data high   73h
@   selected  the same with select high                              77h
@   low/high  select high, clock low (64h) or high (66h), plus 10h when the data pin is driven
@   deselect  select low, clock high, data an input                  62h

@ Selects the chip: the idle level, then select high.
begin:
    ldr r2, =RTCCNT
    mov r3, #0x73
    strb r3, [r2]
    mov r3, #0x77
    strb r3, [r2]
    bx lr

@ Deselects the chip.
end:
    ldr r2, =RTCCNT
    mov r3, #0x62
    strb r3, [r2]
    bx lr

@ Sends the byte in r0, least significant bit first.
tx_lsb:
    ldr r2, =RTCCNT
    mov r1, #8
tl_bit:
    and r3, r0, #1
    orr ip, r3, #0x74          @ clock low, data driven
    strb ip, [r2]
    orr ip, r3, #0x76          @ clock high, data driven
    strb ip, [r2]
    mov r0, r0, lsr #1
    subs r1, r1, #1
    bne tl_bit
    bx lr

@ Sends the byte in r0, most significant bit first.
tx_msb:
    ldr r2, =RTCCNT
    mov r1, #8
tm_bit:
    mov r3, r0, lsr #7
    and r3, r3, #1
    orr ip, r3, #0x74
    strb ip, [r2]
    orr ip, r3, #0x76
    strb ip, [r2]
    mov r0, r0, lsl #1
    subs r1, r1, #1
    bne tm_bit
    bx lr

@ Receives a byte into r0, least significant bit first: the clock goes low and high with the data
@ pin an input, and the line is read after the clock rises.
rx_lsb:
    ldr r2, =RTCCNT
    mov r1, #0
    mov r0, #0
rl_bit:
    mov r3, #0x64
    strb r3, [r2]
    mov r3, #0x66
    strb r3, [r2]
    ldrb r3, [r2]
    and r3, r3, #1
    orr r0, r0, r3, lsl r1
    add r1, r1, #1
    cmp r1, #8
    bne rl_bit
    bx lr
    .ltorg
