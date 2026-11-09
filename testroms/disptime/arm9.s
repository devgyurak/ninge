@ SPDX-License-Identifier: Apache-2.0
@ Display-timing ROM, ARM9 side: records VCOUNT over a frame and DISPSTAT and IF at chosen
@ lines, from ARM9's own view. The two CPUs run the same sequence independently, each polling
@ VCOUNT, so there is no handshake. No libnds, no crt0. Loaded and entered at 0x02000000
@ (docs/hw/cart-header.md). The ROM records observed values only; the expected ones live in
@ crates/runner/tests/display_timing.rs, each citing docs/hw/display-timing.md.
@
@ Record at 0x02004000, one word per slot:
@   0  DISPSTAT before anything wrote it
@   1  DISPSTAT just after VCOUNT reads 100      2  ... 192      3  ... 261
@   4  DISPSTAT just after VCOUNT reads 262      5  ... 0 (the wrap)
@   6  DISPSTAT when the HBlank flag is first seen in line 50
@   7  VCOUNT at that moment                     8  VCOUNT when the flag clears again
@   9  DISPSTAT with the setting 100, just after VCOUNT reads 99    10  ... 100    11  ... 101
@   12 DISPSTAT after writing FFFF at line 5
@   13 IF at line 125 (IME = 0, IE = 7, DISPSTAT: setting 120, all enables, IF cleared before)
@   14 IF after writing 2 to it
@   15 IF at line 195 after clearing IF at line 191
@   16 IF after writing 1 to it
@   20 IRQs the ARM9 handler took (vector at 0x18)   21 OR of the IF values it saw
@   30 progress (1 when done)  31 sentinel, written last
@ VCOUNT sequence at 0x02010000: 264 halfwords, each new value polled over one frame.
    .equ REC, 0x02004000
    .equ SEQ, 0x02010000

    .macro at_line line, slot
    ldr r0, =\line
    bl wait_line
    ldrh r1, [r9, #4]
    str r1, [r8, #(\slot * 4)]
    .endm

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    mov r9, #0x04000000        @ I/O base: DISPSTAT +4, VCOUNT +6, IME +0x208, IE +0x210, IF +0x214
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
    @ The ITCM is off after direct boot (direct-boot.md): give it 32 KiB (C9,C1,1 size field 6)
    @ and enable it (control bit 18), then copy the IRQ handler to its vector at 0x18.
    mov r0, #0x0C
    mcr p15, 0, r0, c9, c1, 1
    mrc p15, 0, r0, c1, c0, 0
    orr r0, r0, #0x00040000
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    adr r0, handler_start
    adr r2, handler_end
    mov r1, #0x18
copy_handler:
    ldr r3, [r0], #4
    str r3, [r1], #4
    cmp r0, r2
    bne copy_handler
    ldr r0, =0x02006000        @ handler counters: IRQ count, OR of the IF values it saw
    mov r1, #0
    str r1, [r0]
    str r1, [r0, #4]

    @ ---- 0: DISPSTAT as it reads before anything wrote it
    ldrh r0, [r9, #4]
    str r0, [r8, #0]

    @ ---- VCOUNT sequence: every new value over one frame, wrap included (264 halfwords)
    ldr r6, =SEQ
    ldrh r4, [r9, #6]
    mov r5, #0
seq_loop:
    ldrh r0, [r9, #6]
    cmp r0, r4
    beq seq_loop
    mov r4, r0
    strh r0, [r6, r5]
    add r5, r5, #2
    cmp r5, #528
    bne seq_loop

    @ ---- 12: a write of FFFF keeps only the enables and the setting (line 5: no flag is set,
    @ and the setting 511 matches no line)
    mov r0, #5
    bl wait_line
    ldr r0, =0xFFFF
    strh r0, [r9, #4]
    ldrh r0, [r9, #4]
    str r0, [r8, #48]
    mov r0, #0
    strh r0, [r9, #4]

    @ ---- 6-8: HBlank flag sets in line 50 and clears at line 51
    mov r0, #50
    bl wait_line
wait_hblank:
    ldrh r0, [r9, #4]
    tst r0, #2
    beq wait_hblank
    str r0, [r8, #24]
    ldrh r1, [r9, #6]
    str r1, [r8, #28]
wait_hblank_end:
    ldrh r0, [r9, #4]
    tst r0, #2
    bne wait_hblank_end
    ldrh r1, [r9, #6]
    str r1, [r8, #32]

    @ ---- 9-11: VCount-match flag with the setting 100
    mov r0, #0x6400
    strh r0, [r9, #4]
    at_line 99, 9
    at_line 100, 10
    at_line 101, 11
    mov r0, #0
    strh r0, [r9, #4]

    @ ---- 13-14: IF latches HBlank and VCount-match with IME = 0, and clears by writing 1
    ldr r0, =0x7838            @ setting 120, all three IRQ enables
    strh r0, [r9, #4]
    mov r0, #7
    str r0, [r9, #0x210]       @ IE
    mov r0, #0
    str r0, [r9, #0x208]       @ IME
    mov r0, #7
    str r0, [r9, #0x214]       @ clear IF
    mov r0, #125
    bl wait_line
    ldr r0, [r9, #0x214]
    str r0, [r8, #52]
    mov r0, #2
    str r0, [r9, #0x214]       @ clear the HBlank bit only
    ldr r0, [r9, #0x214]
    str r0, [r8, #56]

    @ ---- 15-16: IF latches VBlank (line 192) and clears by writing 1
    mov r0, #191
    bl wait_line
    mov r0, #7
    str r0, [r9, #0x214]
    mov r0, #195
    bl wait_line
    ldr r0, [r9, #0x214]
    str r0, [r8, #60]
    mov r0, #1
    str r0, [r9, #0x214]
    ldr r0, [r9, #0x214]
    str r0, [r8, #64]
    mov r0, #0
    strh r0, [r9, #4]
    str r0, [r9, #0x210]
    mov r0, #7
    str r0, [r9, #0x214]

    @ ---- 1-5: the VBlank flag around lines 100, 192, 261, 262 and the wrap to 0
    at_line 100, 1
    at_line 192, 2
    at_line 261, 3
    at_line 262, 4
    at_line 0, 5


    @ ---- 20-21: the ARM9 takes the VCount and VBlank IRQs through the vector at 0x18
    mov r0, #5
    bl wait_line
    mov r0, #7
    str r0, [r9, #0x214]       @ clear IF
    ldr r0, =0x1E28            @ setting 30, VBlank and VCount enables
    strh r0, [r9, #4]
    mov r0, #5
    str r0, [r9, #0x210]       @ IE = VBlank | VCount
    mov r0, #1
    str r0, [r9, #0x208]       @ IME
    mov r0, #0x1F
    msr cpsr_c, r0             @ I clear, System mode
    mov r0, #200
    bl wait_line
    mov r0, #0x9F
    msr cpsr_c, r0             @ I set again
    mov r0, #0
    str r0, [r9, #0x208]
    strh r0, [r9, #4]
    ldr r0, =0x02006000
    ldr r1, [r0]
    str r1, [r8, #80]
    ldr r1, [r0, #4]
    str r1, [r8, #84]
    mov r0, #1
    str r0, [r8, #120]         @ slot 30
    ldr r0, =0x44495339
    str r0, [r8, #124]         @ slot 31
spin:
    b spin

@ Polls VCOUNT until it reads r0.
wait_line:
    ldrh r1, [r9, #6]
    cmp r1, r0
    bne wait_line
    mov pc, lr

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
