// SPDX-License-Identifier: Apache-2.0
// libnds console ROM: the Phase 3 test program. It puts engine A's main BG on a text console
// (consoleInit, not consoleDemoInit, which uses engine B) and prints three lines with printf, then a line after five VBlank waits (the ARM9 VBlank interrupt is delivered) and a line after a FIFO round trip with the ARM7 (it answers a battery request). The
// expected text is in crates/runner/tests/console.rs. Our own source; the font and the SDK come
// from the BlocksDS image at build time and are never copied into this repository.
#include <nds.h>
#include <stdio.h>

int main(void)
{
    videoSetMode(MODE_0_2D);
    vramSetBankA(VRAM_A_MAIN_BG);
    consoleInit(NULL, 0, BgType_Text4bpp, BgSize_T_256x256, 31, 0, true, true);

    printf("ninge console test\n");
    printf("sum: %d + %d = %d\n", 20, 22, 20 + 22);
    printf("hex: %08X\n", 0xC0FFEE);

    // Five VBlank interrupts must arrive before this line prints.
    for (int i = 0; i < 5; i++)
        swiWaitForVBlank();
    printf("vblank: 5\n");

    // The ARM7 must answer over the FIFO; the call blocks until it does. The value is not printed.
    (void)getBatteryLevel();
    printf("fifo: ok\n");

    while (1)
        swiWaitForVBlank();
}
