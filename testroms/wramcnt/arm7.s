@ SPDX-License-Identifier: Apache-2.0
@ Shared-WRAM ROM, ARM7 side: for each WRAMCNT mode the ARM9 sets, reads what it sees at
@ 0x03000000, 0x03004000, 0x037F8000 and 0x03800000 and the WRAMSTAT byte, writes its own
@ markers, and reads them back. Loaded and entered at 0x02380000 (docs/hw/cart-header.md).
@ No libnds. The ROM records observed values only; the expected ones live in
@ crates/runner/tests/wramcnt.rs (docs/hw/wramcnt.md).
@
@ It waits for the flag word at 0x02003000 to hold 2m+1 (the ARM9 is done with mode m), and
@ stores 2m+2 when it is done.
@
@ ARM7 record at 0x02005000, one word per slot; mode m owns slots 8m..8m+6:
@   +0  read of 0x03000000
@   +1  read of 0x03004000
@   +2  read of 0x037F8000
@   +3  read of 0x03800000
@   +4  byte read of WRAMSTAT (0x04000241)
@   +5  read of 0x03000000 after writing 0xC7000000 | m there
@   +6  read of 0x03800010 after writing 0xD7000000 | m there
@ Slot 32: progress, the number of modes finished. Slot 33: sentinel, written last.
    .equ REC, 0x02005000
    .equ FLAG, 0x02003000

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r8, =REC
    ldr r10, =FLAG
    ldr r11, =0x03000000
    add r2, r11, #0x4000       @ 0x03004000
    ldr r3, =0x037F8000
    ldr r9, =0x03800000
    ldr r12, =0x04000241       @ WRAMSTAT
    mov r4, #0                 @ the mode
mode_loop:
    mov r5, r4, lsl #5         @ 8 words per mode
    add r5, r8, r5             @ this mode's record
    mov r6, r4, lsl #1
    add r6, r6, #1
wait_arm9:
    ldr r0, [r10]
    cmp r0, r6
    bne wait_arm9              @ until the ARM9 stores 2m+1
    ldr r0, [r11]
    str r0, [r5]
    ldr r0, [r2]
    str r0, [r5, #4]
    ldr r0, [r3]
    str r0, [r5, #8]
    ldr r0, [r9]
    str r0, [r5, #12]
    ldrb r0, [r12]
    str r0, [r5, #16]
    ldr r0, =0xC7000000
    orr r0, r0, r4
    str r0, [r11]
    ldr r1, =0xD7000000
    orr r1, r1, r4
    str r1, [r9, #0x10]
    ldr r0, [r11]
    str r0, [r5, #20]
    ldr r0, [r9, #0x10]
    str r0, [r5, #24]
    add r6, r6, #1
    str r6, [r10]              @ hand back: 2m+2
    add r4, r4, #1
    cmp r4, #4
    bne mode_loop

    mov r0, #4
    str r0, [r8, #128]         @ slot 32
    ldr r0, =0x57524D37        @ "WRM7"
    str r0, [r8, #132]         @ slot 33
spin:
    b spin
    .ltorg
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
