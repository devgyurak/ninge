/* SPDX-License-Identifier: Apache-2.0 */
ENTRY(_start)
SECTIONS
{
    . = 0x02000000;
    .head : { *(.head) }
    .text 0x02000100 : { *(.text*) }
    /DISCARD/ : { *(.ARM.attributes) *(.comment) }
}
