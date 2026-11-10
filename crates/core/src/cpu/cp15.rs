// SPDX-License-Identifier: Apache-2.0
//! The ARM9's system control coprocessor (CP15), minimal: the control register, the TCM region
//! registers, the read-only ID registers, and the protection-unit, cache and process-ID
//! registers, which are held but not consulted. Note: `docs/hw/cp15.md`.

use serde::{Deserialize, Serialize};

/// What a read of an unmapped register returns.
// TODO(hw-unverified): the unmapped registers may take Undefined instead
// (`docs/hw/cp15.md`, "Unverified").
pub(super) const UNMAPPED_READ: u32 = 0;

/// Main ID register: ARM, ARMv5TE, ARM946, revision 1 (`docs/hw/cp15.md`, "C0,C0,0 / 1 / 2").
const MAIN_ID: u32 = 0x4105_9461;
/// Cache type register (`docs/hw/cp15.md`, "C0,C0,0 / 1 / 2").
const CACHE_TYPE: u32 = 0x0F0D_2112;
/// TCM physical size register: 32 KiB ITCM, 16 KiB DTCM (`docs/hw/cp15.md`, "C0,C0,0 / 1 / 2").
const TCM_SIZE: u32 = 0x0014_0180;

// TODO(hw-unverified): bits 2, 7, 12, 14 and 15 are only held; nothing consults them yet
// (`docs/hw/cp15.md`, "Unverified").
/// Control register bits that read and write; the rest are zero or fixed
/// (`docs/hw/cp15.md`, "C1,C0,0 — control register"): 0 PU enable, 2 data cache, 7 big-endian,
/// 12 instruction cache, 13 vector base, 14 replacement, 15 pre-ARMv5 mode, 16-19 TCM enable and
/// load mode.
const CONTROL_WRITABLE: u32 = 0x000F_F085;
/// Control register bits 3-6, which always read as one.
const CONTROL_FIXED: u32 = 0x0000_0078;
/// Control register bit 13: the exception vectors sit at `FFFF0000h`.
const CONTROL_HIGH_VECTORS: u32 = 1 << 13;
/// Control register bit 16: DTCM enable.
const CONTROL_DTCM_ENABLE: u32 = 1 << 16;
/// Control register bit 17: DTCM load mode.
const CONTROL_DTCM_LOAD: u32 = 1 << 17;
/// Control register bit 18: ITCM enable.
const CONTROL_ITCM_ENABLE: u32 = 1 << 18;
/// Control register bit 19: ITCM load mode.
const CONTROL_ITCM_LOAD: u32 = 1 << 19;

// TODO(hw-unverified): the reserved bits of the registers below are dropped on a write and read as
// zero, taking GBATEK's "reserved/zero" literally (`docs/hw/cp15.md`, "Unverified").
/// Per-region bits of the cachability and write-bufferability registers.
const REGION_BITS_MASK: u32 = 0xFF;
/// Two permission bits per region.
const ACCESS_MASK: u32 = 0xFFFF;
/// Protection region register: enable, size, and base; bits 6-11 are reserved
/// (`docs/hw/cp15.md`, "C2, C3, C5, C6 — protection unit").
const REGION_MASK: u32 = 0xFFFF_F03F;
/// TCM region register: size (bits 1-5) and base (bits 12-31); the rest are reserved.
const TCM_REGISTER_MASK: u32 = 0xFFFF_F03E;
/// The ITCM register keeps the size only: its base is fixed at 0.
const ITCM_REGISTER_MASK: u32 = TCM_REGISTER_MASK & !TCM_BASE_MASK;
/// Where the size field of a TCM region register starts: bit 1.
const TCM_SIZE_SHIFT: u32 = 1;
/// The width of that field: five bits (bits 1-5).
const TCM_SIZE_FIELD: u32 = 0x1F;
/// The smallest TCM size, 512 bytes, shifted left by the size field.
const TCM_SIZE_UNIT: u64 = 512;
/// The base field of a TCM region register starts at bit 12.
const TCM_BASE_MASK: u32 = 0xFFFF_F000;

/// A TCM region as the control and region registers resolve it (`docs/hw/cp15.md`, "C9,C1,0 /
/// C9,C1,1 — TCM region registers").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tcm {
    /// The address the region starts at; always 0 for the ITCM.
    pub base: u32,
    /// The virtual size in bytes, `512 << N`; 4 GiB does not fit in 32 bits.
    pub size: u64,
    /// The control register's enable bit for this TCM.
    pub enabled: bool,
    /// The control register's load-mode bit: loads read the underlying memory, stores write the
    /// TCM.
    pub load_mode: bool,
}

/// The CP15 state (`docs/hw/cp15.md`).
// The debugger's CPU panel (#56) will show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cp15 {
    /// C1,C0,0, with only the writable bits kept.
    // TODO(hw-unverified): every register starts at zero; GBATEK gives no reset values
    // (`docs/hw/cp15.md`, "Unverified").
    control: u32,
    /// C2,C0,0 and C2,C0,1: cachability bits, data then instruction.
    cachable: [u32; 2],
    /// C3,C0,0: write-bufferability bits.
    bufferable: u32,
    /// C5,C0,0 to C5,C0,3: access permission, data, instruction, extended data, extended
    /// instruction.
    // TODO(hw-unverified): the two-bit and four-bit views are separate storage here
    // (`docs/hw/cp15.md`, "Unverified").
    access: [u32; 4],
    /// C6,C0..C7: the protection regions, shared by the data and instruction views.
    regions: [u32; 8],
    /// C9,C0,0 and C9,C0,1: cache lockdown, data then instruction.
    // TODO(hw-unverified): the full 32-bit value is held (`docs/hw/cp15.md`, "Unverified").
    lockdown: [u32; 2],
    /// C9,C1,0: the DTCM region register.
    dtcm: u32,
    /// C9,C1,1: the ITCM region register.
    itcm: u32,
    /// C13,C0,1 and C13,C1,1: the trace process ID.
    trace_id: u32,
}

