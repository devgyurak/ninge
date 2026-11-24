// SPDX-License-Identifier: Apache-2.0
//! POS_TEST, VEC_TEST and BOX_TEST as a device. Expected values come from
//! `docs/hw/geometry-engine.md` ("POS_TEST, VEC_TEST and BOX_TEST"), worked out by hand. With the
//! matrices at their reset (the identity) a position `(x, y, z)` gives the clip point
//! `(x, y, z, 1000h)`, and the view volume is `-1000h <= x, y, z <= 1000h`.

use super::*;

const POS_RESULT: u32 = GEOMETRY_OFFSET + 0x220;
const VEC_RESULT: u32 = GEOMETRY_OFFSET + 0x230;
const BOX_TEST_CMD: u32 = 0x70;
const POS_TEST_CMD: u32 = 0x71;
const VEC_TEST_CMD: u32 = 0x72;
const VTX_XY: u32 = 0x25;
const BEGIN_VTXS: u32 = 0x40;
const VTX_16: u32 = 0x23;
const BOX_BIT: u32 = 1 << 1;
const TEST_BUSY: u32 = 1;

fn command(gpu: &mut Gpu3d, id: u32, params: &[u32]) {
    send(gpu, &[id]);
    send(gpu, params);
}

fn pos_test(gpu: &mut Gpu3d, x: i16, y: i16, z: i16) {
    let xy = u32::from(y as u16) << 16 | u32::from(x as u16);
    command(gpu, POS_TEST_CMD, &[xy, u32::from(z as u16)]);
}

fn pos_result(gpu: &Gpu3d) -> Vec<u32> {
    (0..4)
        .map(|i| read(gpu, POS_RESULT + 4 * i, Width::Word))
        .collect()
}

fn vec_result(gpu: &Gpu3d) -> Vec<u32> {
    (0..3)
        .map(|i| read(gpu, VEC_RESULT + 2 * i, Width::Half))
        .collect()
}

/// A 1.0.9 vector as the three 10-bit fields of VEC_TEST.
fn vec10(x: i32, y: i32, z: i32) -> u32 {
    let field = |v: i32| (v as u32) & 0x3FF;
    field(x) | field(y) << 10 | field(z) << 20
}

/// BOX_TEST with the corner `(x, y, z)` and the size `(w, h, d)`; returns GXSTAT bit 1.
fn box_test(gpu: &mut Gpu3d, corner: [i16; 3], size: [i16; 3]) -> bool {
    let pair = |lo: i16, hi: i16| u32::from(hi as u16) << 16 | u32::from(lo as u16);
    let params = [
        pair(corner[0], corner[1]),
        pair(corner[2], size[0]),
        pair(size[1], size[2]),
    ];
    command(gpu, BOX_TEST_CMD, &params);
    gxstat(gpu) & BOX_BIT != 0
}

#[test]
fn given_the_identity_clip_matrix_when_pos_test_runs_then_the_result_is_the_point_with_w_1() {
    let mut gpu = Gpu3d::new();
    pos_test(&mut gpu, 1, 2, 3);
    assert_eq!(pos_result(&gpu), vec![1, 2, 3, 0x1000]);
}

#[test]
fn given_a_translation_when_pos_test_runs_then_the_result_is_the_translated_point() {
    let mut gpu = in_mode(1);
    command(&mut gpu, 0x1C, &[0x100, 0x200, 0x300]);
    pos_test(&mut gpu, 1, 2, 3);
    assert_eq!(pos_result(&gpu), vec![0x101, 0x202, 0x303, 0x1000]);
}

#[test]
fn given_a_scale_when_pos_test_runs_then_the_coordinates_scale_and_w_stays_1() {
    let mut gpu = in_mode(1);
    command(&mut gpu, 0x1B, &[0x2000, 0x2000, 0x2000]);
    pos_test(&mut gpu, 1, 2, 3);
    assert_eq!(pos_result(&gpu), vec![2, 4, 6, 0x1000]);
}

#[test]
fn given_a_negative_coordinate_when_pos_test_runs_then_the_result_is_a_signed_word() {
    let mut gpu = Gpu3d::new();
    pos_test(&mut gpu, -1, 0, 0);
    assert_eq!(pos_result(&gpu)[0], 0xFFFF_FFFF);
}

