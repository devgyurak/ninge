// SPDX-License-Identifier: Apache-2.0
//! Where each VRAM bank goes: the interpretation of a VRAMCNT register (`mapping`), the four BG
//! and OBJ windows of the two 2D engines, and the renderer's view of a window ([`VramWindow`])
//! (`docs/hw/vram-banks.md`, "Where each bank goes" and "The 2D engines' view of a window").

use super::{BANK_BASE, BANK_COUNT, Memory, PAGE_MASK, Region, VRAM_BLOCK_BASE, page_of};

/// The banks are named by their index, A = 0 to I = 8.
const A: usize = 0;
const B: usize = 1;
const C: usize = 2;
const D: usize = 3;
const E: usize = 4;
const F: usize = 5;
const G: usize = 6;
const H: usize = 7;
const I: usize = 8;

/// What a bank is used for, decoded from its VRAMCNT register (`vram-banks.md`, "Where each bank
/// goes"). Offsets count in bytes from the start of the window; slots are the display
/// controller's units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VramMapping {
    /// The enable bit is 0.
    Disabled,
    /// Enabled, but the MST names nothing the tables define for this bank: mapped nowhere.
    Unmapped,
    /// Plain CPU access at `0680_0000h` + the bank's offset in the buffer (MST 0).
    Lcdc,
    /// Engine A BG VRAM, `offset` bytes into the window at `0600_0000h`.
    BgA {
        /// Bytes from the start of the window.
        offset: u32,
    },
    /// Engine A OBJ VRAM, `offset` bytes into the window at `0640_0000h`.
    ObjA {
        /// Bytes from the start of the window.
        offset: u32,
    },
    /// Engine B BG VRAM, `offset` bytes into the window at `0620_0000h`.
    BgB {
        /// Bytes from the start of the window.
        offset: u32,
    },
    /// Engine B OBJ VRAM, `offset` bytes into the window at `0660_0000h`.
    ObjB {
        /// Bytes from the start of the window.
        offset: u32,
    },
    /// The ARM7's work RAM (banks C and D, MST 2): `offset` bytes into the ARM7's window at
    /// `0600_0000h`. The ARM9 does not see the bank.
    Arm7 {
        /// Bytes from the start of the window.
        offset: u32,
    },
    /// A texture image slot (not in any CPU address space).
    Texture {
        /// The slot, 0-3.
        slot: u8,
    },
    /// Texture palette slots `first` to `first + count - 1`.
    TexPalette {
        /// The first slot.
        first: u8,
        /// How many slots the bank covers.
        count: u8,
    },
    /// Engine A BG extended palette slots `first` to `first + count - 1`.
    BgExtPaletteA {
        /// The first slot.
        first: u8,
        /// How many slots the bank covers.
        count: u8,
    },
    /// Engine A OBJ extended palette (slot 0).
    ObjExtPaletteA,
    /// Engine B BG extended palette, slots 0-3.
    BgExtPaletteB,
    /// Engine B OBJ extended palette (slot 0).
    ObjExtPaletteB,
}

/// One of the four BG and OBJ windows of the 2D engines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VramWindowKind {
    /// Engine A BG: 512 KiB at `0600_0000h`.
    BgA,
    /// Engine A OBJ: 256 KiB at `0640_0000h`.
    ObjA,
    /// Engine B BG: 128 KiB at `0620_0000h`.
    BgB,
    /// Engine B OBJ: 128 KiB at `0660_0000h`.
    ObjB,
}

impl VramWindowKind {
    /// The CPU address the window starts at.
    pub(crate) const fn base(self) -> u32 {
        match self {
            Self::BgA => VRAM_BLOCK_BASE,
            Self::BgB => 0x0620_0000,
            Self::ObjA => 0x0640_0000,
            Self::ObjB => 0x0660_0000,
        }
    }

    /// How many bytes of the window banks can fill.
    pub(crate) const fn size(self) -> u32 {
        match self {
            Self::BgA => 512 << 10,
            Self::ObjA => 256 << 10,
            Self::BgB | Self::ObjB => 128 << 10,
        }
    }
}

/// Where the ARM7 sees banks C and D: `0600_0000h` (`vram-banks.md`, "Where each bank goes").
const ARM7_BASE: u32 = VRAM_BLOCK_BASE;
/// Bytes in one 128 KiB bank slot of a BG window.
const SLOT_128K: u32 = 0x2_0000;
/// Bytes in one 16 KiB step of banks F and G.
const STEP_16K: u32 = 0x4000;
/// Bytes in one 64 KiB step of banks F and G (OFS.1).
const STEP_64K: u32 = 0x1_0000;
/// Extended-palette slots a 16 KiB bank covers (F, G in BG mode: OFS.0 picks the pair).
const SLOT_PAIR: u8 = 2;
/// Engine B BG: where bank I sits next to bank H's 32 KiB.
const BG_B_BANK_I: u32 = 0x8000;

