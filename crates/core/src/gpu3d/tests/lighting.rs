// SPDX-License-Identifier: Apache-2.0
//! Vertex colours from the lights and the texture coordinate modes as a device. GBATEK gives the
//! registers and no equation, so the expected values come from the model in
//! `docs/hw/geometry-engine.md` ("Lighting", "Texture coordinates", "Unverified"), worked out by
//! hand:
//!
//! - a light of direction `L = (0, 0, -511)` and a normal `N = (0, 0, 511)` (1.0.9, so `511` is
//!   just under 1.0) give `-(N . L) = 261121`, which shifts right by 9 to 510 (of 512);
//! - a white diffuse reflection (31) under a white light (31) is then `31 x 31 x 510 / (31 x 512)`,
//!   which truncates to 30 in each channel;
//! - the halfway vector of `L` and the view direction `(0, 0, -1.0)` is `(0, 0, -512)`, so the
//!   specular dot is `511 x 512 >> 9 = 511`, the table index 127.

use super::*;

const NORMAL: u32 = 0x21;
const TEXCOORD: u32 = 0x22;
const DIF_AMB: u32 = 0x30;
const SPE_EMI: u32 = 0x31;
const LIGHT_VECTOR: u32 = 0x32;
const LIGHT_COLOR: u32 = 0x33;
const SHININESS: u32 = 0x34;
const POLYGON_ATTR: u32 = 0x29;
const TEXIMAGE_PARAM: u32 = 0x2A;
const BEGIN_VTXS: u32 = 0x40;
const VTX_16: u32 = 0x23;
const WHITE: u32 = 0x7FFF;
const LIGHT_0: u32 = 1;

/// A 1.0.9 vector as the three 10-bit fields of LIGHT_VECTOR and NORMAL.
fn vec10(x: i32, y: i32, z: i32) -> u32 {
    let field = |v: i32| (v as u32) & 0x3FF;
    field(x) | field(y) << 10 | field(z) << 20
}

fn command(gpu: &mut Gpu3d, id: u32, params: &[u32]) {
    send(gpu, &[id]);
    send(gpu, params);
}

/// The colours of the three vertices of one triangle that NORMAL `normal` colours, with `setup`
/// run first and the lights `enabled` in POLYGON_ATTR.
fn colors(enabled: u32, normal: u32, setup: impl FnOnce(&mut Gpu3d)) -> Vec<u16> {
    let mut gpu = Gpu3d::new();
    setup(&mut gpu);
    command(&mut gpu, POLYGON_ATTR, &[enabled]);
    command(&mut gpu, BEGIN_VTXS, &[0]);
    command(&mut gpu, NORMAL, &[normal]);
    three_vertices(&mut gpu);
    gpu.geometry_ram()
        .vertices
        .iter()
        .map(|v| v.color)
        .collect()
}

/// Three VTX_16 at the origin: one triangle.
fn three_vertices(gpu: &mut Gpu3d) {
    command(gpu, VTX_16, &[0, 0]);
    command(gpu, VTX_16, &[0, 0]);
    command(gpu, VTX_16, &[0, 0]);
}

/// Light 0 of colour `light`, direction (0, 0, -511); material `diffuse_ambient` and
/// `specular_emission`.
fn material(light: u32, diffuse_ambient: u32, specular_emission: u32) -> impl FnOnce(&mut Gpu3d) {
    move |gpu| {
        command(gpu, LIGHT_COLOR, &[light]);
        command(gpu, LIGHT_VECTOR, &[vec10(0, 0, -511)]);
        command(gpu, DIF_AMB, &[diffuse_ambient]);
        command(gpu, SPE_EMI, &[specular_emission]);
    }
}

/// `0x7BDE`: 30 in each channel.
const THIRTY: u16 = 30 | 30 << 5 | 30 << 10;
const FACING: u32 = 0x1FF << 20;

#[test]
fn given_a_white_diffuse_material_under_a_white_light_when_the_normal_faces_it_then_each_channel_is_30()
 {
    let got = colors(LIGHT_0, FACING, material(WHITE, WHITE, 0));
    assert_eq!(got, vec![THIRTY; 3]);
}

#[test]
fn given_a_normal_turned_away_from_the_light_when_lit_then_the_diffuse_term_is_zero() {
    let away = vec10(0, 0, -511);
    assert_eq!(colors(LIGHT_0, away, material(WHITE, WHITE, 0)), vec![0; 3]);
}

#[test]
fn given_no_light_enabled_when_normal_runs_then_only_the_emission_shows() {
    // Emission (10, 0, 0) in SPE_EMI bits 16-20.
    let got = colors(0, FACING, material(WHITE, WHITE, 10 << 16));
    assert_eq!(got, vec![10; 3]);
}

