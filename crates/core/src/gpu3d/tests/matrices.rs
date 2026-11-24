// SPDX-License-Identifier: Apache-2.0
//! The matrix commands, the stacks and the result registers as a device. Expected values come from
//! `docs/hw/geometry-engine.md` ("Matrices") and GBATEK's matrix chapters, worked out by hand:
//! `1.0` is `1000h`, a multiply command computes `C = M x C`, the clip matrix is position times
//! projection, and MTX_SCALE leaves the vector matrix alone even in mode 2.

use super::*;
use crate::gpu3d::matrix::{IDENTITY, ONE};

const CLIPMTX: u32 = GEOMETRY_OFFSET + 0x240;
const VECMTX: u32 = GEOMETRY_OFFSET + 0x280;
const MTX_IDENTITY: u32 = 0x15;
const MTX_LOAD_4X3: u32 = 0x17;
const MTX_MULT_3X3: u32 = 0x1A;
const MTX_SCALE: u32 = 0x1B;
const MTX_TRANS: u32 = 0x1C;
/// `-1.0`.
const MINUS_ONE: u32 = (-ONE) as u32;
const TWO: u32 = 2 * ONE as u32;

fn words(gpu: &Gpu3d, base: u32, count: u32) -> Vec<u32> {
    (0..count)
        .map(|i| read(gpu, base + 4 * i, Width::Word))
        .collect()
}

/// CLIPMTX_RESULT: 16 words.
fn clip(gpu: &Gpu3d) -> Vec<u32> {
    words(gpu, CLIPMTX, 16)
}

/// VECMTX_RESULT: 9 words.
fn vector(gpu: &Gpu3d) -> Vec<u32> {
    words(gpu, VECMTX, 9)
}

fn identity_words() -> Vec<u32> {
    IDENTITY.iter().map(|&e| e as u32).collect()
}

/// A command and its parameters as unpacked GXFIFO words.
fn command(gpu: &mut Gpu3d, id: u32, params: &[u32]) {
    send(gpu, &[id]);
    send(gpu, params);
}

fn mode(gpu: &mut Gpu3d, mode: u32) {
    command(gpu, MTX_MODE.into(), &[mode]);
}

/// `diag` on the diagonal's first three places and `bottom` in the bottom row's first three.
fn matrix_words(diag: [u32; 3], bottom: [u32; 3]) -> Vec<u32> {
    vec![
        diag[0], 0, 0, 0, //
        0, diag[1], 0, 0, //
        0, 0, diag[2], 0, //
        bottom[0], bottom[1], bottom[2], ONE as u32,
    ]
}

#[test]
fn given_a_new_device_when_the_result_registers_are_read_then_the_matrices_are_the_identity() {
    let gpu = Gpu3d::new();
    let vec_identity = vec![ONE as u32, 0, 0, 0, ONE as u32, 0, 0, 0, ONE as u32];
    assert_eq!((clip(&gpu), vector(&gpu)), (identity_words(), vec_identity));
}

#[test]
fn given_the_projection_mode_when_a_4x3_matrix_is_loaded_then_the_clip_matrix_is_that_matrix() {
    let mut gpu = in_mode(0);
    let params = [
        ONE as u32, 0, 0, //
        0, ONE as u32, 0, //
        0, 0, ONE as u32, //
        0x800, 0, 0,
    ];
    command(&mut gpu, MTX_LOAD_4X3, &params);
    assert_eq!(clip(&gpu), matrix_words([ONE as u32; 3], [0x800, 0, 0]));
}

#[test]
fn given_the_position_mode_when_a_translation_is_multiplied_then_the_clip_matrix_has_it_in_the_bottom_row()
 {
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[TWO, ONE as u32, MINUS_ONE]);
    assert_eq!(
        clip(&gpu),
        matrix_words([ONE as u32; 3], [TWO, ONE as u32, MINUS_ONE])
    );
}

#[test]
fn given_a_translation_and_then_a_scale_when_multiplied_then_the_scale_acts_first_on_the_left() {
    // C = S x T: the scale's diagonal multiplies the translation's rows, and the bottom row
    // (0, 0, 0, 1.0 in S) keeps the translation as it is.
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[TWO, ONE as u32, MINUS_ONE]);
    command(&mut gpu, MTX_SCALE, &[TWO, TWO, TWO]);
    assert_eq!(
        clip(&gpu),
        matrix_words([TWO; 3], [TWO, ONE as u32, MINUS_ONE])
    );
}