/// What bank `bank` (A = 0 to I = 8) is used for, given its decoded register: enable bit, MST and
/// OFS (`vram-banks.md`, "Where each bank goes").
// TODO(hw-unverified): an MST the tables do not define maps nowhere, and only OFS.0 counts where
// OFS.1 must be zero or is not used (`vram-banks.md`, "Unverified").
pub(crate) fn mapping(bank: usize, enabled: bool, mst: u8, ofs: u8) -> VramMapping {
    if !enabled {
        return VramMapping::Disabled;
    }
    let ofs = u32::from(ofs);
    let (ofs0, ofs1) = (ofs & 1, (ofs >> 1) & 1);
    // Banks F and G step through a 64 KiB window in 16 KiB pieces.
    let fg_offset = STEP_16K * ofs0 + STEP_64K * ofs1;
    match (bank, mst) {
        (_, 0) => VramMapping::Lcdc,
        (A..=D, 1) => VramMapping::BgA {
            offset: SLOT_128K * ofs,
        },
        (E, 1) => VramMapping::BgA { offset: 0 },
        (F | G, 1) => VramMapping::BgA { offset: fg_offset },
        (C | D, 2) => VramMapping::Arm7 {
            offset: SLOT_128K * ofs0,
        },
        (A | B, 2) => VramMapping::ObjA {
            offset: SLOT_128K * ofs0,
        },
        (E, 2) => VramMapping::ObjA { offset: 0 },
        (F | G, 2) => VramMapping::ObjA { offset: fg_offset },
        (A..=D, 3) => VramMapping::Texture { slot: ofs as u8 },
        (E, 3) => VramMapping::TexPalette { first: 0, count: 4 },
        (F | G, 3) => VramMapping::TexPalette {
            first: (ofs0 + 4 * ofs1) as u8,
            count: 1,
        },
        (E, 4) => VramMapping::BgExtPaletteA { first: 0, count: 4 },
        (F | G, 4) => VramMapping::BgExtPaletteA {
            first: SLOT_PAIR * ofs0 as u8,
            count: SLOT_PAIR,
        },
        (F | G, 5) => VramMapping::ObjExtPaletteA,
        (C, 4) | (H, 1) => VramMapping::BgB { offset: 0 },
        (I, 1) => VramMapping::BgB {
            offset: BG_B_BANK_I,
        },
        (D, 4) | (I, 2) => VramMapping::ObjB { offset: 0 },
        (H, 2) => VramMapping::BgExtPaletteB,
        (I, 3) => VramMapping::ObjExtPaletteB,
        _ => VramMapping::Unmapped,
    }
}

impl VramMapping {
    /// The address the ARM7 sees the bank at, or `None` when the bank is not in the ARM7 mapping.
    pub(crate) fn arm7_base(self) -> Option<u32> {
        match self {
            Self::Arm7 { offset } => Some(ARM7_BASE + offset),
            _ => None,
        }
    }

    /// The ARM9 address the bank starts at while this mapping is in force, or `None` when the
    /// ARM9 does not see the bank (`lcdc` is the bank's LCDC address).
    pub(crate) fn cpu_base(self, lcdc: u32) -> Option<u32> {
        let at = |kind: VramWindowKind, offset: u32| Some(kind.base() + offset);
        match self {
            Self::Lcdc => Some(lcdc),
            Self::BgA { offset } => at(VramWindowKind::BgA, offset),
            Self::ObjA { offset } => at(VramWindowKind::ObjA, offset),
            Self::BgB { offset } => at(VramWindowKind::BgB, offset),
            Self::ObjB { offset } => at(VramWindowKind::ObjB, offset),
            Self::Disabled
            | Self::Unmapped
            | Self::Arm7 { .. }
            | Self::Texture { .. }
            | Self::TexPalette { .. }
            | Self::BgExtPaletteA { .. }
            | Self::ObjExtPaletteA
            | Self::BgExtPaletteB
            | Self::ObjExtPaletteB => None,
        }
    }
}

/// One BG or OBJ window as a renderer reads it: bytes by offset from the window's start. The
/// offset is looked up in the ARM9's page table at the window's address plus the offset, so the
/// renderer sees exactly what the CPU would read there; no second table exists. An offset no
/// bank holds, or past the window's size, reads 0.
#[derive(Clone, Copy, Debug)]
pub struct VramWindow<'a> {
    memory: &'a Memory,
    kind: VramWindowKind,
}

impl<'a> VramWindow<'a> {
    pub(super) fn new(memory: &'a Memory, kind: VramWindowKind) -> Self {
        Self { memory, kind }
    }

