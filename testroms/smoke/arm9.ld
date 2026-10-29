/* SPDX-License-Identifier: Apache-2.0 */
ENTRY(_start)
SECTIONS
{
    . = 0x02000000;
    .text : { *(.text*) }
    /DISCARD/ : { *(.ARM.attributes) *(.comment) }
}