#[test]
fn given_a_pos_test_in_flight_when_the_result_is_read_then_it_is_the_previous_one() {
    let mut gpu = Gpu3d::new();
    pos_test(&mut gpu, 1, 2, 3);
    let xy = 20u32 << 16 | 10;
    gpu.write(GXFIFO, Width::Word, POS_TEST_CMD, cx_at(0));
    gpu.write(GXFIFO, Width::Word, xy, cx_at(0));
    gpu.write(GXFIFO, Width::Word, 30, cx_at(0));
    assert_eq!(pos_result(&gpu)[0], 1);
}

#[test]
fn given_a_pos_test_when_it_runs_then_the_next_relative_vertex_starts_from_the_tested_position() {
    let mut gpu = Gpu3d::new();
    pos_test(&mut gpu, 5, 6, 7);
    command(&mut gpu, BEGIN_VTXS, &[0]);
    command(&mut gpu, VTX_XY, &[2 << 16 | 1]);
    command(&mut gpu, VTX_16, &[0, 0]);
    command(&mut gpu, VTX_16, &[0, 0]);
    let first = gpu.geometry_ram().vertices[0];
    assert_eq!((first.x, first.y, first.z), (1, 2, 7));
}

#[test]
fn given_the_identity_vector_matrix_when_vec_test_runs_then_the_components_have_12_fraction_bits() {
    // 256 and -256 are 0.5 and -0.5 with 9 fraction bits: 800h and F800h with 12 and a copied sign.
    let mut gpu = Gpu3d::new();
    command(&mut gpu, VEC_TEST_CMD, &[vec10(256, -256, 0)]);
    assert_eq!(vec_result(&gpu), vec![0x0800, 0xF800, 0]);
}

#[test]
fn given_a_vector_of_1_0_after_the_matrix_when_vec_test_runs_then_it_overflows_to_minus_1_0() {
    // A scale by 2 multiplied into the vector matrix turns 256 (0.5) into 512 (1.0): 1000h does not
    // fit in 12 fraction bits and a sign, and reads as F000h (-1.0).
    let mut gpu = in_mode(2);
    command(&mut gpu, 0x1A, &[0x2000, 0, 0, 0, 0x2000, 0, 0, 0, 0x2000]);
    command(&mut gpu, VEC_TEST_CMD, &[vec10(256, 0, 0)]);
    assert_eq!(vec_result(&gpu)[0], 0xF000);
}

#[test]
fn given_a_rotated_vector_matrix_when_vec_test_runs_then_the_vector_is_rotated() {
    let mut gpu = in_mode(2);
    command(
        &mut gpu,
        0x1A,
        &[0, 0x1000, 0, 0xFFFF_F000, 0, 0, 0, 0, 0x1000],
    );
    command(&mut gpu, VEC_TEST_CMD, &[vec10(256, 0, 0)]);
    assert_eq!(vec_result(&gpu), vec![0, 0x0800, 0]);
}

#[test]
fn given_the_result_registers_when_written_then_they_keep_their_values() {
    let mut gpu = Gpu3d::new();
    pos_test(&mut gpu, 1, 2, 3);
    write(&mut gpu, POS_RESULT, Width::Word, 0xFFFF_FFFF);
    assert_eq!(pos_result(&gpu)[0], 1);
}

// ------------------------------------------------------------------------------- BOX_TEST

#[test]
fn given_a_box_wholly_inside_the_view_volume_when_tested_then_the_result_is_inside() {
    let mut gpu = Gpu3d::new();
    assert!(box_test(&mut gpu, [-0x100; 3], [0x200; 3]));
}

#[test]
fn given_a_box_beyond_the_right_plane_when_tested_then_the_result_is_outside() {
    let mut gpu = Gpu3d::new();
    assert!(!box_test(&mut gpu, [0x3000, 0, 0], [0x100; 3]));
}

#[test]
fn given_a_box_straddling_a_plane_when_tested_then_the_result_is_inside() {
    let mut gpu = Gpu3d::new();
    assert!(box_test(
        &mut gpu,
        [0x800, -0x100, -0x100],
        [0x1000, 0x200, 0x200]
    ));
}

#[test]
fn given_a_box_touching_a_plane_when_tested_then_the_result_is_inside() {
    let mut gpu = Gpu3d::new();
    assert!(box_test(&mut gpu, [0x1000, 0, 0], [0x100; 3]));
}

#[test]
fn given_a_box_around_the_whole_view_volume_when_tested_then_only_the_faces_count_so_it_is_outside()
{
    let mut gpu = Gpu3d::new();
    assert!(!box_test(&mut gpu, [-0x2000; 3], [0x4000; 3]));
}

#[test]
fn given_a_box_past_the_far_plane_when_tested_then_the_result_is_outside() {
    let mut gpu = Gpu3d::new();
    assert!(!box_test(&mut gpu, [0, 0, 0x2000], [0x100; 3]));
}

