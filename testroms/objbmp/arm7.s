@ SPDX-License-Identifier: Apache-2.0
@ Bitmap-object ROM, ARM7 side: spins forever, like the smoke ROM. Loaded and entered at
@ 0x02380000.
    .section .text, "ax"
    .arm
    .global _start
_start:
    b _start
    .space 28, 0
