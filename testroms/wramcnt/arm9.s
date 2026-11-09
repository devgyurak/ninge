@ SPDX-License-Identifier: Apache-2.0
@ Shared-WRAM ROM, ARM9 side: sets WRAMCNT to 0, 1, 2 and 3 in turn, writes a marker at
@ 0x03000000 and another at 0x03004000 in each mode, hands over to the ARM7 through a main-RAM
@ flag (there is no IPC yet), and records what it read. No libnds, no crt0. Loaded and entered at
@ 0x02000000 (docs/hw/cart-header.md). The ROM records observed values only; the expected ones
@ live in crates/runner/tests/wramcnt.rs, each citing docs/hw/wramcnt.md.
@
@ Handshake word at 0x02003000: for mode m the ARM9 stores 2m+1 when its part is done and waits
@ for 2m+2, which the ARM7 stores when its part is done. Caches and the protection unit are
@ switched off first, so a flag store is visible to the other CPU at once.
@
@ ARM9 record at 0x02004000, one word per slot; mode m owns slots 8m..8m+5:
@   +0  read of 0x03000000 after the ARM9 wrote its markers
@   +1  read of 0x03004000
@   +2  read of 0x037F8000
@   +3  read of 0x03000000 after the ARM7 wrote its marker
@   +4  read of 0x03004000 after the ARM7 wrote its marker
@   +5  byte read of WRAMCNT (0x04000247) after writing 0xFC | m
@ The markers the ARM9 writes in mode m are 0xA9000000 | m at 0x03000000, then
@ 0xB9000000 | m at 0x03004000.
@ Slots 32 and 33: after WRAMCNT is set back to 0, the reads of 0x03000000 and 0x03004000.
@ Slot 34: progress, the number of modes finished. Slot 35: sentinel, written last.
    .equ REC, 0x02004000
    .equ FLAG, 0x02003000

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r10, =FLAG
    ldr r11, =0x03000000
    ldr r12, =0x04000247       @ WRAMCNT
    add r2, r11, #0x4000       @ 0x03004000
    ldr r3, =0x037F8000
    @ Entry state of CP15 is not stated (direct-boot.md): clear the protection unit enable
    @ (bit 0), data cache enable (bit 2) and instruction cache enable (bit 12).
    mrc p15, 0, r0, c1, c0, 0
    bic r0, r0, #0x00001000
    bic r0, r0, #0x00000005
    mcr p15, 0, r0, c1, c0, 0
    nop
    nop
    nop
    mov r4, #0                 @ the mode
mode_loop:
    mov r5, r4, lsl #5         @ 8 words per mode
    add r5, r8, r5             @ this mode's record
    orr r0, r4, #0xFC          @ bits 2-7 set: the register keeps only the mode
    strb r0, [r12]
    ldrb r0, [r12]
    str r0, [r5, #20]
    ldr r0, =0xA9000000
    orr r0, r0, r4
    str r0, [r11]
    ldr r1, =0xB9000000
    orr r1, r1, r4
    str r1, [r2]
    ldr r0, [r11]
    str r0, [r5]
    ldr r0, [r2]
    str r0, [r5, #4]
    ldr r0, [r3]
    str r0, [r5, #8]
    mov r6, r4, lsl #1
    add r6, r6, #1
    str r6, [r10]              @ hand over: 2m+1
    add r6, r6, #1
wait_arm7:
    ldr r0, [r10]
    cmp r0, r6
    bne wait_arm7              @ until the ARM7 stores 2m+2
    ldr r0, [r11]
    str r0, [r5, #12]
    ldr r0, [r2]
    str r0, [r5, #16]
    add r4, r4, #1
    cmp r4, #4
    bne mode_loop

    mov r0, #0
    strb r0, [r12]             @ all 32 KiB back to the ARM9
    ldr r0, [r11]
    str r0, [r8, #128]         @ slot 32
    ldr r0, [r2]
    str r0, [r8, #132]         @ slot 33
    mov r0, #4
    str r0, [r8, #136]         @ slot 34
    ldr r0, =0x57524D21        @ "WRM!"
    str r0, [r8, #140]         @ slot 35
spin:
    b spin
    .ltorg