#[test]
fn given_a_box_inside_after_an_outside_one_when_tested_then_the_result_follows_the_last_test() {
    let mut gpu = Gpu3d::new();
    box_test(&mut gpu, [0x3000, 0, 0], [0x100; 3]);
    assert!(box_test(&mut gpu, [0; 3], [0x100; 3]));
}

#[test]
fn given_a_translation_when_a_box_is_tested_then_the_clip_matrix_moves_it() {
    // The box at x = 0 is inside; translated by 3.0 it is beyond the right plane.
    let mut gpu = in_mode(1);
    command(&mut gpu, 0x1C, &[0x3000, 0, 0]);
    assert!(!box_test(&mut gpu, [0; 3], [0x100; 3]));
}

#[test]
fn given_a_box_test_in_flight_when_gxstat_is_read_then_the_test_busy_bit_is_set_and_the_result_is_old()
 {
    let mut gpu = Gpu3d::new();
    gpu.write(GXFIFO, Width::Word, BOX_TEST_CMD, cx_at(0));
    gpu.write(GXFIFO, Width::Word, 0, cx_at(0));
    gpu.write(GXFIFO, Width::Word, 0, cx_at(0));
    gpu.write(GXFIFO, Width::Word, 0, cx_at(0));
    assert_eq!(gxstat(&gpu) & (TEST_BUSY | BOX_BIT), TEST_BUSY);
}

#[test]
fn given_the_results_in_a_state_when_saved_and_loaded_then_they_are_kept() {
    let mut original = Gpu3d::new();
    pos_test(&mut original, 1, 2, 3);
    command(&mut original, VEC_TEST_CMD, &[vec10(256, -256, 0)]);
    let bytes = postcard::to_allocvec(&original).unwrap();
    let restored: Gpu3d = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(
        (
            restored == original,
            pos_result(&restored),
            vec_result(&restored)
        ),
        (true, vec![1, 2, 3, 0x1000], vec![0x0800, 0xF800, 0])
    );
}

// ----------------------------------------------------- BOX_TEST's parameter order

// Parameter 1 is `y << 16 | x`, parameter 2 `width << 16 | z` and parameter 3 `depth << 16 | height`.
// Each box below sticks out of the view volume along one axis only (the corner at 1.5), and its size
// along that axis is -1.0, which brings it back inside: the box is inside only if the right field is
// that axis' size. The other two sizes are 0x100, which would leave it outside.

#[test]
fn given_a_box_whose_width_alone_brings_it_back_inside_when_tested_then_the_width_is_parameter_2_high()
 {
    let mut gpu = Gpu3d::new();
    assert!(box_test(&mut gpu, [0x1800, 0, 0], [-0x1000, 0x100, 0x100]));
}

#[test]
fn given_a_box_whose_height_alone_brings_it_back_inside_when_tested_then_the_height_is_parameter_3_low()
 {
    let mut gpu = Gpu3d::new();
    assert!(box_test(&mut gpu, [0, 0x1800, 0], [0x100, -0x1000, 0x100]));
}

#[test]
fn given_a_box_whose_depth_alone_brings_it_back_inside_when_tested_then_the_depth_is_parameter_3_high()
 {
    let mut gpu = Gpu3d::new();
    assert!(box_test(&mut gpu, [0, 0, 0x1800], [0x100, 0x100, -0x1000]));
}

#[test]
fn given_the_same_boxes_with_the_sizes_swapped_when_tested_then_they_stay_outside() {
    let mut gpu = Gpu3d::new();
    let results = [
        box_test(&mut gpu, [0x1800, 0, 0], [0x100, -0x1000, 0x100]),
        box_test(&mut gpu, [0, 0x1800, 0], [0x100, 0x100, -0x1000]),
        box_test(&mut gpu, [0, 0, 0x1800], [-0x1000, 0x100, 0x100]),
    ];
    assert_eq!(results, [false, false, false]);
}

#[test]
fn given_a_vector_and_a_half_matrix_when_vec_test_runs_then_the_low_bits_of_the_result_are_kept() {
    // 1/512 (the vector's smallest step) times 0.5 (800h) is 1/1024, which is 4 in 12 fraction bits.
    let mut gpu = in_mode(2);
    command(&mut gpu, 0x1A, &[0x800, 0, 0, 0, 0x800, 0, 0, 0, 0x800]);
    command(&mut gpu, VEC_TEST_CMD, &[vec10(1, 0, 0)]);
    assert_eq!(vec_result(&gpu)[0], 4);
}
