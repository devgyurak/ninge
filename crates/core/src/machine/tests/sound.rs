// SPDX-License-Identifier: Apache-2.0
//! The sound registers and POWCNT2 through `Machine`
//! (`docs/hw/sound-registers.md`): the ARM7's bus reaching the device, the ARM9 not seeing it, and
//! a savestate taken mid-playback. Expected values are worked out by hand from the note: with TMR `FFF0h` a sample is 64 master cycles, so 4 words of PCM8 (16
//! samples) last 1,024.

use super::dma::machine;
use super::*;
use crate::cpu::Bus;

const SOUND0CNT: u32 = 0x0400_0400;
const SOUND0TMR: u32 = 0x0400_0408;
const SOUND0LEN: u32 = 0x0400_040C;
const SOUNDCNT: u32 = 0x0400_0500;
const SOUNDBIAS: u32 = 0x0400_0504;
const POWCNT2: u32 = 0x0400_0304;
/// Start (bit 31), one-shot (repeat 2), PCM8.
const ONE_SHOT_PCM8: u32 = 1 << 31 | 2 << 27;

/// A machine whose channel 0 was started as a 4-word PCM8 one-shot at the ARM7's clock plus
/// 1,000; the start time.
fn playing() -> (Machine, u64) {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1_000;
    m.bus7(at).write16(SOUND0TMR, 0xFFF0);
    m.bus7(at).write32(SOUND0LEN, 4);
    m.bus7(at).write32(SOUND0CNT, ONE_SHOT_PCM8);
    (m, at)
}

fn busy(m: &mut Machine, at: u64) -> bool {
    m.bus7(at).read32(SOUND0CNT) & (1 << 31) != 0
}

#[test]
fn given_the_arm7_wrote_soundcnt_when_each_cpu_reads_it_then_only_the_arm7_sees_it() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write16(SOUNDCNT, 0xFFFF);
    let arm7 = m.bus7(at + 1).read16(SOUNDCNT);
    let arm9 = m.bus9().read16(SOUNDCNT);
    assert_eq!((arm7, arm9), (0xBF7F, 0));
}

#[test]
fn given_the_arm9_writes_to_the_sound_window_when_the_arm7_reads_then_nothing_changed() {
    let mut m = machine();
    m.bus9().write32(SOUNDBIAS, 0x1FF);
    let at = m.inspect().arm7_clock() + 1;
    assert_eq!(m.bus7(at).read32(SOUNDBIAS), 0);
}

#[test]
fn given_a_started_one_shot_when_the_arm7_polls_then_the_bit_clears_at_the_computed_end() {
    let (mut m, at) = playing();
    let polls = [at + 1_023, at + 1_024].map(|t| busy(&mut m, t));
    assert_eq!(polls, [true, false]);
}

#[test]
fn given_a_savestate_mid_playback_when_loaded_then_the_bit_clears_at_the_same_time() {
    let (m, at) = playing();
    let state = m.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    let polls = [at + 1_023, at + 1_024].map(|t| busy(&mut loaded, t));
    assert_eq!((result, polls), (Ok(()), [true, false]));
}

#[test]
fn given_the_same_sound_writes_when_two_machines_save_then_the_states_are_identical() {
    let (a, _) = playing();
    let (b, _) = playing();
    assert_eq!(a.save_state(), b.save_state());
}

#[test]
fn given_a_started_channel_when_inspected_then_the_registers_and_end_time_are_visible() {
    let (m, at) = playing();
    let channel = m.inspect().spu().channel(0);
    assert_eq!(
        (channel.tmr(), channel.length(), channel.end()),
        (0xFFF0, 4, at + 1_024)
    );
}

#[test]
fn given_a_new_machine_when_the_arm7_reads_powcnt2_then_the_speakers_are_on_and_wifi_is_off() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    assert_eq!(m.bus7(at).read16(POWCNT2), 1);
}

#[test]
fn given_all_ones_written_to_powcnt2_when_read_back_then_only_bits_0_and_1_remain() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write16(POWCNT2, 0xFFFF);
    let read = m.bus7(at + 1).read16(POWCNT2);
    assert_eq!((read, m.inspect().power().powcnt2()), (3, 3));
}
