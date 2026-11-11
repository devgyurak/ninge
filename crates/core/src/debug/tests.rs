// SPDX-License-Identifier: Apache-2.0
//! `Inspect` shows the machine's real state: each test changes the state the way a CPU would (a
//! bus write, a register write, the clock) and reads it back through the view, so an accessor
//! wired to the wrong field or the wrong CPU fails. Expected values come from the `docs/hw/`
//! notes of the device each accessor shows, never from running the code.

use super::*;
use crate::cpu::Bus;
use crate::io::dma::DMA0SAD_ADDR;
use crate::io::ipc::{IPCFIFOCNT_ADDR, IPCFIFORECV_ADDR, IPCFIFOSEND_ADDR};
use crate::io::irq::{IE_ADDR, IME_ADDR, VBLANK_IRQ};
use crate::io::power::POSTFLG_ADDR;
use crate::io::timers::TM0CNT_L_ADDR;
use crate::io::timing::{DISPSTAT_ADDR, VBLANK_IRQ_ENABLE};
use crate::scheduler::{HBLANK_START_CYCLES, LINE_CYCLES};

const VRAMCNT_A: u32 = 0x0400_0240;
const WRAMCNT: u32 = 0x0400_0247;
const DISPCNT_A: u32 = 0x0400_0000;
const MAIN_RAM: u32 = 0x0200_0000;
const TM0CNT_H: u32 = TM0CNT_L_ADDR + 2;
/// TIMxCNT_H bit 7: the timer runs.
const TIMER_START: u16 = 0x80;
/// IPCFIFOCNT bit 15: the FIFOs are switched on.
const FIFO_ENABLE: u16 = 0x8000;
/// HALTCNT mode 2 (bits 6-7): halt until an interrupt.
const HALT_MODE: u8 = 0x80;
/// VRAMCNT_A: the bank on, MST 0, offset 0.
const BANK_ON: u8 = 0x80;
/// The timer's reload, 16 ticks from overflow.
const RELOAD_16_TICKS: u16 = 0xFFF0;
/// A tick takes two master cycles at the F/1 prescaler (`docs/hw/timers.md`, "Model").
const THREE_TICKS: u64 = 6;
const WORD: u32 = 0x1122_3344;

/// A machine after the ARM9 wrote a halfword to `addr`.
fn after_arm9_write16(addr: u32, value: u16) -> Machine {
    let mut m = Machine::new();
    m.bus9().write16(addr, value);
    m
}

/// A machine after the ARM9 wrote a word to `addr`.
fn after_arm9_write32(addr: u32, value: u32) -> Machine {
    let mut m = Machine::new();
    m.bus9().write32(addr, value);
    m
}

/// A machine after the ARM9 wrote a byte to `addr`.
fn after_arm9_write8(addr: u32, value: u8) -> Machine {
    let mut m = Machine::new();
    m.bus9().write8(addr, value);
    m
}

/// A machine whose timer 0 on `arm9`'s side was started at clock 0 from `RELOAD_16_TICKS`.
fn with_timer_started(arm9: bool) -> Machine {
    let mut m = Machine::new();
    match arm9 {
        true => {
            m.bus9().write16(TM0CNT_L_ADDR, RELOAD_16_TICKS);
            m.bus9().write16(TM0CNT_H, TIMER_START);
        }
        false => {
            m.bus7(0).write16(TM0CNT_L_ADDR, RELOAD_16_TICKS);
            m.bus7(0).write16(TM0CNT_H, TIMER_START);
        }
    }
    m
}

#[test]
fn given_a_register_set_on_the_arm9_when_inspected_then_only_the_arm9_shows_it() {
    let mut m = Machine::new();
    m.arm9.regs_mut().set(1, 0xA9);
    let view = m.inspect();
    assert_eq!(
        (view.arm9().regs().get(1), view.arm7().regs().get(1)),
        (0xA9, 0)
    );
}

#[test]
fn given_a_register_set_on_the_arm7_when_inspected_then_only_the_arm7_shows_it() {
    let mut m = Machine::new();
    m.arm7.regs_mut().set(2, 0xA7);
    let view = m.inspect();
    assert_eq!(
        (view.arm9().regs().get(2), view.arm7().regs().get(2)),
        (0, 0xA7)
    );
}

#[test]
fn given_a_halted_arm7_when_inspected_then_the_arm7_is_halted_and_the_arm9_is_not() {
    let mut m = Machine::new();
    m.arm7.set_halted(true);
    let view = m.inspect();
    assert_eq!((view.arm9().halted(), view.arm7().halted()), (false, true));
}

