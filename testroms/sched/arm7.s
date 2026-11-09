@ SPDX-License-Identifier: Apache-2.0
@ Scheduler ROM, ARM7 side: counts forever, storing the counter at 0x02002000 after every
@ increment. Loaded and entered at 0x02380000 (docs/hw/cart-header.md). No libnds.
@
@ Instruction budget per frame (docs/hw/scheduler.md, "Cost per instruction"): the ARM7 runs
@ 560,190 instructions. The prologue is 3 instructions and the loop 4 (the `mov r2, r2` is
@ padding so the loop length differs from the ARM9's). With L = N * 560,190 - 3 loop
@ instructions after N frames, each full iteration stores once and a partial one stores only
@ when it got past its `str` (2 or 3 instructions), so the counter is floor((L + 2) / 4)
@ (expected values live in crates/runner/tests/scheduler.rs).
    .section .text, "ax"
    .arm
    .global _start
_start:
    mov r1, #0x02000000
    orr r1, r1, #0x2000
    mov r0, #0
loop:
    add r0, r0, #1
    str r0, [r1]
    mov r2, r2
    b loop
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
