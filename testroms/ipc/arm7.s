@ SPDX-License-Identifier: Apache-2.0
@ IPC ROM, ARM7 side: IPCSYNC and the IPC FIFOs from the ARM7's view, with the ARM9 as the other
@ end. The turn-taking is described at the top of arm9.s: this side owns steps 2, 4, 6, 9, 11, 13,
@ 15, 17, 18, 20, 22, 24, 26, 28, 31, 33 and waits for the ARM9's nibble before each. No libnds. Loaded and
@ entered at 0x02380000 (docs/hw/cart-header.md). The ARM7 takes no IRQ (its vector is BIOS, #27):
@ it only records IF, which stays visible because IME is 0. The ROM records observed values only;
@ the expected ones live in crates/runner/tests/ipc.rs (docs/hw/ipc.md).
@
@ ARM7 record at 0x02005000, one word per slot:
@   0  IPCSYNC at reset (bits 0-3 depend on when the ARM9's first post lands)
@   1  IPCFIFOCNT at reset
@   2  CNT in step 2 (FIFO still disabled, after the ARM9's send with its FIFO disabled)
@   3  IPCSYNC in step 2        4  CNT after enabling
@   5  IF after the ARM9's IRQ while this CPU had IPCSYNC bit 14 clear
@   6  IPCSYNC in step 4        7  IF after the ARM9's IRQ with bit 14 set    8  IPCSYNC in step 6
@   9  IF after acknowledging
@   10 CNT with 3 words waiting   11-13 the words   14 CNT after taking them
@   15 an empty read              16 CNT after it (error)   17 CNT after the acknowledge
@   18 CNT with 16 words waiting  19-34 the words           35 CNT after taking them
@   36 an empty read after 16 words (a 17th sent word would show here)
@   37 the first of 3 words (the ARM9 flushes the rest in step 14)
@   38 CNT after the flush        39 read after the flush   40 CNT after it (error)
@   41,42 two reads with the FIFO disabled   43 CNT then   44,45 two reads after enabling
@   46 CNT after them             47 a read with the FIFO disabled and empty   48 CNT after it
@   49 IF with the not-empty IRQ enabled on an empty FIFO
@   50 IF after one word arrived   51 IF after acknowledging   52 IF after a second word arrived
@   53,54 the two words           55 IF after a word arrived into the drained FIFO   56 the word
@   57 IF after a word arrived with the IRQ off   58 IF after enabling it   59 the word
@   60 the word taken in step 31   61 CNT after sending in step 33
@ Slot 62: progress (1 when done). Slot 63: sentinel, written last.
    .equ REC, 0x02005000

    @ Waits until the ARM9's output nibble in IPCSYNC bits 0-3 is V(step).
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

    .macro RECORD slot, reg
    str \reg, [r8, #(\slot * 4)]
    .endm

    .macro SET_CNT value
    ldr r0, =\value
    strh r0, [r6, #4]
    .endm

    @ Reads IPCFIFORECV into a slot.
    .macro TAKE slot
    ldr r0, [r5]
    RECORD \slot, r0
    .endm

    .macro CNT slot
    ldrh r0, [r6, #4]
    RECORD \slot, r0
    .endm

    .macro IFREG slot
    ldr r0, [r9, #0x214]
    RECORD \slot, r0
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: IME +0x208, IE +0x210, IF +0x214
    add r6, r9, #0x180         @ IPCSYNC +0, IPCFIFOCNT +4, IPCFIFOSEND +8
    mov r5, #0x04100000        @ IPCFIFORECV
    mov r7, #0                 @ IPCSYNC bit 14, kept by DONE

    @ ---- reset values
    ldrh r0, [r6]
    RECORD 0, r0
    ldrh r0, [r6, #4]
    RECORD 1, r0

    @ ---- step 2: the ARM9's send with its FIFO disabled left nothing here; enable
    WAIT 1
    CNT 2
    ldrh r0, [r6]
    RECORD 3, r0
    SET_CNT 0x8000
    CNT 4
    DONE 2

    @ ---- step 4: the IRQ the ARM9 sent while bit 14 was clear is not here; start listening
    WAIT 3
    IFREG 5
    ldrh r0, [r6]
    RECORD 6, r0
    mov r7, #0x4000
    DONE 4

    @ ---- step 6: the ARM9's second IRQ arrived; acknowledge it and send one back
    WAIT 5
    IFREG 7
    ldrh r0, [r6]
    RECORD 8, r0
    mov r0, #0x00010000
    str r0, [r9, #0x214]
    IFREG 9
    DONE_IRQ 6

    @ ---- step 9: take the three words, then read the empty FIFO and acknowledge the error
    WAIT 8
    CNT 10
    TAKE 11
    TAKE 12
    TAKE 13
    CNT 14
    TAKE 15
    CNT 16
    SET_CNT 0xC000
    CNT 17
    DONE 9

    @ ---- step 11: 16 words waiting (the 17th was dropped by the ARM9's FIFO)
    WAIT 10
    CNT 18
    add r3, r8, #(19 * 4)
    mov r2, #0
take16:
    ldr r0, [r5]
    str r0, [r3], #4
    add r2, r2, #1
    cmp r2, #16
    bne take16
    CNT 35
    TAKE 36
    SET_CNT 0xC000
    DONE 11

    @ ---- step 13: take one of the three words
    WAIT 12
    TAKE 37
    DONE 13

    @ ---- step 15: the ARM9 flushed its send FIFO
    WAIT 14
    CNT 38
    TAKE 39
    CNT 40
    SET_CNT 0xC000
    DONE 15

    @ ---- step 17: reads with the FIFO disabled leave the words in place
    WAIT 16
    mov r0, #0
    strh r0, [r6, #4]
    TAKE 41
    TAKE 42
    CNT 43
    SET_CNT 0x8000
    TAKE 44
    TAKE 45
    CNT 46
    mov r0, #0
    strh r0, [r6, #4]
    TAKE 47
    CNT 48
    SET_CNT 0x8000

    @ ---- step 18: the not-empty IRQ on, FIFO empty: nothing raised
    SET_CNT 0x8400
    IFREG 49
    DONE 18

    @ ---- step 20: one word arrived
    WAIT 19
    IFREG 50
    mov r0, #0x00040000
    str r0, [r9, #0x214]
    IFREG 51
    DONE 20

    @ ---- step 22: a second word arrived while the first is still there; take both
    WAIT 21
    IFREG 52
    TAKE 53
    TAKE 54
    DONE 22

    @ ---- step 24: a word arrived into the drained FIFO; take it, then turn the IRQ off
    WAIT 23
    IFREG 55
    mov r0, #0x00040000
    str r0, [r9, #0x214]
    TAKE 56
    SET_CNT 0x8000
    DONE 24

    @ ---- step 26: a word arrived with the IRQ off; turn it on over the waiting word
    WAIT 25
    IFREG 57
    SET_CNT 0x8400
    IFREG 58
    mov r0, #0x00040000
    str r0, [r9, #0x214]
    TAKE 59
    SET_CNT 0x8000
    DONE 26

    @ ---- step 28: take the ARM9's last word, so its send FIFO goes empty
    WAIT 27
    ldr r0, [r5]
    DONE 28

    @ ---- step 31: take the word: IF.17 on the ARM9
    WAIT 30
    TAKE 60
    DONE 31

    @ ---- step 33: send a word: IF.18 on the ARM9
    WAIT 32
    ldr r0, =0x00000081
    str r0, [r6, #8]
    CNT 61
    DONE 33
    mov r0, #1
    RECORD 62, r0
    ldr r0, =0x49504337
    RECORD 63, r0
spin:
    b spin
    .ltorg
