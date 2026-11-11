@ SPDX-License-Identifier: Apache-2.0
@ VRAM-banks ROM, ARM7 side: spins forever, like the smoke ROM. Loaded and entered at 0x02380000.
@ The ARM7's own VRAM mapping (banks C and D, VRAMSTAT) joins this ROM with #35's ARM7 step.
    .section .text, "ax"
    .arm
    .global _start
_start:
    b _start
    .space 28, 0
