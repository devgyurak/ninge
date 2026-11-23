@ SPDX-License-Identifier: Apache-2.0
@ PSG and noise test ROM, ARM7 side (docs/hw/spu.md, "PSG", "Noise"). No libnds, no crt0. Loaded and
@ entered at 0x02380000 (docs/hw/cart-header.md).
@
@ Three channels at TMR FE00h (one position or step per output sample), all at volume 32:
@   8   PSG, duty 3 (four LOW then four HIGH), panning 0 (all left).
@   9   PSG, duty 6 (one LOW then seven HIGH), panning 127 (all right).
@   14  noise, panning 64 (half to each side).
@ Master volume 127 with the master enable, bias 200h.
    .equ SND8, 0x04000480
    .equ SND9, 0x04000490
    .equ SND14, 0x040004E0
    .equ SOUNDCNT, 0x04000500
    .equ SOUNDBIAS, 0x04000504
    .equ TMR, 0xFE00

    .section .text, "ax"
    .arm
    .global _start
_start:
    ldr r0, =SOUNDCNT
    ldr r1, =0x807F
    strh r1, [r0]
    ldr r0, =SOUNDBIAS
    ldr r1, =0x200
    strh r1, [r0]

    ldr r0, =SND8
    ldr r1, =TMR
    strh r1, [r0, #8]
    ldr r2, =SND9
    strh r1, [r2, #8]
    ldr r3, =SND14
    strh r1, [r3, #8]

    @ Start: bit 31, format 3 (PSG / noise, 3 << 29), the duty in bits 24-26, the panning in
    @ bits 16-22, the volume in bits 0-6. The repeat mode and the length do not matter for PSG.
    ldr r1, =0xE3000020         @ PSG, duty 3, volume 32, panning 0
    str r1, [r0]
    ldr r1, =0xE67F0020         @ PSG, duty 6, volume 32, panning 127
    str r1, [r2]
    ldr r1, =0xE0400020         @ noise, volume 32, panning 64
    str r1, [r3]
spin:
    b spin
    .ltorg
