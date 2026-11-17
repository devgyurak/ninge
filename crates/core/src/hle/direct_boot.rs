// SPDX-License-Identifier: Apache-2.0
//! Direct boot: places the binaries and starts both CPUs at their entry points. Note:
//! `docs/hw/direct-boot.md`.

use crate::bus::{Arm7View, Arm9View, Memory, TcmConfig};
use crate::cart::{BinarySpec, HEADER_LEN, Header};
use crate::cpu::{ARM9_INSTRUCTION_COST, Bank, Bus, Cpu};
use crate::io::Io;
use crate::scheduler::Scheduler;
use crate::spi::firmware::USER_COPY_LEN;
use crate::spi::newest_user_settings;

/// WRAMCNT, and the mode that gives all of the shared WRAM to the ARM7 (`wramcnt.md`).
const WRAMCNT_ADDR: u32 = 0x0400_0247;
const WRAMCNT_ARM7_ALL: u8 = 3;
/// Where the loader keeps the header copy (`direct-boot.md`, "Placement").
const HEADER_COPY_ADDRESS: u32 = 0x027F_FE00;

/// Register values at cartridge boot time (`direct-boot.md`, "ARM9 state at entry").
const SP_SYSTEM: u32 = 0x0300_2F7C;
const SP_IRQ: u32 = 0x0300_3F80;
const SP_SUPERVISOR: u32 = 0x0300_3FC0;
/// CPSR: System mode, ARM state, I and F set. Chosen so the machine can run; GBATEK does not
/// state it.
// TODO(hw-unverified): see `direct-boot.md`, "Unverified".
const CPSR_AT_ENTRY: u32 = 0x0000_00DF;
/// The ARM7's stacks (`direct-boot.md`, "ARM9 state at entry"; "ARM7 state at entry").
const ARM7_SP_SYSTEM: u32 = 0x0380_FD80;
const ARM7_SP_IRQ: u32 = 0x0380_FF80;
const ARM7_SP_SUPERVISOR: u32 = 0x0380_FFC0;

const STACK_REGISTER: usize = 13;
const LINK_REGISTER: usize = 14;
const IP_REGISTER: usize = 12;

/// Copies `bytes` to `address` through the ARM9 data bus, so they land where the ARM9 would see
/// them.
fn place(view: &mut Arm9View, address: u32, bytes: &[u8]) {
    bytes.iter().enumerate().for_each(|(index, &byte)| {
        view.write8(address.wrapping_add(index as u32), byte);
    });
}

/// Copies `bytes` to `address` through the ARM7 data bus: main RAM, or the `037F_8000h` range,
/// which lands in shared or ARM7 WRAM as WRAMCNT says (`wramcnt.md`).
fn place7(view: &mut Arm7View, address: u32, bytes: &[u8]) {
    bytes.iter().enumerate().for_each(|(index, &byte)| {
        view.write8(address.wrapping_add(index as u32), byte);
    });
}

/// Where the BIOS leaves the cartridge's chip ID (twice, from the first and second ID command) and
/// the firmware's copy of both (`direct-boot.md`, "The chip ID").
const CHIP_ID_ADDRESSES: [u32; 4] = [0x027F_F800, 0x027F_F804, 0x027F_FC00, 0x027F_FC04];

/// Header offsets the boot-time values copy (`cart-header.md`, "Layout").
const HEADER_SECURE_CRC: usize = 0x6C;
const HEADER_CRC: usize = 0x15E;

/// Where the user-settings copy starts, and how long it is (`direct-boot.md`, "Boot-time values in
/// main RAM").
const USER_SETTINGS_COPY: u32 = 0x027F_FC80;

/// The firmware header's offsets of the user-settings address (divided by 8) and the part 5 CRC.
const FIRMWARE_SETTINGS_ADDRESS: usize = 0x20;
const FIRMWARE_PART5_CRC: usize = 0x26;

/// One value the BIOS and firmware leave in main RAM: `bytes` wide (2 or 4) at `address`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BootValue {
    pub(super) address: u32,
    pub(super) bytes: u8,
    pub(super) value: u32,
}

const fn half(address: u32, value: u16) -> BootValue {
    BootValue {
        address,
        bytes: 2,
        value: value as u32,
    }
}

const fn word(address: u32, value: u32) -> BootValue {
    BootValue {
        address,
        bytes: 4,
        value,
    }
}