#[test]
fn given_an_arm7_clock_ahead_of_the_master_clock_when_inspected_then_it_shows_the_arm7_clock() {
    let mut m = Machine::new();
    m.arm7_clock = 77;
    assert_eq!(m.inspect().arm7_clock(), 77);
}

#[test]
fn given_swi_numbers_recorded_per_cpu_when_inspected_then_each_record_is_its_cpus() {
    let mut m = Machine::new();
    m.bios.arm9.last = Some(0x05);
    m.bios.arm7.last = Some(0x06);
    m.bios.arm7.last_unimplemented = Some(0x1F);
    let view = m.inspect();
    assert_eq!(
        (
            view.swi_record9().last(),
            view.swi_record9().last_unimplemented(),
            view.swi_record7().last(),
            view.swi_record7().last_unimplemented(),
        ),
        (Some(0x05), None, Some(0x06), Some(0x1F))
    );
}

#[test]
fn given_a_machine_advanced_by_cycles_when_inspected_then_the_scheduler_shows_the_clock() {
    let mut m = Machine::new();
    m.scheduler.advance(123);
    assert_eq!(m.inspect().scheduler().now(), 123);
}

#[test]
fn given_a_new_machine_when_inspected_then_pending_lists_the_line_events_in_firing_order() {
    let m = Machine::new();
    let kinds: Vec<(u64, EventKind)> = m
        .inspect()
        .scheduler()
        .pending()
        .iter()
        .map(|event| (event.time, event.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (HBLANK_START_CYCLES, EventKind::HBlankStart),
            (LINE_CYCLES, EventKind::LineStart),
        ]
    );
}

#[test]
fn given_a_running_arm9_timer_when_inspected_then_pending_lists_its_overflow_first() {
    let m = with_timer_started(true);
    let kinds: Vec<EventKind> = m
        .inspect()
        .scheduler()
        .pending()
        .iter()
        .map(|event| event.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::TimerOverflow {
                arm9: true,
                index: 0
            },
            EventKind::HBlankStart,
            EventKind::LineStart,
        ]
    );
}

#[test]
fn given_a_started_line_when_inspected_then_vcount_and_timing_show_the_line() {
    let mut m = Machine::new();
    let requests = m.io.timing_mut().line_started();
    m.io.request_irq(requests);
    let view = m.inspect();
    assert_eq!((view.vcount(), view.timing().vcount()), (1, 1));
}

#[test]
fn given_dispstat_written_by_the_arm9_when_inspected_then_only_the_arm9_dispstat_has_it() {
    let m = after_arm9_write16(DISPSTAT_ADDR, VBLANK_IRQ_ENABLE);
    let view = m.inspect();
    assert_eq!((view.dispstat9(), view.dispstat7()), (VBLANK_IRQ_ENABLE, 0));
}

#[test]
fn given_dispcnt_a_written_when_inspected_then_gpu2d_shows_it() {
    let m = after_arm9_write32(DISPCNT_A, 0x0001_0100);
    assert_eq!(m.inspect().gpu2d().dispcnt_a(), 0x0001_0100);
}

#[test]
fn given_ime_and_ie_written_by_the_arm9_when_inspected_then_only_the_arm9_controller_has_them() {
    let mut m = Machine::new();
    m.bus9().write32(IME_ADDR, 1);
    m.bus9().write32(IE_ADDR, VBLANK_IRQ);
    let view = m.inspect();
    assert_eq!(
        (
            view.irq9().ime(),
            view.irq9().ie(),
            view.irq7().ime(),
            view.irq7().ie()
        ),
        (true, VBLANK_IRQ, false, 0)
    );
}

#[test]
fn given_a_flag_requested_for_the_arm7_when_inspected_then_only_the_arm7_controller_has_it() {
    let mut m = Machine::new();
    m.io.request_irq(crate::io::irq::Requests::for_cpu::<false>(VBLANK_IRQ));
    let view = m.inspect();
    assert_eq!((view.irq9().if_(), view.irq7().if_()), (0, VBLANK_IRQ));
}

#[test]
fn given_a_word_sent_by_the_arm9_when_inspected_then_ipc_lists_it_in_the_arm9_send_fifo() {
    let mut m = Machine::new();
    m.bus9().write16(IPCFIFOCNT_ADDR, FIFO_ENABLE);
    m.bus9().write32(IPCFIFOSEND_ADDR, WORD);
    let view = m.inspect();
    let sent9: Vec<u32> = view.ipc().send_words::<true>().collect();
    let sent7: Vec<u32> = view.ipc().send_words::<false>().collect();
    assert_eq!((sent9, sent7), (vec![WORD], Vec::new()));
}

