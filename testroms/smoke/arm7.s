@ SPDX-License-Identifier: Apache-2.0
@ Smoke ROM, ARM7 side: own entry point, no libnds, no default ARM7 core. Spins forever.
@ Loaded and entered at 0x02380000 (docs/hw/cart-header.md).
    .section .text, "ax"
    .arm
    .global _start
_start:
    b _start
    @ ndstool probes 12 bytes before the end of each binary for a footer; pad past that.
    .space 28, 0
