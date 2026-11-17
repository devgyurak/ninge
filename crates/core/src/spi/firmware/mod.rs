// SPDX-License-Identifier: Apache-2.0
//! The synthetic firmware image (`docs/hw/firmware.md`): a 256 KiB flash image built from our own
//! settings in the layout GBATEK describes, with every CRC computed here. No dump is read and
//! nothing is random: the same settings always give the same bytes.

use serde::{Deserialize, Serialize};

use super::CALIBRATION;
use crate::cart::{crc16, crc16_update};

/// The size of the firmware flash, an ST M45PE20 (`docs/hw/firmware.md`, "The chip").
pub const FIRMWARE_LEN: usize = 0x4_0000;

/// Where the first user-settings area sits; the second follows it (`docs/hw/firmware.md`, "The
/// memory map").
pub(crate) const USER_SETTINGS_OFFSET: usize = 0x3_FE00;
/// The size of one user-settings area.
pub(crate) const USER_AREA_LEN: usize = 0x100;
/// The part of an area the boot code copies to main RAM; the counter and the CRC stay in flash.
pub(crate) const USER_COPY_LEN: usize = 0x70;

/// Header offset of the user-settings address divided by 8.
pub(crate) const HEADER_USER_SETTINGS: usize = 0x20;
/// Header offset of the CRC16 of firmware part 5.
pub(crate) const HEADER_PART5_CRC: usize = 0x26;

const HEADER_IDENTIFIER: usize = 0x08;
const HEADER_SHIFTS: usize = 0x14;
const HEADER_CONSOLE_TYPE: usize = 0x1D;
const HEADER_UNKNOWN_A: usize = 0x22;
const HEADER_UNKNOWN_B: usize = 0x24;
const HEADER_UNUSED: usize = 0x28;

/// Our own four-byte identifier ("NNGE"), not the vendor's.
const IDENTIFIER: [u8; 4] = *b"NNGE";
/// Bits 12-15 of the shift word: the chip size in 128 KiB units (2 for 256 KiB).
const SHIFTS: u16 = 2 << 12;
/// The original console's type byte.
const CONSOLE_TYPE_ORIGINAL: u8 = 0xFF;

const WIFI_CRC: usize = 0x2A;
const WIFI_CONFIG_LENGTH: usize = 0x2C;
const WIFI_BODY_END: usize = 0x163;
const WIFI_MAC: usize = 0x36;
const WIFI_CHANNELS: usize = 0x3C;
const WIFI_FLAGS: usize = 0x3E;
const WIFI_RF_TYPE: usize = 0x40;
const WIFI_RF_BITS: usize = 0x41;
const WIFI_RF_ENTRIES: usize = 0x42;
const WIFI_UNKNOWN: usize = 0x43;
const WIFI_LAST: usize = 0x162;
/// GBATEK's usual `config_length`: entries `02Ch`-`163h`.
const CONFIG_LENGTH: u16 = 0x138;
/// A locally administered, unicast address (bit 1 of the first byte set, bit 0 clear), never one
/// of the vendor prefixes.
const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

const AREA_VERSION: usize = 0x00;
const AREA_COLOR: usize = 0x02;
const AREA_MONTH: usize = 0x03;
const AREA_DAY: usize = 0x04;
const AREA_NICKNAME: usize = 0x06;
const AREA_NICKNAME_LEN: usize = 0x1A;
const AREA_MESSAGE: usize = 0x1C;
const AREA_MESSAGE_LEN: usize = 0x50;
const AREA_CALIBRATION: usize = 0x58;
const AREA_FLAGS: usize = 0x64;
const AREA_YEAR: usize = 0x66;
const AREA_RTC_OFFSET: usize = 0x68;
const AREA_UNUSED: usize = 0x6C;
const AREA_COUNTER: usize = 0x70;
const AREA_CRC: usize = 0x72;
/// The user-settings version every firmware writes.
const AREA_VERSION_VALUE: u16 = 5;
/// The characters a nickname and a message hold at most.
const NICKNAME_CHARS: usize = 10;
const MESSAGE_CHARS: usize = 26;

/// Language and flags bits (`docs/hw/firmware.md`, "Language and flags").
const FLAG_BOOT_MENU_DISABLE: u16 = 1 << 6;
/// "Settings okay" bits 10, 11, 13, 14, 15, with bit 9 ("settings lost") clear.
const FLAGS_SETTINGS_OKAY: u16 = 0xEC00;

/// The language setting, bits 0-2 of the language and flags word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// Japanese.
    Japanese = 0,
    /// English.
    #[default]
    English = 1,
    /// French.
    French = 2,
    /// German.
    German = 3,
    /// Italian.
    Italian = 4,
    /// Spanish.
    Spanish = 5,
}