#[test]
fn given_a_timer_started_by_the_arm9_when_inspected_then_only_the_arm9_timers_count() {
    let m = with_timer_started(true);
    let view = m.inspect();
    assert_eq!(
        (
            view.timers9().reload(0),
            view.timers9().counter(0, THREE_TICKS),
            view.timers7().reload(0),
        ),
        (RELOAD_16_TICKS, RELOAD_16_TICKS + 3, 0)
    );
}

#[test]
fn given_a_timer_started_by_the_arm7_when_inspected_then_only_the_arm7_timers_count() {
    let m = with_timer_started(false);
    let view = m.inspect();
    assert_eq!(
        (
            view.timers7().reload(0),
            view.timers7().counter(0, THREE_TICKS),
            view.timers9().reload(0),
        ),
        (RELOAD_16_TICKS, RELOAD_16_TICKS + 3, 0)
    );
}

#[test]
fn given_a_source_written_by_the_arm9_when_inspected_then_only_the_arm9_dma_has_it() {
    let m = after_arm9_write32(DMA0SAD_ADDR, MAIN_RAM);
    let view = m.inspect();
    assert_eq!(
        (view.dma9().source(0), view.dma7().source(0)),
        (MAIN_RAM, 0)
    );
}

#[test]
fn given_a_halt_written_by_the_arm7_when_inspected_then_power_shows_the_request() {
    let mut m = Machine::new();
    m.bus7(0).write8(POSTFLG_ADDR + 1, HALT_MODE);
    assert!(m.inspect().power().halt_requested());
}

#[test]
fn given_vramcnt_a_written_when_inspected_then_vram_a_shows_the_bank_enabled() {
    let m = after_arm9_write8(VRAMCNT_A, BANK_ON);
    assert_eq!(
        m.inspect().vram_a(),
        VramBankState {
            enabled: true,
            mst: 0,
            offset: 0,
            mapping: VramMapping::Lcdc
        }
    );
}

#[test]
fn given_wramcnt_written_by_the_arm9_when_inspected_then_wramcnt_shows_the_mode() {
    let m = after_arm9_write8(WRAMCNT, 1);
    assert_eq!(m.inspect().wramcnt().mode, 1);
}

#[test]
fn given_a_word_in_main_ram_when_the_arm9_peeks_then_each_width_reads_its_little_endian_part() {
    let m = after_arm9_write32(MAIN_RAM, WORD);
    let view = m.inspect();
    assert_eq!(
        (
            view.arm9_peek8(MAIN_RAM),
            view.arm9_peek16(MAIN_RAM),
            view.arm9_peek32(MAIN_RAM)
        ),
        (0x44, 0x3344, WORD)
    );
}

#[test]
fn given_a_word_in_main_ram_when_the_arm7_peeks_then_each_width_reads_its_little_endian_part() {
    let m = after_arm9_write32(MAIN_RAM, WORD);
    let view = m.inspect();
    assert_eq!(
        (
            view.arm7_peek8(0, MAIN_RAM),
            view.arm7_peek16(0, MAIN_RAM),
            view.arm7_peek32(0, MAIN_RAM)
        ),
        (0x44, 0x3344, WORD)
    );
}

#[test]
fn given_a_running_arm7_timer_when_the_arm7_peeks_its_counter_then_it_reads_as_of_the_clock_given()
{
    let m = with_timer_started(false);
    let view = m.inspect();
    assert_eq!(
        (
            view.arm7_peek16(0, TM0CNT_L_ADDR),
            view.arm7_peek16(THREE_TICKS, TM0CNT_L_ADDR)
        ),
        (RELOAD_16_TICKS, RELOAD_16_TICKS + 3)
    );
}

#[test]
fn given_a_word_in_the_arm7_send_fifo_when_the_arm9_peeks_the_receive_port_twice_then_the_cpu_read_still_gets_it()
 {
    let mut m = Machine::new();
    m.bus9().write16(IPCFIFOCNT_ADDR, FIFO_ENABLE);
    m.bus7(0).write16(IPCFIFOCNT_ADDR, FIFO_ENABLE);
    m.bus7(0).write32(IPCFIFOSEND_ADDR, WORD);
    let peeks = (
        m.inspect().arm9_peek32(IPCFIFORECV_ADDR),
        m.inspect().arm9_peek32(IPCFIFORECV_ADDR),
    );
    assert_eq!(
        (peeks, m.bus9().read32(IPCFIFORECV_ADDR)),
        ((WORD, WORD), WORD)
    );
}

#[test]
fn given_a_running_arm9_timer_and_an_advanced_clock_when_the_arm9_peeks_its_counter_then_it_reads_as_of_the_master_clock()
 {
    let mut m = with_timer_started(true);
    m.scheduler.advance(THREE_TICKS);
    assert_eq!(m.inspect().arm9_peek16(TM0CNT_L_ADDR), RELOAD_16_TICKS + 3);
}
