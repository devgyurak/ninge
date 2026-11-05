@ SPDX-License-Identifier: Apache-2.0
@ Boot ROM, ARM9 side: records the register state at entry, then spins. No libnds, no crt0.
@ Loaded at 0x02000000, entered at 0x02000100 (docs/hw/direct-boot.md), so a mix-up between the
@ load address and the entry address changes what lands in the record.
@
@ Record at 0x02000200, one word each (offsets in bytes):
@   0x00..0x30  r0..r12 as they were at entry
@   0x34        r13 (sp of the running mode at entry)
@   0x38        r14 at entry
@   0x3C        CPSR at entry
@   0x40        sp_svc
@   0x44        sp_irq
@ Every store is pc-relative, so no register is spent on a base address before it is recorded.
    .section .head, "ax"
    .arm
    .space 0x100, 0

    .section .text, "ax"
    .arm
    .global _start
_start:
    str r0, rec_r0
    str r1, rec_r1
    str r2, rec_r2
    str r3, rec_r3
    str r4, rec_r4
    str r5, rec_r5
    str r6, rec_r6
    str r7, rec_r7
    str r8, rec_r8
    str r9, rec_r9
    str r10, rec_r10
    str r11, rec_r11
    str r12, rec_r12
    str sp, rec_sp
    str lr, rec_lr
    mrs r0, cpsr
    str r0, rec_cpsr
    msr cpsr_c, #0xD3          @ Supervisor, I and F set
    str sp, rec_sp_svc
    msr cpsr_c, #0xD2          @ IRQ, I and F set
    str sp, rec_sp_irq
    msr cpsr_c, #0xDF          @ back to System
spin:
    b spin

    @ The record sits in the same section as the code, because a pc-relative store cannot reach a
    @ label in another section. .org places it 0x100 bytes after the entry point.
    .org 0x100
rec_r0:     .word 0
rec_r1:     .word 0
rec_r2:     .word 0
rec_r3:     .word 0
rec_r4:     .word 0
rec_r5:     .word 0
rec_r6:     .word 0
rec_r7:     .word 0
rec_r8:     .word 0
rec_r9:     .word 0
rec_r10:    .word 0
rec_r11:    .word 0
rec_r12:    .word 0
rec_sp:     .word 0
rec_lr:     .word 0
rec_cpsr:   .word 0
rec_sp_svc: .word 0
rec_sp_irq: .word 0
    .space 28, 0