    /// The byte at `offset` in the window, or 0 where no bank holds it.
    // TODO(hw-unverified): an address no bank holds reads 0 (`vram-banks.md`, "Unverified").
    pub(crate) fn byte(&self, offset: u32) -> u8 {
        if offset >= self.kind.size() {
            return 0;
        }
        let addr = self.kind.base() + offset;
        let page = page_of(&self.memory.pages9, addr);
        match page.region {
            Region::Vram => self
                .memory
                .vram
                .get(page.offset as usize + (addr & PAGE_MASK) as usize)
                .copied()
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// The little-endian halfword at `offset`.
    pub(crate) fn half(&self, offset: u32) -> u16 {
        u16::from_le_bytes([self.byte(offset), self.byte(offset.wrapping_add(1))])
    }
}

/// Bytes of one extended-palette slot: 16 palettes of 256 colours of 2 bytes
/// (`docs/hw/extended-palettes.md`, "Where the palette is").
const EXT_SLOT_BYTES: u32 = 8 << 10;
/// Bytes of one palette of 256 colours.
const EXT_PALETTE_BYTES: u32 = 512;
/// Palette numbers a slot holds, and colours in a palette.
const EXT_PALETTES: u32 = 16;
const EXT_COLORS: u32 = 256;

/// Where one extended-palette slot's 8 KiB is: the bank that backs it (A = 0 to I = 8) and the
/// byte offset of the slot in the VRAM buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExtSlot {
    pub(crate) bank: usize,
    pub(crate) start: u32,
}

/// The slots of both engines: four BG slots and one OBJ slot each, `None` where no bank backs
/// them. Plain data built from the VRAMCNT registers each line, so it is never saved
/// (`docs/hw/extended-palettes.md`, "Which bank backs a slot").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtSlots {
    pub(crate) bg_a: [Option<ExtSlot>; 4],
    pub(crate) obj_a: Option<ExtSlot>,
    pub(crate) bg_b: [Option<ExtSlot>; 4],
    pub(crate) obj_b: Option<ExtSlot>,
}

/// The slots the nine banks back, given what each is mapped as, A first. A slot a bank does not
/// back in its mapping stays empty; when two banks back the same slot the earlier letter wins, as
/// for the CPU windows.
// TODO(hw-unverified): the earlier letter wins on the display side too
// (`extended-palettes.md`, "Unverified").
pub(crate) fn ext_slots(mappings: &[VramMapping; BANK_COUNT]) -> ExtSlots {
    let mut slots = ExtSlots::default();
    for (bank, mapping) in mappings.iter().enumerate() {
        let base = BANK_BASE[bank] as u32;
        // The slot that starts `index` slots into the bank.
        let at = |index: u32| ExtSlot {
            bank,
            start: base + index * EXT_SLOT_BYTES,
        };
        // A slot already taken by an earlier bank is kept.
        let claim = |slot: Option<&mut Option<ExtSlot>>, value: ExtSlot| {
            if let Some(slot) = slot.filter(|slot| slot.is_none()) {
                *slot = Some(value);
            }
        };
        match *mapping {
            VramMapping::BgExtPaletteA { first, count } => (0..u32::from(count)).for_each(|n| {
                claim(slots.bg_a.get_mut(usize::from(first) + n as usize), at(n));
            }),
            VramMapping::ObjExtPaletteA => claim(Some(&mut slots.obj_a), at(0)),
            VramMapping::BgExtPaletteB => (0..4u32).for_each(|n| {
                claim(slots.bg_b.get_mut(n as usize), at(n));
            }),
            VramMapping::ObjExtPaletteB => claim(Some(&mut slots.obj_b), at(0)),
            _ => {}
        }
    }
    slots
}

/// The extended palettes of one engine and one kind (BG or OBJ) as a renderer reads them: a colour
/// by (slot, palette number, index). A slot no bank backs reads 0.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExtPalette<'a> {
    vram: &'a [u8],
    slots: [Option<ExtSlot>; 4],
}

impl<'a> ExtPalette<'a> {
    pub(super) fn new(vram: &'a [u8], slots: [Option<ExtSlot>; 4]) -> Self {
        Self { vram, slots }
    }

    /// The 15-bit colour of palette `palette` (0-15), entry `index` (0-255) of slot `slot`.
    // TODO(hw-unverified): a slot no bank backs reads colour 0 (`extended-palettes.md`,
    // "Unverified").
    pub(crate) fn color(&self, slot: usize, palette: u32, index: u32) -> u16 {
        let Some(Some(slot)) = self.slots.get(slot) else {
            return 0;
        };
        let at =
            slot.start + (palette % EXT_PALETTES) * EXT_PALETTE_BYTES + (index % EXT_COLORS) * 2;
        self.vram
            .get(at as usize..at as usize + 2)
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(0, u16::from_le_bytes)
    }
}

/// Which bank (A = 0 to I = 8) backs each extended-palette slot of one engine, `None` where no bank
/// does. A debugger's VRAM panel (#58) shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtPaletteBanks {
    /// The bank behind BG slots 0-3.
    pub bg: [Option<usize>; 4],
    /// The bank behind the OBJ slot.
    pub obj: Option<usize>,
}

impl ExtSlots {
    /// The banks behind the slots of engine A, or of engine B when `engine_b`.
    pub(crate) fn banks(&self, engine_b: bool) -> ExtPaletteBanks {
        let (bg, obj) = match engine_b {
            true => (&self.bg_b, &self.obj_b),
            false => (&self.bg_a, &self.obj_a),
        };
        ExtPaletteBanks {
            bg: bg.map(|slot| slot.map(|slot| slot.bank)),
            obj: obj.map(|slot| slot.bank),
        }
    }
}
