@ SPDX-License-Identifier: Apache-2.0
@ Interrupt-controller ROM, ARM7 side: the same probes as the ARM9 side on the ARM7's own IME,
@ IE and IF, and a read of its IE after the ARM9 has written a pattern to the ARM9's IE. The I
@ flag stays set and nothing requests an interrupt. Loaded and entered at 0x02380000
@ (docs/hw/cart-header.md). No libnds. The ROM records observed values only; the expected ones
@ live in crates/runner/tests/irq.rs (docs/hw/interrupts.md). The handshake is described at the
@ top of arm9.s: this side stores 1 after its reset reads, waits for 2, and stores 3 when done.
@
@ ARM7 record at 0x02005000, one word per slot:
@   0  IME read at reset            1  IE read at reset             2  IF read at reset
@   3  IME after writing FFFFFFFF   4  IE after writing FFFFFFFF   5  IF after that
@   6  IE after a byte write of FF to its low byte (IE was 0)
@   7  IE after a halfword write of FFFF to its high half (IE was 0)
@   8  IF after writing FFFFFFFF
@   9  IME after a byte write of FF to byte 1 (IME was 0)
@   10 IE read after the ARM9 wrote its pattern, before the ARM7 wrote its IE
@   11 IE read right after writing AAAAAAAA to it
@ Slot 14: progress (1 when done). Slot 15: sentinel, written last.
    .equ REC, 0x02005000
    .equ FLAG, 0x02003000

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r10, =FLAG
    ldr r12, =0x04000000       @ I/O base: IME +0x208, IE +0x210, IF +0x214
    ldr r0, [r12, #0x208]
    str r0, [r8, #0]
    ldr r0, [r12, #0x210]
    str r0, [r8, #4]
    ldr r0, [r12, #0x214]
    str r0, [r8, #8]
    mov r0, #1
    str r0, [r10]              @ the ARM9 may write its registers now
wait_arm9:
    ldr r0, [r10]
    cmp r0, #2
    bne wait_arm9
    ldr r0, [r12, #0x210]
    str r0, [r8, #40]          @ slot 10
    mvn r1, #0                 @ FFFFFFFF
    mov r2, #0
    str r1, [r12, #0x208]
    ldr r0, [r12, #0x208]
    str r0, [r8, #12]
    str r1, [r12, #0x210]
    ldr r0, [r12, #0x210]
    str r0, [r8, #16]
    ldr r0, [r12, #0x214]
    str r0, [r8, #20]
    str r2, [r12, #0x210]
    strb r1, [r12, #0x210]
    ldr r0, [r12, #0x210]
    str r0, [r8, #24]
    str r2, [r12, #0x210]
    add r9, r12, #0x200
    strh r1, [r9, #0x12]       @ IE + 2
    ldr r0, [r12, #0x210]
    str r0, [r8, #28]
    str r1, [r12, #0x214]
    ldr r0, [r12, #0x214]
    str r0, [r8, #32]
    str r2, [r12, #0x208]
    strb r1, [r12, #0x209]     @ IME byte 1
    ldr r0, [r12, #0x208]
    str r0, [r8, #36]
    str r2, [r12, #0x210]
    ldr r3, =0xAAAAAAAA
    str r3, [r12, #0x210]
    ldr r0, [r12, #0x210]
    str r0, [r8, #44]
    str r2, [r12, #0x208]      @ IME off again
    mov r0, #1
    str r0, [r8, #56]          @ slot 14
    ldr r0, =0x49525137        @ "IRQ7"
    str r0, [r8, #60]          @ slot 15
    mov r0, #3
    str r0, [r10]              @ hand back: done
spin:
    b spin
    .ltorg
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
