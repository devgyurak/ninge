@ SPDX-License-Identifier: Apache-2.0
@ IPC ROM, ARM9 side: IPCSYNC and the IPC FIFOs from the ARM9's view, with the ARM7 as the other
@ end. The two CPUs take turns, and the turns pass through IPCSYNC itself, not through a flag in
@ RAM: step k belongs to one CPU; when it is done it posts the nibble V(k) = (k mod 15) + 1 in
@ IPCSYNC bits 8-11, and the CPU that owns step k+1 waits for that nibble in its own bits 0-3.
@ Only one CPU works at a time, so the other's IPCSYNC output is stable while a step runs. Both
@ sources hold the same step numbers; the ARM9 owns 1, 3, 5, 7, 8, 10, 12, 14, 16, 19, 21, 23, 25,
@ 27, 29, 30, 32, 34 and the ARM7 owns the rest. No libnds, no crt0. Loaded and entered at 0x02000000
@ (docs/hw/cart-header.md). The ROM records observed values only; the expected ones live in
@ crates/runner/tests/ipc.rs, each citing docs/hw/ipc.md.
@
@ The ARM9 takes IRQs through a handler at ITCM 0x18 (low vectors, as in disptime/arm9.s) in two
@ windows: step 6 (IF.16 sent by the ARM7) and steps 30-33 (IF.17 and IF.18). The handler
@ acknowledges IF, counts at 0x02006000 and ORs the IF values it saw into 0x02006004.
@
@ ARM9 record at 0x02004000, one word per slot:
@   0  IPCSYNC at reset (bits 0-3 depend on when the ARM7's first post lands)
@   1  IPCFIFOCNT at reset
@   2  IPCFIFOCNT after a send with the FIFO disabled        3  ... after enabling it
@   4  IPCSYNC in step 3 (both nibbles posted so far)
@   5  IF after the ARM7's sync IRQ was taken by the handler  6  handler count   7  OR of IF seen
@   8  IPCSYNC in step 7
@   9  IPCFIFOCNT after sending 3 words
@   10 CNT after the ARM7 took them   11 CNT after 16 sends   12 CNT after a 17th   13 ... after ack
@   14 CNT after a flush with 3 words in flight (send FIFO)
@   15 IF with the send-empty IRQ enabled and the FIFO not empty
@   16 IF after the ARM7 took the last word   17 IF after acknowledging it
@   18 IF after a flush made the FIFO empty   19 IF after enabling the IRQ on an empty FIFO
@   20 handler count after the ARM7 took a word   21 OR of IF seen then
@   22 handler count after the ARM7 sent a word   23 OR of IF seen then
@   24 a word read through IPCFIFORECV            25 IF at the end
@ Slot 30: progress (1 when done). Slot 31: sentinel, written last.
    .equ REC, 0x02004000

    @ Waits until the ARM7's output nibble in IPCSYNC bits 0-3 is V(step).
    .macro WAIT step
1:  ldrh r0, [r6]
    and r0, r0, #0xF
    cmp r0, #((\step % 15) + 1)
    bne 1b
    .endm

    @ Posts V(step) in bits 8-11, keeping the enable bit (r7).
    .macro DONE step
    mov r0, #((\step % 15) + 1)
    orr r0, r7, r0, lsl #8
    strh r0, [r6]
    .endm

    @ Posts V(step) and sends an IRQ to the other CPU (bit 13).
    .macro DONE_IRQ step
    mov r0, #((\step % 15) + 1)
    orr r0, r7, r0, lsl #8
    orr r0, r0, #0x2000
    strh r0, [r6]
    .endm

    @ Waits until the IRQ handler's count (r2 = 0x02006000) is at least n, with I still clear. The IRQ
    @ may be taken a few instructions after the ARM7's post is seen, so the count is the only
    @ reliable sign that the handler ran.
    .macro WAIT_COUNT n
1:  ldr r0, [r2]
    cmp r0, #\n
    blo 1b                     @ n or more: an extra IRQ shows in the record, not as a hang
    .endm

    .macro RECORD slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    .macro SEND_WORD value
    ldr r0, =\value
    str r0, [r6, #8]
    .endm

    .macro SET_CNT value
    ldr r0, =\value
    strh r0, [r6, #4]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: IME +0x208, IE +0x210, IF +0x214
    add r6, r9, #0x180         @ IPCSYNC +0, IPCFIFOCNT +4, IPCFIFOSEND +8
    mov r5, #0x04100000        @ IPCFIFORECV
    mov r7, #0                 @ IPCSYNC bit 14 (enable IRQ from the ARM7), kept by DONE
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
    ldr r0, =0x02006000
    mov r1, #0
    str r1, [r0]
    str r1, [r0, #4]

    @ ---- reset values
    ldrh r0, [r6]
    RECORD 0, r0
    ldrh r0, [r6, #4]
    RECORD 1, r0

    @ ---- step 1: a send with the FIFO disabled is ignored; then enable
    SEND_WORD 0xAAAA0001
    ldrh r0, [r6, #4]
    RECORD 2, r0
    SET_CNT 0x8000
    ldrh r0, [r6, #4]
    RECORD 3, r0
    DONE 1

    @ ---- step 3: IPCSYNC with both nibbles, then an IRQ to the ARM7, which is not listening
    WAIT 2
    ldrh r0, [r6]
    RECORD 4, r0
    DONE_IRQ 3

    @ ---- step 5: start listening, send the IRQ again (the ARM7 listens now), open the IRQ window
    WAIT 4
    mov r7, #0x4000
    DONE_IRQ 5
    mov r0, #0x00010000
    str r0, [r9, #0x210]       @ IE = IPC sync
    mov r0, #1
    str r0, [r9, #0x208]       @ IME
    mov r0, #0x1F
    msr cpsr_c, r0             @ I clear, System mode

    @ ---- step 7: the ARM7's IRQ was taken by the handler (during this wait)
    ldr r2, =0x02006000
    WAIT 6
    WAIT_COUNT 1
    mov r0, #0x9F
    msr cpsr_c, r0             @ I set again
    mov r0, #0
    str r0, [r9, #0x208]
    ldr r0, [r9, #0x214]
    RECORD 5, r0
    ldr r0, [r2]
    RECORD 6, r0
    ldr r0, [r2, #4]
    RECORD 7, r0
    ldrh r0, [r6]
    RECORD 8, r0
    DONE 7

    @ ---- step 8: three words
    SEND_WORD 0x11111111
    SEND_WORD 0x22222222
    SEND_WORD 0x33333333
    ldrh r0, [r6, #4]
    RECORD 9, r0
    DONE 8

    @ ---- step 10: the ARM7 took them; fill to 16 and send a 17th
    WAIT 9
    ldrh r0, [r6, #4]
    RECORD 10, r0
    mov r2, #0
fill:
    add r0, r2, #0x100
    str r0, [r6, #8]
    add r2, r2, #1
    cmp r2, #16
    bne fill
    ldrh r0, [r6, #4]
    RECORD 11, r0
    SEND_WORD 0x00000999
    ldrh r0, [r6, #4]
    RECORD 12, r0
    SET_CNT 0xC000             @ enable kept, error acknowledged
    ldrh r0, [r6, #4]
    RECORD 13, r0
    DONE 10

    @ ---- step 12: three words in flight; the ARM7 takes one in step 13
    WAIT 11
    SEND_WORD 0x00000041
    SEND_WORD 0x00000042
    SEND_WORD 0x00000043
    DONE 12

    @ ---- step 14: flush the send FIFO
    WAIT 13
    SET_CNT 0x8008
    ldrh r0, [r6, #4]
    RECORD 14, r0
    DONE 14

    @ ---- step 16: two words for the disabled-receiver probe
    WAIT 15
    SEND_WORD 0x00000051
    SEND_WORD 0x00000052
    DONE 16

    @ ---- step 19: one word to the ARM7, which has the not-empty IRQ on
    WAIT 18
    SEND_WORD 0x00000061
    DONE 19

    @ ---- step 21: a second word while the first is still there
    WAIT 20
    SEND_WORD 0x00000062
    DONE 21

    @ ---- step 23: a word after the FIFO drained
    WAIT 22
    SEND_WORD 0x00000063
    DONE 23

    @ ---- step 25: a word while the ARM7 has the IRQ off
    WAIT 24
    SEND_WORD 0x00000064
    DONE 25

    @ ---- step 27: send-empty IRQ enabled with a word in flight
    WAIT 26
    mov r0, #0x7
    lsl r0, r0, #16
    str r0, [r9, #0x214]       @ clear IF
    SEND_WORD 0x00000071
    SET_CNT 0x8004
    ldr r0, [r9, #0x214]
    RECORD 15, r0
    DONE 27

    @ ---- step 29: the FIFO went empty (IF.17); a flush; enabling on an empty FIFO
    WAIT 28
    ldr r0, [r9, #0x214]
    RECORD 16, r0
    mov r4, #0x00020000
    str r4, [r9, #0x214]
    ldr r0, [r9, #0x214]
    RECORD 17, r0
    SEND_WORD 0x00000072
    SEND_WORD 0x00000073
    SET_CNT 0x800C             @ flush, IRQ enable kept
    ldr r0, [r9, #0x214]
    RECORD 18, r0
    str r4, [r9, #0x214]
    SET_CNT 0x8000
    SET_CNT 0x8004             @ enable on an empty FIFO
    ldr r0, [r9, #0x214]
    RECORD 19, r0
    str r4, [r9, #0x214]
    DONE 29

    @ ---- step 30: both IRQs on, IF.17 and IF.18 deliverable, one word in flight
    SEND_WORD 0x00000075
    SET_CNT 0x8404
    mov r0, #0x00060000
    str r0, [r9, #0x210]       @ IE = IF.17 | IF.18
    mov r0, #1
    str r0, [r9, #0x208]
    mov r0, #0x1F
    msr cpsr_c, r0             @ I clear
    DONE 30

    @ ---- step 32: the ARM7 took the word, so IF.17 came: taken by the handler during this wait
    ldr r2, =0x02006000
    WAIT 31
    WAIT_COUNT 2
    ldr r0, [r2]
    RECORD 20, r0
    ldr r0, [r2, #4]
    RECORD 21, r0
    DONE 32

    @ ---- step 34: the ARM7 sent a word, so IF.18 came: taken by the handler as well
    WAIT 33
    WAIT_COUNT 3
    mov r0, #0x9F
    msr cpsr_c, r0             @ I set again
    mov r0, #0
    str r0, [r9, #0x208]
    ldr r0, [r2]
    RECORD 22, r0
    ldr r0, [r2, #4]
    RECORD 23, r0
    ldr r0, [r5]
    RECORD 24, r0
    ldr r0, [r9, #0x214]
    RECORD 25, r0
    mov r0, #1
    RECORD 30, r0
    ldr r0, =0x49504331
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
