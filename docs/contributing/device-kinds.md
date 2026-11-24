# Device kinds and the rayon exception

Longer form of `AGENTS.md` §4 (per-CPU devices) and the `rayon` rationale of §2 rule 1.

## Per-CPU devices

A device is one of three kinds, by who sees it:

- *An independent copy for each CPU*: two instances of `Device<const ARM9: bool>`.
  `InterruptController` and `Timers`; DMA too.
- *Seen by both CPUs*: one instance, with `fn x<const ARM9: bool>` accessors, even when
  per-CPU registers sit inside it. `Ipc`, and `Timing` (DISPSTAT is per CPU, VCOUNT shared).
- *Seen by one CPU*: a plain type, no const generic. `Gpu2d` (both 2D engines, ARM9 only); SPI,
  RTC and SPU (ARM7 only) will be too.

Every device returns the IF bits it would raise as an `irq::Requests` (a device for one CPU
builds it with `Requests::for_cpu::<ARM9>`); only `Io::request_irq` hands them to a controller.

## Why `rayon` is on the forbidden list, and still planned

`rayon` is listed although plan §3.2 names it for the Phase 7 rasterizer (decided,
`docs/STATUS.md`): the rasterizer is a pure per-scanline function in ninge-core, driven
through a `ScanlineExecutor` trait that core defines with a sequential implementation. The
`rayon` executor lives outside core and is passed in as a generic parameter (static dispatch,
no thread dependency in core). Each scanline reads only an immutable snapshot and writes only
its own line, so scheduling cannot change the result; a test pins parallel output =
sequential output.