/// The values a game reads from the user settings. Plain data: the host may hand its own to the
/// builder later; out-of-range values are masked or cut to what the layout holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareSettings {
    /// The nickname; at most 10 UTF-16 units are stored.
    pub nickname: String,
    /// The message; at most 26 UTF-16 units are stored.
    pub message: String,
    /// The language.
    pub language: Language,
    /// The favorite color, 0-15 (the low four bits are stored).
    pub favorite_color: u8,
    /// The birthday month, 1-12.
    pub birthday_month: u8,
    /// The birthday day, 1-31.
    pub birthday_day: u8,
    /// The year, 2000-2255 (stored as the offset from 2000, saturating).
    pub year: u16,
    /// Bit 6 of the language and flags word: autostart the cartridge instead of the boot menu.
    pub boot_menu_disabled: bool,
}

impl Default for FirmwareSettings {
    /// Our own defaults (`docs/hw/firmware.md`, "Synthetic values").
    fn default() -> Self {
        Self {
            nickname: "ninge".to_string(),
            message: String::new(),
            language: Language::English,
            favorite_color: 0,
            birthday_month: 1,
            birthday_day: 1,
            year: 2025,
            // TODO(hw-unverified): the boot menu bit is our choice, because ninge boots the cartridge
            // directly (`firmware.md`, "Unverified").
            boot_menu_disabled: true,
        }
    }
}

