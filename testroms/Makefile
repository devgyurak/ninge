# SPDX-License-Identifier: Apache-2.0
# Builds every test ROM into build/ and writes build/SHA256SUMS. Run it through
# scripts/build_testroms.sh so the toolchain is the pinned BlocksDS image.
# Built ROMs are never committed (AGENTS.md section 3.1); build/ is git-ignored.
WONDERFUL ?= /opt/wonderful
GCC_BIN := $(WONDERFUL)/toolchain/gcc-arm-none-eabi/bin
AS := $(GCC_BIN)/arm-none-eabi-as
LD := $(GCC_BIN)/arm-none-eabi-ld
OBJCOPY := $(GCC_BIN)/arm-none-eabi-objcopy
NDSTOOL := $(WONDERFUL)/thirdparty/blocksds/core/tools/ndstool/ndstool

ROMS := smoke boot memmap vramdisp sched wramcnt irq disptime ipc
OUT := build

# Per-ROM load and entry addresses: ARM9_RAM_<rom>, ARM9_ENTRY_<rom>, ARM7_RAM_<rom> and
# ARM7_ENTRY_<rom>. A ROM that sets none uses the defaults; an entry defaults to the load address.
ARM9_RAM_boot := 0x02000000
ARM9_ENTRY_boot := 0x02000100
ARM9_ENTRY_memmap := 0x02000100
ARM9_ENTRY_vramdisp := 0x02000100
ARM9_RAM_DEFAULT := 0x02000000
ARM7_RAM_DEFAULT := 0x02380000

.PHONY: all clean
all: $(OUT)/SHA256SUMS

$(OUT):
	mkdir -p $@

# One ROM: <name>/arm9.s + arm7.s + arm9.ld + arm7.ld -> build/<name>.nds
define ROM
$(OUT)/$(1).nds: $(1)/arm9.s $(1)/arm7.s $(1)/arm9.ld $(1)/arm7.ld | $(OUT)
	$(AS) -march=armv5te -o $(OUT)/$(1)-arm9.o $(1)/arm9.s
	$(AS) -march=armv4t -o $(OUT)/$(1)-arm7.o $(1)/arm7.s
	$(LD) -T $(1)/arm9.ld -o $(OUT)/$(1)-arm9.elf $(OUT)/$(1)-arm9.o
	$(LD) -T $(1)/arm7.ld -o $(OUT)/$(1)-arm7.elf $(OUT)/$(1)-arm7.o
	$(OBJCOPY) -O binary $(OUT)/$(1)-arm9.elf $(OUT)/$(1)-arm9.bin
	$(OBJCOPY) -O binary $(OUT)/$(1)-arm7.elf $(OUT)/$(1)-arm7.bin
	$(NDSTOOL) -c $$@ -9 $(OUT)/$(1)-arm9.bin -7 $(OUT)/$(1)-arm7.bin \
	  -r9 $(or $(ARM9_RAM_$(1)),$(ARM9_RAM_DEFAULT)) -e9 $(or $(ARM9_ENTRY_$(1)),$(or $(ARM9_RAM_$(1)),$(ARM9_RAM_DEFAULT))) \
	  -r7 $(or $(ARM7_RAM_$(1)),$(ARM7_RAM_DEFAULT)) -e7 $(or $(ARM7_ENTRY_$(1)),$(or $(ARM7_RAM_$(1)),$(ARM7_RAM_DEFAULT))) \
	  -h 0x4000
endef
$(foreach r,$(ROMS),$(eval $(call ROM,$(r))))

$(OUT)/SHA256SUMS: $(ROMS:%=$(OUT)/%.nds)
	cd $(OUT) && sha256sum $(notdir $^) > SHA256SUMS

clean:
	rm -rf $(OUT)
