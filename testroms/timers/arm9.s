@ SPDX-License-Identifier: Apache-2.0
@ Timers ROM, ARM9 side: the ARM9's four timers, measured against the frame (VCOUNT) and the
@ interrupt controller. No libnds, no crt0, no handshake with the ARM7 (its timers are separate
@ hardware, docs/hw/timers.md). Loaded and entered at 0x02000000 (docs/hw/cart-header.md). The ROM
@ records observed values only; the expected ones live in crates/runner/tests/timers.rs, each
@ citing docs/hw/timers.md.
@
@ Timer n is at 0x04000100 + 4n: reload/counter at +0, control at +2.
@ Frame measurements start and stop on the edge of VCOUNT = 192, so a count covers exactly one
@ frame (1,120,380 master cycles) apart from a few instructions of polling.
@
@ ARM9 record at 0x02004000, one word per slot:
@   0  timer 2, F/1024, reload 0: counter one frame after its start
@   1  timer 1, count-up, reload 0, fed by timer 0 (F/1, reload 0): counter one frame after
@   2  timer 0's own counter at that moment
@   3  timer 1 control read back      4  timer 2 control read back
@   5  handler count when timer 3 (F/64, reload FC00h, IRQ on) had been taken 3 times
@   6  OR of the IF values the handler saw
@   7  IF after the handler acknowledged and the timer stopped
@   8  timer 3 counter just after stopping   9  ... again after a delay (frozen)
@   10 IF bit 4 with timer 1 running (F/1, reload FFF0h) and its IRQ enable clear
@   11 IF bit 4 after the IRQ enable was set on the running timer
@   12 IF bit 3 with timer 0 set to count-up (reload FFF0h, IRQ on): timer 0 ignores the bit
@   13 timer 2 counter right after a 32-bit write of reload 1000h and control 80h
@   14 timer 2 counter after a halfword write of reload 8000h while it runs
@   15 timer 2 control read back   16 timer 3 counter right after a restart (reload FC00h)
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02004000

    @ Waits for the edge into line 192: leave it first, so a start in line 192 is not mistaken
    @ for the edge.
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

    @ Spins for about n * 3 instructions: far longer than the timers' periods below.
    .macro SPIN n
    mov r0, #\n