#[test]
fn given_a_position_and_a_projection_when_the_clip_matrix_is_read_then_it_is_position_times_projection()
 {
    // Position S x T (above) times a projection that is the identity with 0.5 in the bottom row's
    // first place: row 3 is (2, 1, -1, 1) x P = (2.5, 1, -1, 1).
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[TWO, ONE as u32, MINUS_ONE]);
    command(&mut gpu, MTX_SCALE, &[TWO, TWO, TWO]);
    mode(&mut gpu, 0);
    let params = [
        ONE as u32, 0, 0, 0, ONE as u32, 0, 0, 0, ONE as u32, 0x800, 0, 0,
    ];
    command(&mut gpu, MTX_LOAD_4X3, &params);
    assert_eq!(
        clip(&gpu),
        matrix_words([TWO; 3], [0x2800, ONE as u32, MINUS_ONE])
    );
}

/// Mode 2 with a quarter turn about Z multiplied in, then a scale by 2.
fn turned_and_scaled() -> Gpu3d {
    let mut gpu = in_mode(2);
    command(&mut gpu, MTX_IDENTITY, &[]);
    let rotation = [
        0, ONE as u32, 0, //
        MINUS_ONE, 0, 0, //
        0, 0, ONE as u32,
    ];
    command(&mut gpu, MTX_MULT_3X3, &rotation);
    command(&mut gpu, MTX_SCALE, &[TWO, TWO, TWO]);
    gpu
}

#[test]
fn given_mode_2_when_a_rotation_is_multiplied_then_the_vector_matrix_holds_it() {
    assert_eq!(
        vector(&turned_and_scaled()),
        vec![0, ONE as u32, 0, MINUS_ONE, 0, 0, 0, 0, ONE as u32]
    );
}

#[test]
fn given_mode_2_when_a_scale_is_multiplied_then_only_the_position_matrix_is_scaled() {
    let gpu = turned_and_scaled();
    // The position matrix is S x R: the rotation's rows doubled in the upper 3x3.
    assert_eq!(gpu.position_matrix()[..3], [0, 2 * ONE, 0]);
}

#[test]
fn given_mode_1_when_a_rotation_is_multiplied_then_the_vector_matrix_is_unchanged() {
    let mut gpu = in_mode(1);
    command(
        &mut gpu,
        MTX_MULT_3X3,
        &[0, ONE as u32, 0, MINUS_ONE, 0, 0, 0, 0, ONE as u32],
    );
    assert_eq!(*gpu.vector_matrix(), IDENTITY);
}

#[test]
fn given_the_texture_mode_when_a_translation_is_multiplied_then_only_the_texture_matrix_changes() {
    let mut gpu = in_mode(3);
    command(&mut gpu, MTX_TRANS, &[1, 2, 3]);
    assert_eq!(
        (gpu.texture_matrix()[12..15].to_vec(), clip(&gpu)),
        (vec![1, 2, 3], identity_words())
    );
}

// ----------------------------------------------------------------------------- the stacks

#[test]
fn given_a_pushed_matrix_when_the_current_one_changes_and_is_popped_then_the_pushed_one_returns() {
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[1, 2, 3]);
    push(&mut gpu, 1);
    command(&mut gpu, MTX_IDENTITY, &[]);
    pop(&mut gpu, 1);
    assert_eq!(gpu.position_matrix()[12..15].to_vec(), vec![1, 2, 3]);
}

#[test]
fn given_a_push_when_it_runs_then_the_current_matrix_is_not_changed() {
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[1, 2, 3]);
    push(&mut gpu, 1);
    assert_eq!(gpu.position_matrix()[12..15].to_vec(), vec![1, 2, 3]);
}

#[test]
fn given_mode_1_when_a_matrix_is_pushed_and_popped_then_the_vector_matrix_comes_back_too() {
    let mut gpu = in_mode(2);
    command(
        &mut gpu,
        MTX_MULT_3X3,
        &[0, ONE as u32, 0, MINUS_ONE, 0, 0, 0, 0, ONE as u32],
    );
    mode(&mut gpu, 1);
    push(&mut gpu, 1);
    command(&mut gpu, MTX_IDENTITY, &[]);
    pop(&mut gpu, 1);
    assert_eq!(gpu.vector_matrix()[..3], [0, ONE, 0]);
}

