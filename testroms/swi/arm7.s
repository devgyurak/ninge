@ SPDX-License-Identifier: Apache-2.0
@ HLE BIOS ROM, ARM7 side: the same sequence as the ARM9 side on the ARM7's own BIOS. The ARM7's
@ vectors are always in the BIOS, and its user handler pointer is at 0x0380FFFC
@ (docs/hw/hle-swi.md, "The IRQ dispatcher"). This is the first test ROM that takes an ARM7
@ interrupt in the ROM rather than only latching IF. Loaded and entered at 0x02380000
@ (docs/hw/cart-header.md). No libnds. The ROM records observed values only; the expected ones
@ live in crates/runner/tests/swi.rs.
@
@ Record at 0x02005000, one word per slot (the slots are the ARM9 side's, see arm9.s):
@   0  VBlank IRQs the user handler took (the ROM waits for 3)
@   1-5  r0, r1, r2, r3, r12 after those IRQs      6  CPSR low byte inside the user handler
@   7  r0 at the user handler's entry              8  SP_irq after the last handler returned
@   10 1 when an ARM SWI 1Eh returned              11 r6 after a Thumb SWI 1Eh
@   12 CPSR flags nibble after the ARM SWI
@   13 1 when VCOUNT >= 192 after VBlankIntrWait returned  14 IRQs the handler took by then (4)
@   15 IME after VBlankIntrWait (forced to 1)   16 CPSR low byte after it (the caller's, 9Fh)
@   17 IF bit 0 on waking from SWI Halt with IME = 1 and I set   18 the same with IME = 0
@   19 IF bit 0 on waking from a HALTCNT write with IME = 0
@   30 progress (1 when done)  31 sentinel, written last
    .equ REC, 0x02005000
    .equ FLAGS, 0x0380FFF8
    .equ COUNT, 0x02007000

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: DISPSTAT +4, IME +0x208, IE +0x210, IF +0x214
    ldr r0, =0x0380FFFC
    adr r1, handler
    str r1, [r0]
    ldr r0, =COUNT
    mov r1, #0
    str r1, [r0]
    mov r0, #8                 @ DISPSTAT bit 3: VBlank IRQ enable
    strh r0, [r9, #4]
    mov r0, #1
    str r0, [r9, #0x210]       @ IE bit 0
    str r0, [r9, #0x208]       @ IME
    mov r0, #0x11
    mov r1, #0x22
    mov r2, #0x33
    mov r3, #0x44
    mov r12, #0x55
    msr cpsr_c, #0x1F          @ I clear: the IRQs start
wait_irqs:
    ldr r4, =COUNT
    ldr r5, [r4]
    cmp r5, #3
    bne wait_irqs
    msr cpsr_c, #0xDF
    mov r4, #0
    str r4, [r9, #0x208]
    str r5, [r8, #0]
    str r0, [r8, #4]
    str r1, [r8, #8]
    str r2, [r8, #12]
    str r3, [r8, #16]
    str r12, [r8, #20]
    msr cpsr_c, #0xD2
    mov r4, sp
    msr cpsr_c, #0xDF
    str r4, [r8, #32]

    msr cpsr_f, #0xA0000000
    swi 0x1E0000
    mrs r4, cpsr
    mov r4, r4, lsr #28
    str r4, [r8, #48]
    mov r4, #1
    str r4, [r8, #40]

    adr r4, thumb_code
    orr r4, r4, #1
    bx r4
    .thumb
thumb_code:
    mov r6, #0
    swi 0x1E
    mov r6, #1
    adr r4, back_to_arm
    bx r4
    .align 2
    .arm
back_to_arm:
    str r6, [r8, #44]


    @ ---- 13-16: VBlankIntrWait with IME = 0 and I set at the call (F clear). It forces IME to 1 and
    @ waits for the next V-blank (the old flags are discarded); the caller's CPSR comes back.
    mov r4, #0
    str r4, [r9, #0x208]
    msr cpsr_c, #0x9F          @ System mode, I set, F clear: the IRQ in the wait enters as 92h
    swi 0x050000
    ldrh r4, [r9, #6]          @ VCOUNT
    cmp r4, #192
    movge r4, #1
    movlt r4, #0
    str r4, [r8, #52]          @ 13: VCOUNT >= 192 on return
    ldr r4, =COUNT
    ldr r4, [r4]
    str r4, [r8, #56]          @ 14: the handler has taken 4 IRQs
    ldr r4, [r9, #0x208]
    str r4, [r8, #60]          @ 15: IME was forced to 1
    mrs r4, cpsr
    and r4, r4, #0xFF
    str r4, [r8, #64]          @ 16: CPSR low byte as before the call
    msr cpsr_c, #0xDF

    @ ---- 17-18: SWI Halt wakes on IE & IF whatever IME and the I flag say
    mov r4, #1
    str r4, [r9, #0x214]       @ acknowledge V-blank
    swi 0x060000
    ldr r4, [r9, #0x214]
    and r4, r4, #1
    str r4, [r8, #68]          @ 17: IF bit 0 set on the wake (IME = 1, I set)
    mov r4, #1
    str r4, [r9, #0x214]
    mov r4, #0
    str r4, [r9, #0x208]       @ IME = 0
    swi 0x060000
    ldr r4, [r9, #0x214]
    and r4, r4, #1
    str r4, [r8, #72]          @ 18: IF bit 0 set on the wake (IME = 0)

    @ ---- 19: the CPU's own halt
    mov r4, #1
    str r4, [r9, #0x214]
    mov r0, #0x80
    strb r0, [r9, #0x301]      @ HALTCNT: mode 2, halt
    ldr r4, [r9, #0x214]
    and r4, r4, #1
    str r4, [r8, #76]          @ 19: IF bit 0 set on the wake (IME = 0)

    mov r0, #1
    str r0, [r8, #120]
    ldr r0, =0x600DF00D
    str r0, [r8, #124]
done:
    b done
    .ltorg

handler:
    ldr r1, =REC
    str r0, [r1, #28]
    mrs r2, cpsr
    and r2, r2, #0xFF
    str r2, [r1, #24]
    mov r0, #0x04000000
    mov r2, #1
    str r2, [r0, #0x214]
    ldr r2, =FLAGS
    ldr r3, [r2]
    orr r3, r3, #1             @ the IRQ check flags: V-blank happened (IntrWait looks here)
    str r3, [r2]
    ldr r2, =COUNT
    ldr r3, [r2]
    add r3, r3, #1
    str r3, [r2]
    mov r0, #0xEE
    mov r1, #0xEE
    mov r2, #0xEE
    mov r3, #0xEE
    mov r12, #0xEE
    bx lr
    .ltorg