impl Cp15 {
    /// CP15 in its power-on state.
    pub fn new() -> Self {
        Self {
            control: 0,
            cachable: [0; 2],
            bufferable: 0,
            access: [0; 4],
            regions: [0; 8],
            lockdown: [0; 2],
            dtcm: 0,
            itcm: 0,
            trace_id: 0,
        }
    }

    /// The control register as an MRC reads it: the writable bits plus the fixed ones.
    pub fn control(&self) -> u32 {
        self.control | CONTROL_FIXED
    }

    /// Whether the exception vectors sit at `FFFF0000h` (control register bit 13).
    pub fn high_vectors(&self) -> bool {
        self.control & CONTROL_HIGH_VECTORS != 0
    }

    /// The data TCM as the registers resolve it.
    // The ARM9 bus view copies it before each step (`docs/hw/arm9-memory-map.md`).
    // TODO(hw-unverified): a size field outside 3-23 and a base not aligned to the size are
    // reported as written (`docs/hw/cp15.md`, "Unverified").
    pub fn dtcm(&self) -> Tcm {
        self.tcm(self.dtcm, CONTROL_DTCM_ENABLE, CONTROL_DTCM_LOAD)
    }

    /// The instruction TCM as the registers resolve it; its base is always 0.
    // The ARM9 bus view copies it before each step (`docs/hw/arm9-memory-map.md`).
    // TODO(hw-unverified): see `Cp15::dtcm`; the base is dropped on a write from GBATEK's
    // "always 00000000h" (`docs/hw/cp15.md`, "Unverified").
    pub fn itcm(&self) -> Tcm {
        self.tcm(self.itcm, CONTROL_ITCM_ENABLE, CONTROL_ITCM_LOAD)
    }

    fn tcm(&self, register: u32, enable: u32, load: u32) -> Tcm {
        let size = (register >> TCM_SIZE_SHIFT) & TCM_SIZE_FIELD;
        Tcm {
            base: register & TCM_BASE_MASK,
            size: TCM_SIZE_UNIT << size,
            enabled: self.control & enable != 0,
            load_mode: self.control & load != 0,
        }
    }

    /// MRC: the value of the register CRn, CRm and opc2 select (opc1 = 0).
    // TODO(hw-unverified): a read of the write-only C7 and of an unmapped register returns 0
    // (`docs/hw/cp15.md`, "Unverified").
    // TODO(phase8): the BIST registers at C15 read 0 (`docs/hw/cp15.md`, "Not modeled").
    pub(super) fn read(&self, crn: u32, crm: u32, opc2: u32) -> u32 {
        match (crn, crm, opc2) {
            (0, 0, 1) => CACHE_TYPE,
            (0, 0, 2) => TCM_SIZE,
            // C0,C0,3..7 read as the main ID too (`docs/hw/cp15.md`, "C0,C0,0 / 1 / 2").
            (0, 0, _) => MAIN_ID,
            (1, 0, 0) => self.control(),
            (2, 0, 0 | 1) => self.cachable[opc2 as usize],
            (3, 0, 0) => self.bufferable,
            (5, 0, 0..=3) => self.access[opc2 as usize],
            (6, 0..=7, 0 | 1) => self.regions[crm as usize],
            (9, 0, 0 | 1) => self.lockdown[opc2 as usize],
            (9, 1, 0) => self.dtcm,
            (9, 1, 1) => self.itcm,
            (13, 0, 1) | (13, 1, 1) => self.trace_id,
            // C13,C0,0 (the FCSE process ID) reads zero; C7 is write-only; the rest is unmapped.
            _ => UNMAPPED_READ,
        }
    }

    /// MCR: writes the register CRn, CRm and opc2 select (opc1 = 0).
    // TODO(hw-unverified): a write to an ID register, the FCSE process ID or an unmapped register
    // is ignored (`docs/hw/cp15.md`, "Unverified").
    // TODO(phase8): the BIST registers at C15 ignore writes (`docs/hw/cp15.md`, "Not modeled").
    // C7,C0,4 and C7,C8,2 are wait-for-interrupt: `Cpu::cp15_transfer` halts the core, this
    // register file only accepts the write.
    // TODO(phase8): the protection unit registers are held, but no access is checked against
    // them and no cache is modeled.
    pub(super) fn write(&mut self, crn: u32, crm: u32, opc2: u32, value: u32) {
        match (crn, crm, opc2) {
            (1, 0, 0) => self.control = value & CONTROL_WRITABLE,
            (2, 0, 0 | 1) => self.cachable[opc2 as usize] = value & REGION_BITS_MASK,
            (3, 0, 0) => self.bufferable = value & REGION_BITS_MASK,
            (5, 0, 0 | 1) => self.access[opc2 as usize] = value & ACCESS_MASK,
            (5, 0, 2 | 3) => self.access[opc2 as usize] = value,
            (6, 0..=7, 0 | 1) => self.regions[crm as usize] = value & REGION_MASK,
            (9, 0, 0 | 1) => self.lockdown[opc2 as usize] = value,
            (9, 1, 0) => self.dtcm = value & TCM_REGISTER_MASK,
            (9, 1, 1) => self.itcm = value & ITCM_REGISTER_MASK,
            (13, 0, 1) | (13, 1, 1) => self.trace_id = value,
            // The ID registers and the FCSE process ID are read-only, the cache commands of C7
            // have no effect, and the unmapped registers ignore the write.
            _ => {}
        }
    }
}

impl Default for Cp15 {
    fn default() -> Self {
        Self::new()
    }
}