#[test]
fn given_a_stored_matrix_when_another_is_restored_from_that_entry_then_it_is_loaded() {
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[7, 8, 9]);
    command(&mut gpu, MTX_STORE.into(), &[5]);
    command(&mut gpu, MTX_IDENTITY, &[]);
    command(&mut gpu, MTX_RESTORE.into(), &[5]);
    assert_eq!(gpu.position_matrix()[12..15].to_vec(), vec![7, 8, 9]);
}

#[test]
fn given_two_pushes_when_popped_with_offset_2_then_the_older_entry_is_loaded() {
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[1, 0, 0]);
    push(&mut gpu, 1);
    command(&mut gpu, MTX_TRANS, &[1, 0, 0]);
    push(&mut gpu, 1);
    pop(&mut gpu, 2);
    assert_eq!(gpu.position_matrix()[12], 1);
}

#[test]
fn given_the_projection_mode_when_a_matrix_is_pushed_and_restored_then_the_single_entry_holds_it() {
    let mut gpu = in_mode(0);
    command(&mut gpu, MTX_TRANS, &[4, 5, 6]);
    command(&mut gpu, MTX_STORE.into(), &[0]);
    command(&mut gpu, MTX_IDENTITY, &[]);
    command(&mut gpu, MTX_RESTORE.into(), &[0]);
    assert_eq!(gpu.projection_matrix()[12..15].to_vec(), vec![4, 5, 6]);
}

#[test]
fn given_the_projection_mode_when_a_second_push_finds_entry_1_then_the_error_is_set_and_nothing_is_stored()
 {
    let mut gpu = in_mode(0);
    command(&mut gpu, MTX_TRANS, &[4, 5, 6]);
    push(&mut gpu, 1);
    command(&mut gpu, MTX_TRANS, &[1, 1, 1]);
    push(&mut gpu, 1);
    pop(&mut gpu, 1);
    pop(&mut gpu, 1);
    // The first push stored (4, 5, 6) at entry 0; the second found entry 1 and stored nothing; the
    // pops step the 1-bit pointer back and load entry 0 after the second one, entry 1 (nothing)
    // after the first.
    assert_eq!(
        (error(&gpu), gpu.projection_matrix()[12..15].to_vec()),
        (true, vec![4, 5, 6])
    );
}

#[test]
fn given_entry_31_when_a_matrix_is_stored_and_restored_then_it_is_kept_and_the_error_is_set() {
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[3, 3, 3]);
    command(&mut gpu, MTX_STORE.into(), &[31]);
    command(&mut gpu, MTX_IDENTITY, &[]);
    command(&mut gpu, MTX_RESTORE.into(), &[31]);
    assert_eq!(
        (error(&gpu), gpu.position_matrix()[12..15].to_vec()),
        (true, vec![3, 3, 3])
    );
}

// ------------------------------------------------------------------------------------ timing

#[test]
fn given_an_identity_in_flight_when_the_clip_matrix_is_read_then_it_is_the_matrix_from_before() {
    let mut gpu = in_mode(1);
    command(&mut gpu, MTX_TRANS, &[1, 2, 3]);
    raw_identity(&mut gpu);
    assert_eq!(clip(&gpu)[12..15].to_vec(), vec![1, 2, 3]);
}

/// MTX_IDENTITY written without draining, so it is executing.
fn raw_identity(gpu: &mut Gpu3d) {
    gpu.write(GXFIFO, Width::Word, MTX_IDENTITY, cx_at(0));
}

#[test]
fn given_the_identity_command_in_mode_2_when_it_ends_then_both_matrices_are_the_identity() {
    let mut gpu = turned_and_scaled();
    command(&mut gpu, MTX_IDENTITY, &[]);
    assert_eq!(
        (*gpu.position_matrix(), *gpu.vector_matrix()),
        (IDENTITY, IDENTITY)
    );
}

#[test]
fn given_matrices_in_a_state_when_saved_and_loaded_then_they_are_kept() {
    let original = turned_and_scaled();
    let bytes = postcard::to_allocvec(&original).unwrap();
    let restored: Gpu3d = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(restored, original);
}
