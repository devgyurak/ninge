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
@
@ Data SWIs, results at RES (one word per line, offsets in bytes; the area starts as AAh bytes):
@   +00 CpuSet copy of three halfwords: 22221111h, AAAA3333h   +10 a 32-bit fill, count 3: DEADBEEFh x3
@   +20 CpuFastSet copy, count 9: words 1-16 (the 17th stays AAAAAAAAh at +60)
@   +80 CpuFastSet fill, count 1: DEADBEEFh x8 (+A0 stays AAAAAAAAh)   +C0 CpuSet count 0: untouched
@   +D0 Div -7/2: quotient, remainder, absolute quotient   +DC Div 7/-2: the same
@   +F0 Sqrt(1000000)   +F4 Sqrt(FFFFFFFFh)   +F8 Sqrt(0)
@   +100 GetCRC16 of "12345678" from FFFFh   +104 IsDebugger (r0 preset to 55h)
@   +110 LZ77 with byte writes: "ABABABABAB" then AAh   +130 LZ77 with halfword writes: the same
@   ARM9 only: +150 first word of VRAM bank A after a byte-write LZ77 into it (the write is lost)
@   +160, +164, +168 three words of VRAM bank A after a halfword-write LZ77 at its +10h
    .equ RES, 0x02009000
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


    @ ---- data SWIs: results in memory at RES (layout above); the area is filled with AAh first
    ldr r10, =RES
    ldr r0, =0xAAAAAAAA
    mov r1, #0
fill_results:
    str r0, [r10, r1]
    add r1, r1, #4
    cmp r1, #0x400
    bne fill_results
    @ CpuSet: three 16-bit units copied
    ldr r0, =src16
    mov r1, r10
    mov r2, #3
    swi 0x0B0000
    @ CpuSet: a 32-bit word filled three times
    ldr r0, =fill_word
    add r1, r10, #0x10
    ldr r2, =0x05000003
    swi 0x0B0000
    @ CpuFastSet: a count of 9 copies 16 words
    ldr r0, =fast_src
    add r1, r10, #0x20
    mov r2, #9
    swi 0x0C0000
    @ CpuFastSet: a fill of count 1 writes 8 words
    ldr r0, =fill_word
    add r1, r10, #0x80
    ldr r2, =0x01000001
    swi 0x0C0000
    @ CpuSet: a count of 0 writes nothing
    ldr r0, =src16
    add r1, r10, #0xC0
    mov r2, #0x04000000
    swi 0x0B0000
    @ Div: -7 / 2 and 7 / -2
    mvn r0, #6
    mov r1, #2
    swi 0x090000
    str r0, [r10, #0xD0]
    str r1, [r10, #0xD4]
    str r3, [r10, #0xD8]
    mov r0, #7
    mvn r1, #1
    swi 0x090000
    str r0, [r10, #0xDC]
    str r1, [r10, #0xE0]
    str r3, [r10, #0xE4]
    @ Sqrt of 1000000, of FFFFFFFFh and of 0
    ldr r0, =1000000
    swi 0x0D0000
    str r0, [r10, #0xF0]
    mvn r0, #0
    swi 0x0D0000
    str r0, [r10, #0xF4]
    mov r0, #0
    swi 0x0D0000
    str r0, [r10, #0xF8]
    @ GetCRC16 of the eight ASCII digits from FFFFh, and IsDebugger with r0 preset
    ldr r0, =0xFFFF
    ldr r1, =digits
    mov r2, #8
    swi 0x0E0000
    str r0, [r10, #0x100]
    mov r0, #0x55
    swi 0x0F0000
    str r0, [r10, #0x104]
    @ LZ77: the same stream decoded with byte writes and with halfword writes
    ldr r0, =lz_stream
    add r1, r10, #0x110
    swi 0x110000
    ldr r0, =lz_stream
    add r1, r10, #0x130
    swi 0x120000

    mov r0, #1
    str r0, [r8, #120]
    ldr r0, =0x600DF00D
    str r0, [r8, #124]
done:
    b done
    .ltorg

    .ltorg
    .balign 4
src16:
    .word 0x22221111, 0x44443333
fill_word:
    .word 0xDEADBEEF
fast_src:
    .word 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20
digits:
    .ascii "12345678"
lz_stream:
    .word 0x00000A10, 0x50424120, 0x00000001

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
