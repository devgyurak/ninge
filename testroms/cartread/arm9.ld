/* SPDX-License-Identifier: Apache-2.0 */
ENTRY(_start)
SECTIONS
{
    . = 0x02000000;
    .text : { *(.text*) }
    /* The data blob starts 4000h bytes into the ARM9 binary, which the header places at ROM offset
       4000h, so the blob is at ROM offset 8000h: the first offset a B7 read can reach. */
    . = 0x02004000;
    .blob : { *(.blob*) }
    /DISCARD/ : { *(.ARM.attributes) *(.comment) }
}