/// Builds the firmware image from `settings`.
// TODO(hw-unverified): the header fields, the Wi-Fi block and the unused regions are ours, not a
// real firmware's (`docs/hw/firmware.md`, "Unverified").
pub fn build(settings: &FirmwareSettings) -> Vec<u8> {
    // Erased flash reads FFh; every field below that GBATEK or our table calls zero is written.
    let mut image = vec![0xFF; FIRMWARE_LEN];
    write_header(&mut image);
    write_wifi(&mut image);
    // Area 1 carries the higher counter, so by GBATEK's rule it is the newest.
    for (index, counter) in [0u16, 1].into_iter().enumerate() {
        let at = USER_SETTINGS_OFFSET + index * USER_AREA_LEN;
        image[at..at + USER_AREA_LEN].copy_from_slice(&user_area(settings, counter));
    }
    image
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_header(image: &mut [u8]) {
    // The part addresses and their CRCs, and the part 1/2 fields: there is no boot code.
    image[..HEADER_IDENTIFIER].fill(0);
    image[HEADER_IDENTIFIER..HEADER_IDENTIFIER + 4].copy_from_slice(&IDENTIFIER);
    image[HEADER_IDENTIFIER + 4..HEADER_SHIFTS].fill(0);
    put16(image, HEADER_SHIFTS, SHIFTS);
    image[0x16..0x18].fill(0);
    image[HEADER_CONSOLE_TYPE] = CONSOLE_TYPE_ORIGINAL;
    // The user settings are the last 200h bytes of the chip (the offset is stored divided by 8).
    put16(
        image,
        HEADER_USER_SETTINGS,
        (USER_SETTINGS_OFFSET / 8) as u16,
    );
    put16(image, HEADER_UNKNOWN_A, 0xFFFF);
    put16(image, HEADER_UNKNOWN_B, 0xFFFF);
    put16(image, HEADER_PART5_CRC, 0);
    put16(image, HEADER_UNUSED, 0xFFFF);
}

fn write_wifi(image: &mut [u8]) {
    // The body is zero (nothing models the radio); the fields GBATEK gives a usual value get it.
    image[WIFI_CONFIG_LENGTH..WIFI_LAST].fill(0);
    put16(image, WIFI_CONFIG_LENGTH, CONFIG_LENGTH);
    image[WIFI_MAC..WIFI_MAC + 6].copy_from_slice(&MAC);
    put16(image, WIFI_CHANNELS, 0x3FFE);
    put16(image, WIFI_FLAGS, 0xFFFF);
    image[WIFI_RF_TYPE] = 0x02;
    image[WIFI_RF_BITS] = 0x18;
    image[WIFI_RF_ENTRIES] = 0x0C;
    image[WIFI_UNKNOWN] = 0x01;
    image[WIFI_LAST] = 0x19;
    // The unused byte at 163h stays FFh and lies inside the CRC.
    let body = &image[WIFI_CONFIG_LENGTH..=WIFI_BODY_END];
    // The Wi-Fi block's CRC starts from zero where the user settings' starts from FFFFh.
    let crc = body.iter().fold(0, |crc, &byte| crc16_update(crc, byte));
    put16(image, WIFI_CRC, crc);
}

/// UTF-16 units of `text`, cut at `chars` units without splitting a surrogate pair.
fn utf16_units(text: &str, chars: usize) -> Vec<u16> {
    let mut units = Vec::new();
    for c in text.chars() {
        let mut buffer = [0u16; 2];
        let encoded = c.encode_utf16(&mut buffer);
        if units.len() + encoded.len() > chars {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units
}

/// Writes `units` into the UTF-16 field at `at` (zero padded) and the length at `len_at`.
fn put_text(area: &mut [u8], at: usize, len_at: usize, chars: usize, text: &str) {
    let units = utf16_units(text, chars);
    area[at..at + chars * 2].fill(0);
    for (i, unit) in units.iter().enumerate() {
        put16(area, at + i * 2, *unit);
    }
    put16(area, len_at, units.len() as u16);
}

fn user_area(settings: &FirmwareSettings, counter: u16) -> [u8; USER_AREA_LEN] {
    let mut area = [0xFF; USER_AREA_LEN];
    area[..USER_COPY_LEN].fill(0);
    put16(&mut area, AREA_VERSION, AREA_VERSION_VALUE);
    area[AREA_COLOR] = settings.favorite_color & 0x0F;
    area[AREA_MONTH] = settings.birthday_month;
    area[AREA_DAY] = settings.birthday_day;
    put_text(
        &mut area,
        AREA_NICKNAME,
        AREA_NICKNAME_LEN,
        NICKNAME_CHARS,
        &settings.nickname,
    );
    put_text(
        &mut area,
        AREA_MESSAGE,
        AREA_MESSAGE_LEN,
        MESSAGE_CHARS,
        &settings.message,
    );
    // The alarm and its enable byte stay zero: nothing models an alarm.
    let [(adc_x1, adc_y1), (adc_x2, adc_y2)] = CALIBRATION.adc;
    let [(scr_x1, scr_y1), (scr_x2, scr_y2)] = CALIBRATION.screen;
    put16(&mut area, AREA_CALIBRATION, adc_x1);
    put16(&mut area, AREA_CALIBRATION + 2, adc_y1);
    area[AREA_CALIBRATION + 4] = scr_x1;
    area[AREA_CALIBRATION + 5] = scr_y1;
    put16(&mut area, AREA_CALIBRATION + 6, adc_x2);
    put16(&mut area, AREA_CALIBRATION + 8, adc_y2);
    area[AREA_CALIBRATION + 10] = scr_x2;
    area[AREA_CALIBRATION + 11] = scr_y2;
    let boot = if settings.boot_menu_disabled {
        FLAG_BOOT_MENU_DISABLE
    } else {
        0
    };
    put16(
        &mut area,
        AREA_FLAGS,
        FLAGS_SETTINGS_OKAY | boot | settings.language as u16,
    );
    area[AREA_YEAR] = u8::try_from(settings.year.saturating_sub(2000)).unwrap_or(u8::MAX);
    area[AREA_RTC_OFFSET..AREA_RTC_OFFSET + 4].fill(0);
    area[AREA_UNUSED..AREA_UNUSED + 4].fill(0xFF);
    put16(&mut area, AREA_COUNTER, counter);
    let crc = crc16(&area[..USER_COPY_LEN]);
    put16(&mut area, AREA_CRC, crc);
    area
}

fn area_of(image: &[u8], index: usize) -> Option<&[u8]> {
    let at = USER_SETTINGS_OFFSET + index * USER_AREA_LEN;
    image.get(at..at + USER_AREA_LEN)
}

fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Whether the stored CRC of a user-settings area matches the one computed over its first `70h`
/// bytes.
fn area_valid(area: &[u8]) -> bool {
    crc16(&area[..USER_COPY_LEN]) == word(area, AREA_CRC)
}

/// The newest valid user-settings area of `image`, by GBATEK's rule: with both areas valid, area
/// 1 is the newer exactly when its counter is `(counter0 + 1) & 7Fh`, otherwise area 0. A single
/// valid area is the newest; none gives `None`.
pub fn newest_user_settings(image: &[u8]) -> Option<&[u8]> {
    let area0 = area_of(image, 0).filter(|area| area_valid(area));
    let area1 = area_of(image, 1).filter(|area| area_valid(area));
    match (area0, area1) {
        (Some(a0), Some(a1)) => {
            let newer = word(a1, AREA_COUNTER) == word(a0, AREA_COUNTER).wrapping_add(1) & 0x7F;
            Some(if newer { a1 } else { a0 })
        }
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests;
