@ SPDX-License-Identifier: Apache-2.0
@ Poweroff ROM, ARM9 side: own entry point, no libnds, no BlocksDS crt0. Spins forever; the ARM7
@ shuts the console down (docs/hw/touch-power.md, "What the power register does").
    .section .text, "ax"
    .arm
    .global _start
_start:
    b _start
    @ ndstool failed with "Failed to seek footer" on a 4-byte binary; this padding fixed it.
    .space 28, 0
