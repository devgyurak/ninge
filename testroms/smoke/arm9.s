@ SPDX-License-Identifier: Apache-2.0
@ Smoke ROM, ARM9 side: own entry point, no libnds, no BlocksDS crt0. Spins forever.
@ Loaded and entered at 0x02000000 (docs/hw/cart-header.md).
    .section .text, "ax"
    .arm
    .global _start
_start:
    b _start
    @ ndstool probes 12 bytes before the end of each binary for a footer; pad past that.
    .space 28, 0
