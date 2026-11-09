@ SPDX-License-Identifier: Apache-2.0
@ Timers ROM, ARM7 side: the ARM7's own four timers, measured against the frame (VCOUNT) and IF.
@ No libnds, no handshake with the ARM9, whose timers are separate hardware (docs/hw/timers.md).
@ Loaded and entered at 0x02380000 (docs/hw/cart-header.md). The ARM7 takes no IRQ (its vector is
@ BIOS, #27): it only records IF, which stays visible because IME is 0. The ROM records observed
@ values only; the expected ones live in crates/runner/tests/timers.rs (docs/hw/timers.md).
@
@ ARM7 record at 0x02005000, one word per slot:
@   0  timer 1, F/256, reload 0: counter one frame after its start
@   1  timer 3, count-up, reload 0, fed by timer 2 (F/1, reload 0): counter one frame after
@   2  timer 2's own counter at that moment
@   3  timer 3 control read back   4  timer 1 control read back
@   5  IF with timer 0 (F/1, reload FFF0h, IRQ on) running, IME = 0
@   6  IF after acknowledging and stopping the timer
@   7  timer 0 control after the stop
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02005000

    @ Waits for the edge into line 192, as in arm9.s.
    .macro SYNC_EDGE
1:  ldrh r0, [r9, #6]
    cmp r0, #192
    beq 1b
2:  ldrh r0, [r9, #6]
    cmp r0, #192
    bne 2b
    .endm

    .macro RECORD slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    .macro TM off, value
    ldr r0, =\value
    strh r0, [r6, #\off]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: IF +0x214, VCOUNT +6
    add r6, r9, #0x100         @ TM0CNT_L

    @ ---- one frame: timer 1 (F/256) and the chain timer 2 (F/1) -> timer 3 (count-up)
    SYNC_EDGE
    TM 0x06, 0x82              @ timer 1: start, F/256
    TM 0x0E, 0x84              @ timer 3: start, count-up
    TM 0x0A, 0x80              @ timer 2: start, F/1
    SYNC_EDGE
    ldrh r0, [r6, #4]
    RECORD 0, r0
    ldrh r0, [r6, #12]
    RECORD 1, r0
    ldrh r0, [r6, #8]
    RECORD 2, r0
    ldrh r0, [r6, #14]
    RECORD 3, r0
    ldrh r0, [r6, #6]
    RECORD 4, r0
    TM 0x06, 0
    TM 0x0A, 0
    TM 0x0E, 0

    @ ---- the overflow latches IF with IME = 0
    TM 0x00, 0xFFF0            @ reload: 16 ticks
    TM 0x02, 0xC0              @ start, IRQ on, F/1
    mov r0, #100
spin_irq:
    subs r0, r0, #1
    bne spin_irq
    ldr r0, [r9, #0x214]
    RECORD 5, r0
    mov r0, #0x08
    str r0, [r9, #0x214]       @ acknowledge
    TM 0x02, 0
    ldr r0, [r9, #0x214]
    RECORD 6, r0
    ldrh r0, [r6, #2]
    RECORD 7, r0

    mov r0, #1
    RECORD 30, r0
    ldr r0, =0x544D5237        @ "TMR7"
    RECORD 31, r0
spin:
    b spin
    .ltorg
