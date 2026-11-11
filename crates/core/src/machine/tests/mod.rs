// SPDX-License-Identifier: Apache-2.0
//! Tests of `Machine`: construction, ROM loading, frames and the scheduler loop.

use super::*;
use serde::de::value::{Error, MapDeserializer};

mod arm9;
mod bios;
mod dma;
mod halt;
mod ipc;
mod irq;
mod rebuild;
mod scheduler;
mod swi_data;
mod timers;
mod timing;

fn run_frames(machine: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| machine.run_frame());
}

#[test]
fn given_new_machine_when_reading_each_screen_then_length_is_256_by_192() {
    let machine = Machine::new();
    let lengths = [
        machine.frame(Screen::Top).len(),
        machine.frame(Screen::Bottom).len(),
    ];
    assert_eq!(lengths, [49_152, 49_152]);
}

#[test]
fn given_new_machine_when_reading_frames_then_every_pixel_is_black() {
    let machine = Machine::default();
    let all_black = [Screen::Top, Screen::Bottom]
        .iter()
        .all(|&s| machine.frame(s).iter().all(|&p| p == 0));
    assert!(all_black);
}

#[test]
fn given_saved_machine_when_loaded_onto_a_fresh_one_then_both_screens_are_full_size() {
    let mut saved = Machine::new();
    saved.run_frame();
    let mut machine = Machine::new();
    let result = machine.load_state(&saved.save_state());
    let lengths = [
        machine.frame(Screen::Top).len(),
        machine.frame(Screen::Bottom).len(),
    ];
    assert_eq!((result, lengths), (Ok(()), [49_152, 49_152]));
}

#[test]
fn given_rom_bytes_when_loading_then_rom_is_kept_verbatim() {
    let mut machine = Machine::new();
    let image = crate::cart::test_image::Image::valid().build();
    machine.load_rom(image.clone()).expect("valid image");
    assert_eq!(machine.rom(), image.as_slice());
}

#[test]
fn given_six_byte_rom_when_loading_then_it_is_refused_as_too_short_and_not_kept() {
    let mut machine = Machine::new();
    let result = machine.load_rom(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]);
    assert_eq!(
        (result, machine.rom().len()),
        (Err(crate::cart::HeaderError::ImageTooShort { len: 6 }), 0)
    );
}

#[test]
fn given_new_machine_when_running_one_frame_then_frame_count_is_one() {
    let mut machine = Machine::new();
    machine.run_frame();
    assert_eq!(machine.frame_count(), 1);
}

#[test]
fn given_new_machine_when_reading_counters_then_frame_count_is_zero_and_rom_is_empty() {
    let machine = Machine::new();
    assert_eq!((machine.frame_count(), machine.rom().len()), (0, 0));
}

#[test]
fn given_empty_rom_when_running_a_frame_then_it_is_refused_rom_is_empty_and_frame_count_is_one() {
    let mut machine = Machine::new();
    let result = machine.load_rom(Vec::new());
    machine.run_frame();
    assert_eq!(
        (result, machine.rom().len(), machine.frame_count()),
        (
            Err(crate::cart::HeaderError::ImageTooShort { len: 0 }),
            0,
            1
        )
    );
}

#[test]
fn given_odd_length_rom_when_running_a_frame_then_it_is_refused_rom_is_empty_and_frame_count_is_one()
 {
    let mut machine = Machine::new();
    let result = machine.load_rom(vec![1, 2, 3]);
    machine.run_frame();
    assert_eq!(
        (result, machine.rom().len(), machine.frame_count()),
        (
            Err(crate::cart::HeaderError::ImageTooShort { len: 3 }),
            0,
            1
        )
    );
}

#[test]
fn given_three_frames_run_when_reading_frame_count_then_it_is_three() {
    let mut machine = Machine::new();
    machine.run_frame();
    machine.run_frame();
    machine.run_frame();
    assert_eq!(machine.frame_count(), 3);
}

#[test]
fn given_machine_that_ran_7_frames_when_saved_and_loaded_then_rom_is_empty_and_frame_count_is_seven()
 {
    let mut saved = Machine::new();
    run_frames(&mut saved, 7);
    let mut machine = Machine::new();
    let result = machine.load_state(&saved.save_state());
    assert_eq!(
        (result, machine.rom().len(), machine.frame_count()),
        (Ok(()), 0, 7)
    );
}

#[test]
fn given_savestate_map_with_only_frame_count_when_deserializing_machine_then_it_is_refused() {
    let map = MapDeserializer::<_, Error>::new([("frame_count", 7u32)].into_iter());
    assert!(Machine::deserialize(map).is_err());
}

#[test]
fn given_savestate_map_without_frame_count_when_deserializing_machine_then_it_is_refused() {
    let map = MapDeserializer::<_, Error>::new(std::iter::empty::<(&str, u32)>());
    assert!(Machine::deserialize(map).is_err());
}

#[test]
fn given_machine_that_ran_two_frames_when_loading_rom_then_frame_count_is_zero() {
    let mut machine = Machine::new();
    machine.run_frame();
    machine.run_frame();
    machine
        .load_rom(crate::cart::test_image::Image::valid().build())
        .expect("valid image");
    assert_eq!(machine.frame_count(), 0);
}