/// A little-endian halfword of `bytes` at `at`, 0 outside the slice (the callers check the length
/// where it matters).
fn le16(bytes: &[u8], at: usize) -> u16 {
    bytes
        .get(at..at.saturating_add(2))
        .and_then(|b| b.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

/// The values the BIOS and the firmware's boot code leave in `027FF800h`-`027FF893h` and
/// `027FFC00h`-`027FFC41h`, and the newest user-settings area's first bytes for `027FFC80h`
/// (`direct-boot.md`, "Boot-time values in main RAM"). The chip IDs are written apart (they depend
/// on the card). Zero rows are not listed: memory starts zero.
///
/// `firmware` is the firmware image the machine holds (`firmware.md`).
// TODO(hw-unverified): every value GBATEK states as a cartridge-boot-time constant below (the boot
// handler task, the BIOS CRC, the boot messages, the boot flags, the boot indicator) is its claim, not
// checked on hardware; so is the "somewhat copy" of the ARM7 RAM address at `027FF860h`.
// TODO(hw-unverified): left zero because only a BIOS or a dump could give them: secure disable
// (`027FF812h`, `027FFC12h`), SIO debug (`027FF814h`, `027FFC14h`), RTC status (`027FF816h`,
// `027FFC16h`), the random byte (`027FF818h`, `027FFC17h`; zero for determinism), the GBA-slot
// fields (`027FFC30h`-`027FFC3Bh`), the frame counter (`027FFC3Ch`) and the firmware boot-code
// fragments (`023FEE00h`, `0380F700h`, `0380F980h`).
// TODO(hw-unverified): the secure-area-bad flag is 0 because direct boot never checks the secure
// area.
pub(super) fn boot_values(
    rom: &[u8],
    header: &Header,
    firmware: &[u8],
) -> (Vec<BootValue>, Vec<u8>) {
    let header_crc = le16(rom, HEADER_CRC);
    let secure_crc = le16(rom, HEADER_SECURE_CRC);
    let missing_or_bad = u16::from(!header.crc_ok);
    let settings = newest_user_settings(firmware);
    let settings_address = u32::from(le16(firmware, FIRMWARE_SETTINGS_ADDRESS)) * 8;
    let mut values = vec![
        half(0x027F_F808, header_crc),
        half(0x027F_FC08, header_crc),
        half(0x027F_F80A, secure_crc),
        half(0x027F_FC0A, secure_crc),
        half(0x027F_F80C, missing_or_bad),
        half(0x027F_FC0C, missing_or_bad),
        // TODO(hw-unverified): the boot handler task number, GBATEK's cartridge-boot constant.
        half(0x027F_F810, 0xFFFF),
        half(0x027F_F850, NDS7_BIOS_CRC),
        half(0x027F_FC10, NDS7_BIOS_CRC),
        word(0x027F_F860, header.arm7.ram_address),
        // The firmware's own settings flag: bad when neither area has a valid CRC.
        word(0x027F_F864, u32::from(settings.is_none())),
        word(0x027F_F868, settings_address),
        half(0x027F_F874, le16(firmware, FIRMWARE_PART5_CRC)),
        // TODO(hw-unverified): the two boot messages, the boot flags ("somewhat") and the boot
        // indicator are GBATEK's cartridge-boot constants.
        word(0x027F_F880, 7),
        word(0x027F_F884, 6),
        word(0x027F_F890, 0xB000_2A22),
        half(0x027F_FC40, 1),
    ];
    // Zero values stay zero in memory; dropping them keeps the writes to what changes anything.
    values.retain(|v| v.value != 0);
    let copy = settings.map_or_else(Vec::new, |area| area[..USER_COPY_LEN].to_vec());
    (values, copy)
}

/// The ARM7 BIOS's CRC, which the BIOS leaves at `027FF850h` and the firmware copies to
/// `027FFC10h` (`direct-boot.md`).
// TODO(hw-unverified): GBATEK's constant for cartridge boot time, not checked on hardware.
const NDS7_BIOS_CRC: u16 = 0x5835;

/// The bytes of one binary in the image, or `None` when its range is not inside it.
fn binary_bytes<'a>(rom: &'a [u8], spec: &BinarySpec) -> Option<&'a [u8]> {
    let start = spec.rom_offset as usize;
    rom.get(start..start.saturating_add(spec.size as usize))
}