1:  subs r0, r0, #1
    bne 1b
    .endm

    @ Writes a halfword to a timer register: byte offset from 0x04000100.
    .macro TM off, value
    ldr r0, =\value
    strh r0, [r6, #\off]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: IME +0x208, IE +0x210, IF +0x214, VCOUNT +6
    add r6, r9, #0x100         @ TM0CNT_L
    @ Entry state of CP15 is not stated (direct-boot.md): clear the protection unit enable
    @ (bit 0), data cache enable (bit 2), instruction cache enable (bit 12) and the high-vectors
    @ bit (13), so the IRQ vector is at 0x18.
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00002000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    @ The ITCM is off after direct boot (direct-boot.md): 32 KiB, enabled, handler copied in.
    mov r0, #0x0C
    mcr p15, 0, r0, c9, c1, 1
    mrc p15, 0, r0, c1, c0, 0
    orr r0, r0, #0x00040000
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    ldr r0, =handler_start
    ldr r2, =handler_end
    mov r1, #0x18
copy_handler:
    ldr r3, [r0], #4
    str r3, [r1], #4
    cmp r0, r2
    bne copy_handler
    ldr r2, =0x02006000
    mov r1, #0
    str r1, [r2]
    str r1, [r2, #4]

    @ ---- one frame: timer 2 (F/1024) and the chain timer 0 (F/1) -> timer 1 (count-up)
    SYNC_EDGE
    TM 0x0A, 0x83              @ timer 2: start, F/1024
    TM 0x06, 0x84              @ timer 1: start, count-up
    TM 0x02, 0x80              @ timer 0: start, F/1
    SYNC_EDGE
    ldrh r0, [r6, #8]
    RECORD 0, r0
    ldrh r0, [r6, #4]
    RECORD 1, r0
    ldrh r0, [r6]
    RECORD 2, r0
    ldrh r0, [r6, #6]
    RECORD 3, r0
    ldrh r0, [r6, #10]
    RECORD 4, r0
    TM 0x02, 0
    TM 0x06, 0
    TM 0x0A, 0

    @ ---- timer 3 raises its IRQ and the handler takes it
    TM 0x0C, 0xFC00            @ reload: 1024 ticks of F/64 = 131,072 master cycles
    mov r0, #0x40
    str r0, [r9, #0x210]       @ IE = timer 3
    mov r0, #1
    str r0, [r9, #0x208]       @ IME
    TM 0x0E, 0xC1              @ start, IRQ on, F/64
    mov r0, #0x1F
    msr cpsr_c, r0             @ I clear, System mode
wait_irqs:
    ldr r0, [r2]
    cmp r0, #3
    blo wait_irqs
    TM 0x0E, 0                 @ stop
    mov r0, #0x9F
    msr cpsr_c, r0             @ I set again
    mov r0, #0
    str r0, [r9, #0x208]
    ldr r0, [r2]
    RECORD 5, r0
    ldr r0, [r2, #4]
    RECORD 6, r0
    ldr r0, [r9, #0x214]
    RECORD 7, r0
    ldrh r0, [r6, #12]
    RECORD 8, r0
    SPIN 100
    ldrh r0, [r6, #12]
    RECORD 9, r0
    TM 0x0E, 0x81              @ restart without the IRQ: the reload value is loaded again
    ldrh r0, [r6, #12]
    RECORD 16, r0
    TM 0x0E, 0

    @ ---- IF is latched only when the timer's IRQ enable is set (IME stays 0)
    TM 0x04, 0xFFF0            @ timer 1 reload: 16 ticks
    TM 0x06, 0x80              @ start, F/1, IRQ off
    SPIN 100
    ldr r0, [r9, #0x214]
    RECORD 10, r0
    TM 0x06, 0xC0              @ IRQ on, still running
    SPIN 100
    ldr r0, [r9, #0x214]
    RECORD 11, r0
    mov r0, #0x10
    str r0, [r9, #0x214]       @ acknowledge
    TM 0x06, 0

    @ ---- timer 0 has no previous timer: its count-up bit is ignored
    TM 0x00, 0xFFF0
    TM 0x02, 0xC4              @ start, IRQ on, count-up
    SPIN 100
    ldr r0, [r9, #0x214]
    RECORD 12, r0
    mov r0, #0x08
    str r0, [r9, #0x214]
    TM 0x02, 0

    @ ---- the reload is loaded at the start; a later write does not touch the counter
    ldr r0, =0x00801000        @ control 80h in the high half, reload 1000h in the low
    str r0, [r6, #8]
    ldrh r0, [r6, #8]
    RECORD 13, r0
    TM 0x08, 0x8000
    ldrh r0, [r6, #8]
    RECORD 14, r0
    ldrh r0, [r6, #10]
    RECORD 15, r0
    TM 0x0A, 0

    mov r0, #1
    RECORD 30, r0
    ldr r0, =0x544D5239        @ "TMR9"
    RECORD 31, r0
spin:
    b spin

@ The IRQ handler, copied to the vector at ITCM 0x18. It uses r10-r12, which the main code
@ leaves alone (IRQ mode banks only r13 and r14), acknowledges IF, counts and ORs what it saw.
    .align 2
handler_start:
    mov r12, #0x04000000
    ldr r11, [r12, #0x214]
    str r11, [r12, #0x214]
    mov r12, #0x02000000
    add r12, r12, #0x6000
    ldr r10, [r12]
    add r10, r10, #1
    str r10, [r12]
    ldr r10, [r12, #4]
    orr r10, r10, r11
    str r10, [r12, #4]
    subs pc, lr, #4
handler_end:
    .ltorg