#[test]
fn given_only_light_1_enabled_when_light_0_is_the_white_one_then_it_is_ignored() {
    assert_eq!(colors(2, FACING, material(WHITE, WHITE, 0)), vec![0; 3]);
}

#[test]
fn given_an_ambient_reflection_when_lit_then_the_light_colour_times_it_adds_whatever_the_normal() {
    // Ambient white in DIF_AMB bits 16-30, diffuse 0: 31 x 31 / 31 = 31.
    let away = vec10(0, 0, -511);
    assert_eq!(
        colors(LIGHT_0, away, material(WHITE, WHITE << 16, 0)),
        vec![WHITE as u16; 3]
    );
}

#[test]
fn given_emission_and_ambient_when_they_sum_past_31_then_the_channel_clamps() {
    let got = colors(LIGHT_0, FACING, material(WHITE, WHITE << 16, 31 << 16));
    assert_eq!(got, vec![WHITE as u16; 3]);
}

/// A shininess table whose last entry (index 127) is 200, all others 0: the last word's top byte.
fn with_table(gpu: &mut Gpu3d) {
    let mut words = [0u32; 32];
    words[31] = 200 << 24;
    command(gpu, SHININESS, &words);
}

#[test]
fn given_the_shininess_table_when_the_specular_dot_is_511_then_entry_127_scales_the_specular_term()
{
    // 31 x 31 x 200 / (31 x 256) = 24.2: 24 per channel. Bit 15 of SPE_EMI switches the table on.
    let got = colors(LIGHT_0, FACING, |gpu| {
        material(WHITE, 0, WHITE | 1 << 15)(gpu);
        with_table(gpu);
    });
    assert_eq!(got, vec![24 | 24 << 5 | 24 << 10; 3]);
}

#[test]
fn given_the_table_switched_off_when_the_specular_dot_is_511_then_the_linear_ramp_gives_254() {
    // 31 x 31 x 254 / (31 x 256) = 30.76: 30.
    let got = colors(LIGHT_0, FACING, |gpu| {
        material(WHITE, 0, WHITE)(gpu);
        with_table(gpu);
    });
    assert_eq!(got, vec![THIRTY; 3]);
}

#[test]
fn given_two_lights_when_both_are_enabled_then_their_ambient_terms_add() {
    // Both lights are (10, 0, 0): ambient white gives 31 x 10 / 31 = 10 each, 20 in red.
    let got = colors(LIGHT_0 | 2, FACING, |gpu| {
        command(gpu, LIGHT_COLOR, &[10]);
        command(gpu, LIGHT_COLOR, &[1 << 30 | 10]);
        command(gpu, DIF_AMB, &[WHITE << 16]);
    });
    assert_eq!(got, vec![20; 3]);
}

#[test]
fn given_dif_amb_with_bit_15_when_it_runs_then_the_diffuse_colour_becomes_the_vertex_colour() {
    let mut gpu = Gpu3d::new();
    command(&mut gpu, BEGIN_VTXS, &[0]);
    command(&mut gpu, DIF_AMB, &[1 << 15 | 0x001F]);
    three_vertices(&mut gpu);
    assert_eq!(gpu.geometry_ram().vertices[0].color, 0x001F);
}

/// A quarter turn about Z in matrix mode 2: a vector along +x goes to +y and one along +y to -x.
fn quarter_turn(gpu: &mut Gpu3d) {
    command(gpu, 0x10, &[2]);
    command(gpu, 0x1A, &[0, 0x1000, 0, 0xFFFF_F000, 0, 0, 0, 0, 0x1000]);
}

#[test]
fn given_the_vector_matrix_rotated_when_the_light_and_the_normal_are_set_then_both_are_rotated() {
    // The light (511, 0, 0) becomes (0, 511, 0) and the normal (-511, 0, 0) becomes (0, -511, 0): they
    // oppose, -(N . L) = 261121, 510, the same 30 per channel.
    let got = colors(LIGHT_0, vec10(-511, 0, 0), |gpu| {
        quarter_turn(gpu);
        command(gpu, LIGHT_COLOR, &[WHITE]);
        command(gpu, LIGHT_VECTOR, &[vec10(511, 0, 0)]);
        command(gpu, DIF_AMB, &[WHITE]);
    });
    assert_eq!(got, vec![THIRTY; 3]);
}

#[test]
fn given_a_light_set_before_the_rotation_when_the_normal_is_rotated_then_the_light_keeps_its_old_direction()
 {
    // The light stays (511, 0, 0); the normal (-511, 0, 0) becomes (0, -511, 0): perpendicular.
    let got = colors(LIGHT_0, vec10(-511, 0, 0), |gpu| {
        command(gpu, LIGHT_COLOR, &[WHITE]);
        command(gpu, LIGHT_VECTOR, &[vec10(511, 0, 0)]);
        command(gpu, DIF_AMB, &[WHITE]);
        quarter_turn(gpu);
    });
    assert_eq!(got, vec![0; 3]);
}

