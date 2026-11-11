@ SPDX-License-Identifier: Apache-2.0
@ Smoke ROM, ARM9 side: own entry point, no libnds, no BlocksDS crt0. Spins forever.
@ Loaded and entered at 0x02000000 (docs/hw/cart-header.md).
    .section .text, "ax"
    .arm
    .global _start
_start:
    b _start
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