/// Places the binaries and sets the ARM9 up at its entry point. `header` has been validated by
/// `cart::parse`, so every range it names is inside `rom` and main RAM.
// TODO(hw-unverified): CPSR at entry, CP15 left at power-on, the other register banks at zero and
// the ARM9-then-ARM7 placement order (`direct-boot.md`, "Unverified").
// TODO(hw-unverified): the ARM7's CPSR at entry is chosen like the ARM9's (`direct-boot.md`).
// The BIOS and firmware values in `027FF800h`-`027FFCEFh` are `boot_values` (#39).
pub(crate) fn boot(
    arm9: &mut Cpu<true>,
    arm7: &mut Cpu<false>,
    memory: &mut Memory,
    io: &mut Io,
    scheduler: &mut Scheduler,
    rom: &[u8],
    header: &Header,
) {
    io.cart_mut().direct_boot();
    let (values, user_settings) = boot_values(rom, header, io.spi().firmware().data());
    // TODO(hw-unverified): CP15 is left at power-on, so both TCMs are disabled here
    // (`direct-boot.md`, "CP15 state at entry").
    let tcm = TcmConfig::from_cp15(arm9.cp15());
    let floor9 = scheduler.now().saturating_add(ARM9_INSTRUCTION_COST);
    let mut view = Arm9View::new(memory, io, scheduler, rom, tcm);
    view.begin_step(0, floor9);
    if let Some(copy) = rom.get(..HEADER_LEN) {
        place(&mut view, HEADER_COPY_ADDRESS, copy);
    }
    // TODO(hw-unverified): the BIOS leaves the real card's answer; the model's ID, from the same
    // function `B8` uses, stands in for it (`direct-boot.md`, "Unverified").
    let chip_id = crate::cart::protocol::chip_id(rom);
    CHIP_ID_ADDRESSES
        .iter()
        .for_each(|&address| view.write32(address, chip_id));
    // The values the BIOS and firmware leave, after the header copy and before the binaries (none
    // of which can reach `027F_F800h`: a binary ends at `023B_FE00h` at the latest).
    values.iter().for_each(|v| match v.bytes {
        2 => view.write16(v.address, v.value as u16),
        _ => view.write32(v.address, v.value),
    });
    place(&mut view, USER_SETTINGS_COPY, &user_settings);
    // TODO(hw-unverified): the ARM9 binary is placed before the ARM7's; GBATEK gives no order.
    if let Some(bytes) = binary_bytes(rom, &header.arm9) {
        place(&mut view, header.arm9.ram_address, bytes);
    }
    // All 32 KiB of shared WRAM go to the ARM7 first, so the boot area at `037F_8000h`, its stacks
    // at `0380_FD80h` and the IRQ vector at `0380_FFFCh` are different memory; with mode 0 they
    // would overlap through the ARM7's 64 KiB repetition (`wramcnt.md`, "Behavior").
    // TODO(hw-unverified): the value after boot is inferred, not stated, and leaves the ARM9
    // stacks at `0300_2F7Ch` unbacked (`wramcnt.md`, "Unverified").
    view.write8(WRAMCNT_ADDR, WRAMCNT_ARM7_ALL);
    if let Some(bytes) = binary_bytes(rom, &header.arm7) {
        place7(
            &mut Arm7View::new(memory, io, scheduler, rom, 0),
            header.arm7.ram_address,
            bytes,
        );
    }

    let entry = header.arm9.entry;
    let regs = arm9.regs_mut();
    regs.set_cpsr(CPSR_AT_ENTRY);
    regs.set(STACK_REGISTER, SP_SYSTEM);
    regs.set(IP_REGISTER, entry);
    regs.set(LINK_REGISTER, entry);
    regs.set_banked(Bank::Irq, STACK_REGISTER, SP_IRQ);
    regs.set_banked(Bank::Supervisor, STACK_REGISTER, SP_SUPERVISOR);
    let mut view = Arm9View::new(memory, io, scheduler, rom, tcm);
    view.begin_step(0, floor9);
    arm9.start_at(&mut view, entry);

    let entry7 = header.arm7.entry;
    let regs7 = arm7.regs_mut();
    regs7.set_cpsr(CPSR_AT_ENTRY);
    regs7.set(STACK_REGISTER, ARM7_SP_SYSTEM);
    regs7.set(IP_REGISTER, entry7);
    regs7.set(LINK_REGISTER, entry7);
    regs7.set_banked(Bank::Irq, STACK_REGISTER, ARM7_SP_IRQ);
    regs7.set_banked(Bank::Supervisor, STACK_REGISTER, ARM7_SP_SUPERVISOR);
    let mut view7 = Arm7View::new(memory, io, scheduler, rom, 0);
    arm7.start_at(&mut view7, entry7);
}