/// NORMAL's end time with `enabled` lights in POLYGON_ATTR, written without draining.
fn normal_ends(enabled: u32) -> Option<u64> {
    let mut gpu = Gpu3d::new();
    command(&mut gpu, POLYGON_ATTR, &[enabled]);
    command(&mut gpu, BEGIN_VTXS, &[0]);
    gpu.write(GXFIFO, Width::Word, NORMAL, cx_at(0));
    gpu.write(GXFIFO, Width::Word, 0, cx_at(0));
    gpu.slot_due(0)
}

#[test]
fn given_zero_to_four_lights_when_normal_runs_then_it_takes_9_to_12_bus_cycles() {
    let got: Vec<Option<u64>> = [0, 1, 3, 7, 15].map(normal_ends).to_vec();
    assert_eq!(got, vec![Some(18), Some(20), Some(22), Some(24), Some(24)]);
}

// ------------------------------------------------------------------- texture coordinates

const S: u32 = 0x10;
const T: u32 = 0x20;

/// The (S, T) of the first vertex of a triangle after TEXCOORD `(S, T)` in `mode`, with `setup` run
/// first and `normal` sent before the vertices when given.
fn st(mode: u32, normal: Option<u32>, setup: impl FnOnce(&mut Gpu3d)) -> (i16, i16) {
    let mut gpu = Gpu3d::new();
    setup(&mut gpu);
    command(&mut gpu, TEXIMAGE_PARAM, &[mode << 30]);
    command(&mut gpu, BEGIN_VTXS, &[0]);
    command(&mut gpu, TEXCOORD, &[T << 16 | S]);
    normal.iter().for_each(|&n| command(&mut gpu, NORMAL, &[n]));
    command(&mut gpu, VTX_16, &[0, 0]);
    command(&mut gpu, VTX_16, &[0x1000, 0]);
    command(&mut gpu, VTX_16, &[0, 0x1000]);
    let vertex = gpu.geometry_ram().vertices[1];
    (vertex.s, vertex.t)
}

fn nothing(_: &mut Gpu3d) {}

#[test]
fn given_mode_0_when_texcoord_runs_then_the_vertex_takes_s_and_t_as_they_are() {
    assert_eq!(st(0, None, nothing), (0x10, 0x20));
}

#[test]
fn given_mode_1_and_the_identity_texture_matrix_when_texcoord_runs_then_s_and_t_are_unchanged() {
    assert_eq!(st(1, None, nothing), (0x10, 0x20));
}

#[test]
fn given_mode_1_and_a_scale_of_2_when_texcoord_runs_then_s_and_t_double() {
    let got = st(1, None, |gpu| {
        command(gpu, 0x10, &[3]);
        command(gpu, 0x1B, &[0x2000, 0x2000, 0x2000]);
    });
    assert_eq!(got, (0x20, 0x40));
}

#[test]
fn given_mode_1_and_a_translation_when_texcoord_runs_then_the_bottom_row_adds_in_sixteenths() {
    // m12 = 1.0 and m13 = 2.0 add 1 and 2 raw units: the constants (1/16) are a raw 1 and the
    // sum shifts right by 12.
    let got = st(1, None, |gpu| {
        command(gpu, 0x10, &[3]);
        command(gpu, 0x1C, &[0x1000, 0x2000, 0]);
    });
    assert_eq!(got, (0x11, 0x22));
}

#[test]
fn given_mode_2_when_normal_runs_then_the_normal_through_the_matrix_adds_to_s_and_t() {
    // N = (511, 0, 0): 511 x 1000h >> 17 = 15, plus S = 10h is 31; T is untouched.
    let got = st(2, Some(vec10(511, 0, 0)), nothing);
    assert_eq!(got, (31, 0x20));
}

#[test]
fn given_mode_3_when_a_vertex_runs_then_its_position_through_the_matrix_adds_to_s_and_t() {
    // The second vertex is at x = 1000h: 1000h x 1000h >> 20 = 16, plus S = 10h is 20h.
    assert_eq!(st(3, None, nothing), (0x20, 0x20));
}

#[test]
fn given_lighting_state_when_saved_and_loaded_then_it_is_kept() {
    let mut original = Gpu3d::new();
    material(WHITE, WHITE, WHITE | 1 << 15)(&mut original);
    with_table(&mut original);
    let bytes = postcard::to_allocvec(&original).unwrap();
    let restored: Gpu3d = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(restored, original);
}
