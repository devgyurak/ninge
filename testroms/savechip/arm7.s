@ SPDX-License-Identifier: Apache-2.0
@ Save-chip ROM, ARM7 side: nothing to do. The ARM9 owns the cartridge slot (EXMEMCNT bit 11
@ clear) and does every transfer. Loaded and entered at 0x02380000 (docs/hw/cart-header.md).
    .section .text, "ax"
    .arm
    .global _start
_start:
spin:
    b spin
