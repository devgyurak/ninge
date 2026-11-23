@ SPDX-License-Identifier: Apache-2.0
@ spupcm ROM, ARM9 side: own entry point, no libnds. The ARM7 makes all the sound (arm7.s); this side only
@ spins, so both screens stay as the machine leaves them and the golden line's audio= field is what
@ the ROM is about. Loaded and entered at 0x02000000 (docs/hw/cart-header.md).
    .section .text, "ax"
    .arm
    .global _start
_start:
    b _start
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
