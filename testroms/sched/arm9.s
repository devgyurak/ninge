@ SPDX-License-Identifier: Apache-2.0
@ Scheduler ROM, ARM9 side: counts forever, storing the counter at 0x02001000 after every
@ increment. Loaded and entered at 0x02000000 (docs/hw/cart-header.md). No libnds.
@
@ Instruction budget per frame (docs/hw/scheduler.md, "Cost per instruction"): the ARM9 runs
@ 1,120,380 instructions. The prologue is 3 instructions and the loop 3, so after N frames the
@ counter is (N * 1,120,380 - 3) / 3 (expected values live in crates/runner/tests/scheduler.rs).
    .section .text, "ax"
    .arm
    .global _start
_start:
    mov r1, #0x02000000
    orr r1, r1, #0x1000
    mov r0, #0
loop:
    add r0, r0, #1
    str r0, [r1]
    b loop
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
