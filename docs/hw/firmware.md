# Firmware: the flash chip and our synthetic image

Sources: GBATEK "DS Firmware Serial Flash Memory" (the chip, the instruction set); GBATEK "DS
Firmware Header"; GBATEK "DS Firmware Wifi Calibration Data"; GBATEK "DS Firmware User Settings"
(the layout, the language and flag bits, the two areas); GBATEK "DS Touch Screen Controller (TSC)"
(the calibration formula); GBATEK "DS Serial Peripheral Interface Bus (SPI)".
Used by: `crates/core/src/spi/firmware/`, `crates/core/src/io/spi/`   Phase: 5 (#39)

The console's firmware is a flash chip on the SPI bus (`spi.md`, select 1). ninge never reads a
firmware dump (AGENTS.md §3.2): it **builds** an image from values of its own, in the layout GBATEK
describes, with every CRC computed in code. Games read the image over SPI (the user settings, the
touch calibration) and see a console that is set up.

## Scope

Implemented (#39): the chip, the image builder and its settings, the user-settings area and the
values games read. Out of scope: executing the firmware's boot code (parts 1-5), Wi-Fi behavior
(the calibration block is data only), writing the image to a host file, the DSi and iQue extended
settings, and a host command line for the settings (a follow-up).

## The chip

An ST M45PE20, 256 KiB (`40000h` bytes), on the original console; RDID answers `20h 40h 12h`. Its
instruction set (WREN, WRDI, RDID, RDSR, READ, FAST READ, PW, PP, PE, SE, DP, RDP), the status
register and the session rules are the FLASH set of the cartridge save chips and are not repeated
here: see `auxspi-saves.md`, "FLASH". The model is the same code. The 256 KiB part is the chip the
save chips call FLASH "256K".

Writes go to the image in memory (WREN, PP, PW, PE and SE work as for a save chip). The image is
part of the machine state and so of a savestate. There is no host file: the image is rebuilt from
the settings at every boot (see "Synthetic values"), so a write lasts until the next boot.

## The memory map

| Range | Content |
|---|---|
| `00000h`-`00029h` | Header |
| `0002Ah`-`001FFh` | Wi-Fi calibration data (`config_length` bytes from `02Ch`) |
| `00200h`-`3F9FFh` | Firmware code and data (the boot code; none here) |
| `3FA00h`-`3FDFFh` | Wi-Fi access points and an unused block (none here) |
| `3FE00h`-`3FEFFh` | User settings, area 0 |
| `3FF00h`-`3FFFFh` | User settings, area 1 |

The user settings are at `[020h] * 8` (`3FE00h`), the last 200h bytes. All offsets and field sizes
below are from GBATEK.

## The header (`000h`-`029h`)

| Offset | Size | Field | Our value |
|---|---|---|---|
| `000h` | 2 | part 3 address / 8 | `0000h` |
| `002h` | 2 | part 4 address / 8 | `0000h` |
| `004h` | 2 | CRC16 of part 3 and 4 | `0000h` |
| `006h` | 2 | CRC16 of part 1 and 2 | `0000h` |
| `008h` | 4 | firmware identifier | `4Eh 4Eh 47h 45h` ("NNGE"), our own |
| `00Ch`-`013h` | 8 | part 1 and part 2 address and RAM fields | zero |
| `014h` | 2 | shift amounts (bits 0-11), chip size / 128 KiB (bits 12-15) | `2000h` (size field 2) |
| `016h` | 2 | part 5 address / 8 | `0000h` |
| `018h` | 5 | build timestamp (BCD) | `FFh` |
| `01Dh` | 1 | console type | `FFh` (the original console) |
| `01Eh` | 2 | unused | `FFFFh` |
| `020h` | 2 | user settings offset / 8 | `7FC0h` (`3FE00h`) |
| `022h` | 2 | unknown | `FFFFh` |
| `024h` | 2 | unknown | `FFFFh` |
| `026h` | 2 | CRC16 of part 5 | `0000h` |
| `028h` | 2 | unused | `FFFFh` |

There is no boot code, so every address of a part is zero and so is its CRC. The unknown fields
at `022h` and `024h` have values on real consoles that GBATEK does not explain; they are never read
by anything modeled here. Console type `FFh` is the original console, which keeps the DS-Lite-only
backlight bits of the language and flags field out of play.

## The Wi-Fi calibration block (`02Ah`-`1FFh`)

| Offset | Size | Field | Our value |
|---|---|---|---|
| `02Ah` | 2 | CRC16 (initial value 0) of the `config_length` bytes from `02Ch` | computed |
| `02Ch` | 2 | `config_length` | `0138h` (entries `02Ch`-`163h`) |
| `02Eh` | 1 | unused | `00h` |
| `02Fh` | 1 | version | `00h` |
| `030h` | 6 | unused | zero |
| `036h` | 6 | MAC address | `02h 00h 00h 00h 00h 01h` (see below) |
| `03Ch` | 2 | enabled channels | `3FFEh` |
| `03Eh` | 2 | flags | `FFFFh` |
| `040h` | 1 | RF chip type | `02h` |
| `041h`-`043h` | 3 | RF bits per entry, entry count, unknown | `18h`, `0Ch`, `01h` |
| `044h`-`0CDh` | | initial register and baseband values | `00h` |
| `0CEh`-`161h` | | RF tables (type 2 layout) | `00h` |
| `162h` | 1 | unknown (per-console calibration on real units) | `19h`, inside GBATEK's "usually 19h..1Ch"; our choice |
| `163h` | 1 | unused (inside the CRC) | `FFh` |
| `164h`-`1FFh` | | unused (outside the CRC) | `FFh` |

Which values are GBATEK's and which are ours: `config_length` (`0138h`), the enabled channels
(`3FFEh`), the flags (`FFFFh`), the RF chip type (`02h`), the three RF fields (`18h`, `0Ch`, `01h`)
and the unused `FFh` bytes are GBATEK's "usual" values for the original console; the version (`00h`),
the zero tables, the MAC and `162h` are our choice.

The CRC is CRC-16/MODBUS with an initial value of **zero** here, where the user settings use
`FFFFh` (the code folds the one CRC function from zero for this block). Nothing models the radio,
so the tables are zero and mean nothing. The MAC is a **locally administered, unicast address**
(bit 1 of its first byte set, bit 0 clear): ninge never writes one of the vendor prefixes GBATEK
lists.

## The user settings (`070h` bytes plus the counter and the CRC)

Both areas have the same layout; each is `100h` bytes.

| Offset | Size | Field |
|---|---|---|
| `000h` | 2 | version (5) |
| `002h` | 1 | favorite color (0-15) |
| `003h` | 1 | birthday month (1-12, binary) |
| `004h` | 1 | birthday day (1-31, binary) |
| `005h` | 1 | unused (0) |
| `006h` | 20 | nickname, UTF-16 |
| `01Ah` | 2 | nickname length in characters (0-10) |
| `01Ch` | 52 | message, UTF-16 |
| `050h` | 2 | message length in characters (0-26) |
| `052h`-`057h` | 6 | alarm hour, alarm minute, 2 unknown bytes, alarm enable, 0 |
| `058h` | 4 | touch calibration: ADC x1, ADC y1 (two 12-bit values) |
| `05Ch` | 2 | touch calibration: screen x1, y1 (two 8-bit values) |
| `05Eh` | 4 | touch calibration: ADC x2, ADC y2 |
| `062h` | 2 | touch calibration: screen x2, y2 |
| `064h` | 2 | language and flags |
| `066h` | 1 | year (2000 + this) |
| `067h` | 1 | unknown |
| `068h` | 4 | RTC offset in seconds |
| `06Ch` | 4 | unused (`FFh`) |
| `070h` | 2 | update counter (0-7Fh) |
| `072h` | 2 | CRC16 of `000h`-`06Fh`, initial value `FFFFh` |
| `074h`-`0FFh` | | unused (`FFh`) |

Words are little endian. The first `70h` bytes are what the boot code copies to main RAM at
`027F_FC80h` (`direct-boot.md`, "Boot-time values in main RAM"); the counter and the CRC stay in the
flash.

### Language and flags (`064h`)

| Bits | Meaning |
|---|---|
| 0-2 | Language: 0 Japanese, 1 English, 2 French, 3 German, 4 Italian, 5 Spanish |
| 3 | GBA-mode screen: 0 upper, 1 lower |
| 4-5 | Backlight level (the DS-Lite only) |
| 6 | Boot menu disable: 1 = autostart the cartridge |
| 9 | Settings lost: 1 = prompt for user info, language and calibration |
| 10, 11, 13, 14, 15 | Settings okay: 0 = prompt for user info (13 also for language) |

The health-and-safety screen is skipped when bit 9 is 1, or when one of bits 10, 11, 13, 14, 15 is
0, and a game that checks these bits wants them all set with bit 9 clear.

### The two areas and the newest one

If both areas have a valid CRC, the newer is the one whose counter is one higher than the other's:
`count1 == (count0 + 1) & 7Fh` makes area 1 the newer, otherwise area 0. A console prefers the
newer area; the older one is the recovery copy.

## Synthetic values

Everything below is our own choice, made so the console looks set up and no game prompts for
setup. The builder is a pure function of a plain-data settings value (`FirmwareSettings`) with
defaults; the same settings always give the same bytes, with no clock and no randomness.

| Field | Default | Why |
|---|---|---|
| nickname | `ninge` | our own |
| message | empty | nothing to say |
| language | English (1) | a default |
| favorite color | 0 | a default |
| birthday | 1 January | a default |
| year | 2025 (`19h`) | a default |
| alarm | off | nothing is modeled |
| language and flags | `EC41h`: English, bit 6 set, bits 10, 11, 13, 14, 15 set, bit 9 clear | settings okay (above); the boot menu is off because ninge boots the cartridge directly |
| RTC offset | 0 | the RTC is #40 |
| update counters | area 0 = 0, area 1 = 1 | area 1 is the newest by the rule above |

Unused regions of the image are `FFh` (erased flash), except where a field above says zero: the
firmware code region, the access-point blocks and the unused parts of the header and settings.

### The touch calibration constant

One constant pair of calibration points, defined once in `spi/` and shared with the touch
controller of #41:

| Point | ADC (12-bit) | Screen (8-bit, origin 1,1) |
|---|---|---|
| 1 | x1 = 512, y1 = 512 | x1 = 33, y1 = 33 |
| 2 | x2 = 3568, y2 = 2544 | x2 = 224, y2 = 160 |

Both axes have **16 ADC counts per pixel**: GBATEK's formula
`scr = (adc - adc1) * (scr2 - scr1) / (adc2 - adc1) + (scr1 - 1)` gives `scr = adc / 16` exactly,
with no rounding. #41's touch controller turns a pixel into the ADC values `16 * pixel`; the pixel
then goes through ADC, the game's formula and back to the same pixel, exactly. All ADC values for
pixels 0-255 and 0-191 fit in 12 bits.

## Unverified

Each item carries `TODO(hw-unverified)` at the code site named. "Verify by" is a test ROM on
hardware, or a comparison with the reference emulator used as an oracle.

- **The header fields we choose** (identifier, the zero parts, the `FFFFh` unknowns at `022h` and
  `024h`, the build timestamp, the unused `FFh` regions): GBATEK's values are those of real
  firmware, ours are free. `spi/firmware/`. Verify by a game that reads the header.
- **The Wi-Fi block's values** beyond the CRC and `config_length`: zero tables, a version of 0, our
  MAC. `spi/firmware/`.
- **The boot menu bit** is set in the defaults. `spi/firmware/`.
- **The calibration constant** is ours; real consoles differ per unit. `spi/`.
- **RDID, status, and the write behavior of the FLASH set** on the firmware chip are the save
  chip's and carry its Unverified list (WRSR, PW with no data, extra bytes after PE and SE, the
  deep power-down lockout): `auxspi-saves.md`, "Unverified". `cart/save/`.
- **The firmware's write protection** (the `/W` pin protecting the first pages) is not modeled. `cart/save/`.
