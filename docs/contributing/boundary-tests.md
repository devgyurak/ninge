# Boundary behavior to cover

Reference detail behind `AGENTS.md` §5.8. Emulator bugs live at the edges. Cover:

- PC-relative reads (pipeline offset +8 / +4)
- writes to r15 and mode switches
- banked registers on exception entry
- ARM/Thumb interworking, including the v5-only `LDR PC` / `POP {PC}` switch
- unaligned accesses per CPU
- open bus and mirrors
- VRAM bank remaps and WRAMCNT changes
- an IRQ raised while IME=0
- FIFO full/empty error flags
- timer overflow cascade
- DMA start modes
- scheduler ties
