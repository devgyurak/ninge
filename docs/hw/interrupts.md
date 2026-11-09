# Interrupt controller (IME, IE, IF)

Sources: GBATEK "DS Interrupts" (IME, IE and IF registers, the interrupt bit table);
GBATEK "ARM CPU Exceptions" for the IRQ entry (see `arm-exceptions-modes.md`).
Used by: `crates/core/src/irq.rs`, `crates/core/src/bus/memory.rs`, `crates/core/src/lib.rs`   Phase: 3 (#23)

Each CPU has its own interrupt controller: three registers that sit at the same addresses on
both CPUs but are separate hardware. A write by one CPU never shows on the other. This note
covers the registers, which bits exist, and when the CPU is interrupted. The sources that raise
the bits (timers, DMA, IPC, the cartridge, the LCD, ...) arrive with their own issues and notes.

## Registers

| Address | Name | Width | Access | Reset |
|---|---|---|---|---|
| `0400_0208h` | IME | 32 | R/W | not stated, 0 here |
| `0400_0210h` | IE | 32 | R/W | not stated, 0 here |
| `0400_0214h` | IF | 32 | R/W (write 1 to clear) | not stated, 0 here |

The same three addresses exist on the ARM9 and on the ARM7.

### IME bits

| Bits | Meaning |
|---|---|
| 0 | Master enable: 0 = no interrupt reaches the CPU, 1 = enabled |
| 1-31 | Not used |

### IE and IF bits

IE bit *n* enables the source whose request sets IF bit *n*. Which bits exist depends on the
CPU:

| Bit | Source | ARM9 | ARM7 |
|---|---|---|---|
| 0 | LCD V-blank | yes | yes |
| 1 | LCD H-blank | yes | yes |
| 2 | LCD V-counter match | yes | yes |
| 3-6 | Timer 0-3 overflow | yes | yes |
| 7 | Serial port / RCNT / RTC | no | yes |
| 8-11 | DMA 0-3 | yes | yes |
| 12 | Keypad | yes | yes |
| 13 | Game Pak slot (IREQ_MC) | yes | yes |
| 14-15 | Not used | no | no |
| 16 | IPC sync | yes | yes |
| 17 | IPC send FIFO empty | yes | yes |
| 18 | IPC receive FIFO not empty | yes | yes |
| 19 | Card slot transfer complete | yes | yes |
| 20 | Card slot IREQ_MC | yes | yes |
| 21 | Geometry command FIFO | yes | no |
| 22 | Screens unfolding | no | yes |
| 23 | SPI bus | no | yes |
| 24 | Wi-Fi | no | yes |
| 25-31 | Not used | no | no |

Valid-bit masks, worked out from the table: ARM9 = bits 0-6, 8-13 and 16-21 = `0x003F_3F7F`;
ARM7 = bits 0-13, 16-20 and 22-24 = `0x01DF_3FFF`. See "Unverified" for what a write to a bit
that does not exist does.

## Behavior

**IME** keeps only bit 0; the other bits read 0.

**IE** is an ordinary read/write register: a bit set enables that source.

**IF** latches a request. A source that fires sets its IF bit, whether or not the source is
enabled in IE and whether or not IME is set; IF records that the request happened. Software
acknowledges a request by writing 1 to the bit: a 1 in the written value clears that IF bit, a
0 leaves the bit as it was. Software cannot set an IF bit by writing.

**Wider and narrower accesses.** Each register is a 32-bit register. A 16-bit or 8-bit access
sees the corresponding part of it (little-endian), and a write to a part acts on only that
part: an 8-bit write of `0x01` to the low byte of IF clears bit 0 and touches no other bit.

**The IRQ line.** The controller asserts the CPU's IRQ line while

`IME = 1` and `(IE & IF) != 0`.

The CPU takes the IRQ exception between two instructions when the line is asserted and the I
flag of CPSR is clear (`arm-exceptions-modes.md`, "IRQ and FIQ"). With I set the request waits:
the line stays asserted and the exception is taken once I is cleared. The DS has no source for
the FIQ line, so it is never raised.

Taking the exception does not clear IF. The handler does it, by writing 1 to the bit; a
handler that returns without doing so is interrupted again at once.

**The IRQ dispatcher is not the controller.** On real hardware the BIOS vector code saves
registers and calls the user's handler through a pointer in memory; that routine belongs to the
HLE BIOS (#27). #23 stops at the exception entry: IRQ mode, R14_irq, SPSR_irq, and the vector.

## Edge cases

- IME = 0: IF still latches requests; the line is low and no exception is taken. Setting IME
  later takes the pending, enabled requests.
- IE = 0 for a source: IF still latches it; nothing is taken until IE enables it.
- Clearing IF while the CPU has not yet taken the exception drops the request.
- The two controllers are independent: a request for the ARM9 does not set the ARM7's IF.

## Unverified

- Reset values of IME, IE and IF are not stated — `crates/core/src/irq.rs` — all 0, which is
  what a handoff by the BIOS expects; verify by reading the registers in a test ROM on
  hardware before any write.
- A write to a bit that does not exist (the "no" cells above) — `crates/core/src/irq.rs`
  — IE drops such a bit and a request cannot set it, so reads show 0 there; GBATEK marks the
  bits "unused" without saying whether they store a value. Verify with a test ROM that writes
  `FFFF_FFFFh` to IE on hardware and reads it back.
- Bit 13 is listed for both CPUs, and bit 7 only for the ARM7 — `crates/core/src/irq.rs` —
  as in the GBATEK table; a DSi-mode difference is out of scope.
- Whether an IME or IE write reaches the IRQ line instantly or a few cycles later — the model
  is instant, and the line is read before each instruction — `crates/core/src/lib.rs` —
  verify with a cycle-timed test ROM.
- The IRQ line is sampled before each instruction, so a request raised during an instruction
  is taken after it — `crates/core/src/lib.rs` — approximate timing is the goal
  (AGENTS.md §2 rule 7).
