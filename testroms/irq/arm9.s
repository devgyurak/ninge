@ SPDX-License-Identifier: Apache-2.0
@ Interrupt-controller ROM, ARM9 side: reads IME, IE and IF at reset, writes all ones to IME, IE
@ and IF, probes byte and halfword writes to IE and IME, writes a pattern to IE and, after the
@ ARM7 has done the same with its own registers, reads IE again. Nothing requests an interrupt,
@ and the I flag stays set, so no IRQ is delivered. No libnds, no crt0. Loaded and entered at
@ 0x02000000 (docs/hw/cart-header.md). The ROM records observed values only; the expected ones
@ live in crates/runner/tests/irq.rs, each citing docs/hw/interrupts.md.
@
@ Handshake word at 0x02003000 (there is no IPC yet), which only grows:
@   the ARM7 stores 1 once it has read its reset values;
@   the ARM9 waits for 1, runs its sequence, then stores 2;
@   the ARM7 waits for 2, reads its own IE, runs its sequence, then stores 3;
@   the ARM9 waits for 3, then reads its own IE again.
@ Caches and the protection unit are switched off first, so a flag store is visible to the other
@ CPU at once.
@
@ ARM9 record at 0x02004000, one word per slot:
@   0  IME read at reset            1  IE read at reset             2  IF read at reset
@   3  IME after writing FFFFFFFF   4  IE after writing FFFFFFFF   5  IF after that
@   6  IE after a byte write of FF to its low byte (IE was 0)
@   7  IE after a halfword write of FFFF to its high half (IE was 0)
@   8  IF after writing FFFFFFFF
@   9  IME after a byte write of FF to byte 1 (IME was 0)
@   10 IE read after the ARM7 wrote its own pattern to its IE
@   11 IE read right after writing 55555555 to it
@ Slot 14: progress (1 when done). Slot 15: sentinel, written last.
    .equ REC, 0x02004000
    .equ FLAG, 0x02003000

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r10, =FLAG
    ldr r12, =0x04000000       @ I/O base: IME +0x208, IE +0x210, IF +0x214
    @ Entry state of CP15 is not stated (direct-boot.md): clear the protection unit enable
    @ (bit 0), data cache enable (bit 2) and instruction cache enable (bit 12).
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    ldr r0, [r12, #0x208]
    str r0, [r8, #0]
    ldr r0, [r12, #0x210]
    str r0, [r8, #4]
    ldr r0, [r12, #0x214]
    str r0, [r8, #8]
wait_arm7_reset:
    ldr r0, [r10]
    cmp r0, #1
    bne wait_arm7_reset        @ until the ARM7 has read its reset values
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
    ldr r3, =0x55555555
    str r3, [r12, #0x210]
    ldr r0, [r12, #0x210]
    str r0, [r8, #44]
    str r2, [r12, #0x208]      @ IME off again
    mov r0, #2
    str r0, [r10]              @ hand over: the ARM7's turn
wait_arm7_done:
    ldr r0, [r10]
    cmp r0, #3
    bne wait_arm7_done
    ldr r0, [r12, #0x210]
    str r0, [r8, #40]
    mov r0, #1
    str r0, [r8, #56]          @ slot 14
    ldr r0, =0x49525139        @ "IRQ9"
    str r0, [r8, #60]          @ slot 15
spin:
    b spin
    .ltorg
