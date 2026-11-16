// SPDX-License-Identifier: Apache-2.0
//! The host's input reaching the keypad registers (`docs/hw/keypad.md`, "Where the bits come
//! from").

use super::*;
use crate::cart::test_image::Image;
use crate::input::{Buttons, Input, Touch};

const KEYINPUT: u32 = 0x0400_0130;
const EXTKEYIN: u32 = 0x0400_0136;

fn held(buttons: Buttons) -> Input {
    Input::new().with_buttons(buttons)
}

#[test]
fn given_no_frame_has_run_when_the_registers_are_read_then_they_read_released() {
    let machine = Machine::new();
    assert_eq!(machine.inspect().arm9_peek16(KEYINPUT), 0x03FF);
}

#[test]
fn given_an_input_passed_to_run_frame_when_the_frame_runs_then_both_cpus_read_the_held_key_as_zero()
{
    let mut machine = Machine::new();
    machine.run_frame(held(Buttons::A | Buttons::START));
    assert_eq!(
        (
            machine.inspect().arm9_peek16(KEYINPUT),
            machine.inspect().arm7_peek16(0, KEYINPUT)
        ),
        (0x03F6, 0x03F6)
    );
}

#[test]
fn given_a_held_key_when_a_later_frame_gets_the_default_input_then_the_key_is_released() {
    let mut machine = Machine::new();
    machine.run_frame(held(Buttons::B));
    machine.run_frame(Input::default());
    assert_eq!(machine.inspect().arm9_peek16(KEYINPUT), 0x03FF);
}

#[test]
fn given_x_y_and_a_closed_hinge_when_a_frame_runs_then_the_arm7_extkeyin_follows_them() {
    let mut machine = Machine::new();
    machine.run_frame(
        Input::new()
            .with_buttons(Buttons::X)
            .with_hinge_closed(true),
    );
    // X held (bit 0 clear), Y released, the unknown bits and the pen released, the hinge closed.
    assert_eq!(machine.inspect().arm7_peek16(0, EXTKEYIN), 0x00FE);
}

#[test]
fn given_a_touch_point_when_a_frame_runs_then_the_pen_bit_still_reads_released() {
    let mut machine = Machine::new();
    machine.run_frame(Input::new().with_touch(Some(Touch { x: 10, y: 20 })));
    assert_eq!(machine.inspect().arm7_peek16(0, EXTKEYIN), 0x007F);
}

#[test]
fn given_a_frame_run_with_an_input_when_inspected_then_it_is_the_one_the_frame_began_with() {
    let mut machine = Machine::new();
    machine.run_frame(held(Buttons::L));
    assert_eq!(machine.inspect().input(), held(Buttons::L));
}

#[test]
fn given_a_held_input_when_a_rom_is_loaded_then_the_input_is_cleared_with_everything_else() {
    let mut machine = Machine::new();
    machine.run_frame(held(Buttons::A));
    machine
        .load_rom(Image::valid().build())
        .expect("synthetic image");
    assert_eq!(machine.inspect().input(), Input::default());
}

#[test]
fn given_a_new_machine_when_the_audio_is_asked_for_then_it_is_empty_until_the_spu_exists() {
    assert_eq!(Machine::new().audio(), &[] as &[i16]);
}
