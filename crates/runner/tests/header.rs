// SPDX-License-Identifier: Apache-2.0
//! The headers of our own built test ROMs, parsed from bytes the test reads. `ndstool` wrote
//! their CRCs, so `crc_ok` is an independent check of `crc16` (`docs/hw/cart-header.md`).

use ninge_core::cart::{Header, parse};
use ninge_runner::{load_rom_file, testrom_path};

fn header_of(name: &str) -> Header {
    let rom = load_rom_file(&testrom_path(name)).expect("test ROM is built");
    parse(&rom).expect("our own ROM has a valid header")
}

#[test]
fn given_smoke_rom_when_parsing_its_header_then_the_crc_is_ok() {
    assert!(header_of("smoke").crc_ok);
}

#[test]
fn given_smoke_rom_when_parsing_its_header_then_arm9_ram_and_entry_are_0x02000000() {
    let header = header_of("smoke");
    assert_eq!(
        (header.arm9.ram_address, header.arm9.entry),
        (0x0200_0000, 0x0200_0000)
    );
}

#[test]
fn given_smoke_rom_when_parsing_its_header_then_arm7_ram_and_entry_are_0x02380000() {
    let header = header_of("smoke");
    assert_eq!(
        (header.arm7.ram_address, header.arm7.entry),
        (0x0238_0000, 0x0238_0000)
    );
}

#[test]
fn given_boot_rom_when_parsing_its_header_then_the_crc_is_ok() {
    assert!(header_of("boot").crc_ok);
}

#[test]
fn given_boot_rom_when_parsing_its_header_then_arm9_loads_at_0x02000000_and_enters_at_0x02000100() {
    let header = header_of("boot");
    assert_eq!(
        (header.arm9.ram_address, header.arm9.entry),
        (0x0200_0000, 0x0200_0100)
    );
}

#[test]
fn given_boot_rom_when_parsing_its_header_then_arm7_ram_and_entry_are_0x02380000() {
    let header = header_of("boot");
    assert_eq!(
        (header.arm7.ram_address, header.arm7.entry),
        (0x0238_0000, 0x0238_0000)
    );
}
