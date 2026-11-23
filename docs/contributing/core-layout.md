# ninge-core layout and placement

Reference detail behind `AGENTS.md` §2.

Inside ninge-core: `machine/` (`Machine`, the one struct that owns all state) · `cpu/` (shared
ARMv4T/v5TE interpreter) · `bus/` (ARM9 view, ARM7 view, shared memory) · `io/` (the register
devices: `irq/` `ipc/` `timers/` `dma/` `timing/` `cart/` `divsqrt/` `keypad/` `power/`; the
address map `io/map.rs`; the one device interface `io/device.rs`) · `savestate/` · `scheduler/` ·
`gpu2d/` (engines A/B) · `gpu3d/` · `spu/` · `cart/` (protocol, AUXSPI saves) · `spi/`
(synthetic firmware, touch, power) · `rtc/` · `hle/` (HLE BIOS SWIs, direct boot) · `debug/` (the
read-only inspect API, `Machine::inspect`; breakpoint/trace hooks come later).

**Placement.** A hardware model apart from its register window (`cart/`, `gpu2d/`, later `spu/` `gpu3d/`
`spi/`) is top-level; the registers the bus reaches are its entrance, `io/<device>/`
(`io/cart/`). A device whose registers are its state (`divsqrt` `keypad` `power` `timers` `ipc`
`irq` `dma` `timing`) lives wholly in `io/`. A model whose registers are its state may skip the
entrance and register directly (`gpu2d`).
