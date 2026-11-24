// SPDX-License-Identifier: Apache-2.0
//! Vertices, polygon assembly, clipping and Polygon / Vertex RAM as a device. Expected values come
//! from `docs/hw/geometry-engine.md` ("Vertices", "Primitives and strips", "Clipping", "Polygon RAM,
//! Vertex RAM and the double buffer", "SWAP_BUFFERS"), worked out by hand. With the matrices at
//! their reset (the identity) a vertex `(x, y, z)` becomes the clip vertex `(x, y, z, 1000h)`,
//! inside while `-1000h <= x, y, z <= 1000h`.

use super::*;

const VTX_16: u32 = 0x23;
const VTX_10: u32 = 0x24;
const VTX_XY: u32 = 0x25;
const VTX_XZ: u32 = 0x26;
const VTX_YZ: u32 = 0x27;
const VTX_DIFF: u32 = 0x28;
const BEGIN_VTXS: u32 = 0x40;
const END_VTXS: u32 = 0x41;
const POLYGON_ATTR: u32 = 0x29;
const TEXIMAGE_PARAM: u32 = 0x2A;
const PLTT_BASE: u32 = 0x2B;
const COLOR_CMD: u32 = 0x20;
const SWAP: u32 = 0x50;
const RAM_COUNT: u32 = GEOMETRY_OFFSET + 0x204;
const DISP3DCNT: u32 = 0;

const TRIANGLES: u32 = 0;
const QUADS: u32 = 1;
const TRIANGLE_STRIP: u32 = 2;
const QUAD_STRIP: u32 = 3;
/// The reference point of the examples: `1.0`.
const W: i32 = 0x1000;

fn send_command(gpu: &mut Gpu3d, id: u32, params: &[u32]) {
    send(gpu, &[id]);
    send(gpu, params);
}

fn begin(gpu: &mut Gpu3d, primitive: u32) {
    send_command(gpu, BEGIN_VTXS, &[primitive]);
}

/// VTX_16 for `(x, y, z)` as 1.3.12 values.
fn vtx(gpu: &mut Gpu3d, x: i16, y: i16, z: i16) {
    let xy = u32::from(y as u16) << 16 | u32::from(x as u16);
    send_command(gpu, VTX_16, &[xy, u32::from(z as u16)]);
}

/// VTX_16 for each of `points`.
fn vtxs(gpu: &mut Gpu3d, points: &[(i16, i16, i16)]) {
    points.iter().for_each(|&(x, y, z)| vtx(gpu, x, y, z));
}

/// `count` MTX_PUSH words written at time 0 without draining.
fn pushes_raw(gpu: &mut Gpu3d, count: u32) {
    (0..count).for_each(|_| {
        gpu.write(GXFIFO, Width::Word, MTX_PUSH.into(), cx_at(0));
    });
}

/// A clip-space vertex with the default colour and texture coordinates.
fn clip(x: i32, y: i32, z: i32) -> Vertex {
    Vertex {
        x,
        y,
        z,
        w: W,
        color: 0x7FFF,
        s: 0,
        t: 0,
    }
}

/// The polygons of the set the engine is filling, as vertex index lists.
fn indices(gpu: &Gpu3d) -> Vec<Vec<u16>> {
    gpu.geometry_ram()
        .polygons
        .iter()
        .map(|polygon| polygon.vertices[..usize::from(polygon.count)].to_vec())
        .collect()
}

fn vertices(gpu: &Gpu3d) -> Vec<Vertex> {
    gpu.geometry_ram().vertices.clone()
}

/// A device with one list of `primitive` made of `points`.
fn list(primitive: u32, points: &[(i16, i16, i16)]) -> Gpu3d {
    let mut gpu = Gpu3d::new();
    begin(&mut gpu, primitive);
    points.iter().for_each(|&(x, y, z)| vtx(&mut gpu, x, y, z));
    gpu
}

#[test]
fn given_a_triangle_when_its_third_vertex_arrives_then_one_polygon_and_three_vertices_are_stored() {
    let gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0), (0, 0x800, 0)]);
    assert_eq!(
        (indices(&gpu), vertices(&gpu)),
        (
            vec![vec![0, 1, 2]],
            vec![clip(0, 0, 0), clip(0x800, 0, 0), clip(0, 0x800, 0)]
        )
    );
}

#[test]
fn given_a_triangle_when_ram_count_is_read_then_it_counts_one_polygon_and_three_vertices() {
    let gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0), (0, 0x800, 0)]);
    assert_eq!(read(&gpu, RAM_COUNT, Width::Word), 1 | 3 << 16);
}

#[test]
fn given_two_vertices_of_a_triangle_when_inspected_then_no_polygon_is_stored_yet() {
    let gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0)]);
    assert_eq!(gpu.geometry_ram().polygons.len(), 0);
}

#[test]
fn given_six_vertices_of_separate_triangles_when_sent_then_two_polygons_use_six_vertices() {
    let points = [
        (0, 0, 0),
        (1, 0, 0),
        (0, 1, 0),
        (2, 0, 0),
        (3, 0, 0),
        (2, 1, 0),
    ];
    let gpu = list(TRIANGLES, &points);
    assert_eq!(indices(&gpu), vec![vec![0, 1, 2], vec![3, 4, 5]]);
}

#[test]
fn given_a_quad_when_its_fourth_vertex_arrives_then_one_four_vertex_polygon_is_stored() {
    let gpu = list(QUADS, &[(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]);
    assert_eq!(indices(&gpu), vec![vec![0, 1, 2, 3]]);
}

#[test]
fn given_a_triangle_strip_of_five_vertices_when_sent_then_three_triangles_share_their_vertices() {
    // Triangle k uses vertices k, k+1, k+2; odd ones swap the first two to keep the facing.
    let points = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 2, 0)];
    let gpu = list(TRIANGLE_STRIP, &points);
    assert_eq!(
        (indices(&gpu), gpu.geometry_ram().vertices.len()),
        (vec![vec![0, 1, 2], vec![2, 1, 3], vec![2, 3, 4]], 5)
    );
}

#[test]
fn given_a_quad_strip_of_six_vertices_when_sent_then_two_quads_share_two_vertices() {
    let points = [
        (0, 0, 0),
        (0, 1, 0),
        (1, 0, 0),
        (1, 1, 0),
        (2, 0, 0),
        (2, 1, 0),
    ];
    let gpu = list(QUAD_STRIP, &points);
    assert_eq!(
        (indices(&gpu), gpu.geometry_ram().vertices.len()),
        (vec![vec![0, 1, 3, 2], vec![2, 3, 5, 4]], 6)
    );
}

#[test]
fn given_a_quad_strip_with_a_dangling_vertex_when_sent_then_only_the_whole_quads_are_stored() {
    let points = [(0, 0, 0), (0, 1, 0), (1, 0, 0), (1, 1, 0), (2, 0, 0)];
    assert_eq!(indices(&list(QUAD_STRIP, &points)), vec![vec![0, 1, 3, 2]]);
}

#[test]
fn given_end_vtxs_in_the_middle_of_a_list_when_sent_then_the_list_goes_on() {
    let mut gpu = list(TRIANGLES, &[(0, 0, 0), (1, 0, 0), (0, 1, 0)]);
    send(&mut gpu, &[END_VTXS]);
    vtx(&mut gpu, 2, 0, 0);
    vtx(&mut gpu, 3, 0, 0);
    vtx(&mut gpu, 2, 1, 0);
    assert_eq!(indices(&gpu), vec![vec![0, 1, 2], vec![3, 4, 5]]);
}

#[test]
fn given_vertices_before_any_begin_when_sent_then_no_polygon_is_built() {
    let mut gpu = Gpu3d::new();
    vtxs(&mut gpu, &[(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
    assert_eq!(gpu.geometry_ram().polygons.len(), 0);
}

// ----------------------------------------------------------------------- vertex commands

#[test]
fn given_each_vertex_command_when_it_runs_then_the_position_changes_as_the_note_says() {
    let mut gpu = Gpu3d::new();
    begin(&mut gpu, QUADS);
    // VTX_16 (1, 2, 3); VTX_XY keeps z; VTX_XZ keeps y; VTX_YZ keeps x; VTX_DIFF adds (+1, -1, +2).
    vtx(&mut gpu, 1, 2, 3);
    send_command(&mut gpu, VTX_XY, &[20 << 16 | 10]);
    send_command(&mut gpu, VTX_XZ, &[40 << 16 | 30]);
    send_command(&mut gpu, VTX_YZ, &[60 << 16 | 50]);
    let got: Vec<(i32, i32, i32)> = vertices(&gpu).iter().map(|v| (v.x, v.y, v.z)).collect();
    assert_eq!(
        got,
        vec![(1, 2, 3), (10, 20, 3), (30, 20, 40), (30, 50, 60)]
    );
}

#[test]
fn given_a_vtx_diff_when_it_runs_then_each_field_is_added_to_the_previous_coordinate() {
    let mut gpu = Gpu3d::new();
    begin(&mut gpu, TRIANGLES);
    vtx(&mut gpu, 100, 100, 100);
    // +1, -1 (3FFh), +2 in the three 10-bit fields.
    send_command(&mut gpu, VTX_DIFF, &[2 << 20 | 0x3FF << 10 | 1]);
    vtx(&mut gpu, 0, 0, 0);
    let got = vertices(&gpu);
    assert_eq!((got[1].x, got[1].y, got[1].z), (101, 99, 102));
}

#[test]
fn given_a_vtx_10_in_a_triangle_when_stored_then_the_coordinates_are_the_fields_times_64() {
    let mut gpu = Gpu3d::new();
    begin(&mut gpu, TRIANGLES);
    send_command(&mut gpu, VTX_10, &[3 << 20 | 0x3FF << 10 | 1]);
    vtx(&mut gpu, 0, 0, 0);
    vtx(&mut gpu, 0, 0, 0);
    let first = vertices(&gpu)[0];
    assert_eq!((first.x, first.y, first.z), (64, -64, 192));
}

#[test]
fn given_a_translation_in_the_position_matrix_when_a_vertex_is_sent_then_it_is_transformed() {
    let mut gpu = in_mode(1);
    send_command(&mut gpu, 0x1C, &[0x100, 0x200, 0x300]);
    begin(&mut gpu, TRIANGLES);
    vtx(&mut gpu, 1, 2, 3);
    vtx(&mut gpu, 1, 2, 3);
    vtx(&mut gpu, 1, 2, 3);
    assert_eq!(vertices(&gpu)[0], clip(0x101, 0x202, 0x303));
}

#[test]
fn given_a_color_when_a_polygon_is_stored_then_its_vertices_carry_the_colour_set_before_each() {
    let mut gpu = Gpu3d::new();
    begin(&mut gpu, TRIANGLES);
    send_command(&mut gpu, COLOR_CMD, &[0x001F]);
    vtx(&mut gpu, 0, 0, 0);
    send_command(&mut gpu, COLOR_CMD, &[0x03E0]);
    vtx(&mut gpu, 1, 0, 0);
    send_command(&mut gpu, COLOR_CMD, &[0x7C00]);
    vtx(&mut gpu, 0, 1, 0);
    let colors: Vec<u16> = vertices(&gpu).iter().map(|v| v.color).collect();
    assert_eq!(colors, vec![0x001F, 0x03E0, 0x7C00]);
}

#[test]
fn given_attributes_when_a_polygon_is_stored_then_they_are_those_of_the_begin_and_the_current_texture_state()
 {
    let mut gpu = Gpu3d::new();
    send_command(&mut gpu, POLYGON_ATTR, &[0x1F00_0000 | 5]);
    begin(&mut gpu, TRIANGLES);
    // A POLYGON_ATTR after BEGIN_VTXS waits for the next one.
    send_command(&mut gpu, POLYGON_ATTR, &[0xFFFF]);
    send_command(&mut gpu, TEXIMAGE_PARAM, &[0xAAAA]);
    send_command(&mut gpu, PLTT_BASE, &[0xBBBB]);
    vtxs(&mut gpu, &[(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
    let polygon = gpu.geometry_ram().polygons[0];
    assert_eq!(
        (polygon.attr, polygon.texture, polygon.palette),
        (0x1F00_0005, 0xAAAA, 0xBBBB)
    );
}

// ------------------------------------------------------------------------------- clipping

/// A triangle whose second corner is beyond the right plane (`x = 2.0`).
fn cut_triangle() -> Gpu3d {
    list(TRIANGLES, &[(0, 0, 0), (0x2000, 0, 0), (0, 0x800, 0)])
}

#[test]
fn given_a_corner_beyond_the_right_plane_when_clipped_then_the_polygon_has_four_corners_on_the_plane()
 {
    // Worked through the planes in order: left keeps all three, the right plane replaces the outside
    // corner B by the two points where edges B-C and A-B cross x = w, and the four planes that
    // follow keep every corner.
    assert_eq!(
        vertices(&cut_triangle()),
        vec![
            clip(0x1000, 0x400, 0),
            clip(0, 0x800, 0),
            clip(0, 0, 0),
            clip(0x1000, 0, 0)
        ]
    );
}

/// A vertex at `(x, y, 0)` with the colour (RGB555) `color` set before it.
fn colored(gpu: &mut Gpu3d, color: u32, x: i16, y: i16) {
    send_command(gpu, COLOR_CMD, &[color]);
    vtx(gpu, x, y, 0);
}

/// The cut triangle again with a colour on each corner: A red, B green beyond the right plane, C
/// blue.
fn colored_cut_triangle() -> Gpu3d {
    let mut gpu = Gpu3d::new();
    begin(&mut gpu, TRIANGLES);
    colored(&mut gpu, 0x001F, 0, 0);
    colored(&mut gpu, 0x03E0, 0x2000, 0);
    colored(&mut gpu, 0x7C00, 0, 0x800);
    gpu
}

#[test]
fn given_a_cut_triangle_with_a_colour_on_each_corner_when_clipped_then_the_new_corners_interpolate_it()
 {
    // The corners come out as [I1, C, A, I2] (see above), both new ones halfway along their edge:
    // - I1 on B-C: each 5-bit channel is `from + (to - from) x da / (da - db)` truncated toward zero
    //   with da / (da - db) = 1/2: green 31 + (-31 x 1/2 = -15) = 16, blue 0 + 15 = 15, red 0, so
    //   `16 << 5 | 15 << 10 = 3E00h`.
    // - I2 on A-B: red 31 - 15 = 16, green 0 + 15: `16 | 15 << 5 = 1F0h`.
    let gpu = colored_cut_triangle();
    let got: Vec<u16> = vertices(&gpu).iter().map(|v| v.color).collect();
    assert_eq!(got, vec![0x3E00, 0x7C00, 0x001F, 0x01F0]);
}

#[test]
fn given_an_edge_with_an_inexact_negative_step_when_clipped_then_the_coordinate_truncates_toward_zero()
 {
    // A (-2.0, 0, 0) is beyond the left plane and B (0, -3, 0) is inside: the edge crosses x = -w at
    // half the way, and y = -3 x 1/2 = -1.5 truncates toward zero to -1 (not -2).
    let gpu = list(TRIANGLES, &[(-0x2000, 0, 0), (0, -3, 0), (0, 0x100, 0)]);
    assert!(vertices(&gpu).contains(&clip(-0x1000, -1, 0)));
}

#[test]
fn given_a_clipped_polygon_when_stored_then_it_has_four_vertices_in_ram_and_one_polygon() {
    let gpu = cut_triangle();
    assert_eq!(
        (indices(&gpu), read(&gpu, RAM_COUNT, Width::Word)),
        (vec![vec![0, 1, 2, 3]], 1 | 4 << 16)
    );
}

#[test]
fn given_a_polygon_wholly_outside_when_sent_then_nothing_is_stored() {
    let gpu = list(
        TRIANGLES,
        &[(0x3000, 0, 0), (0x4000, 0, 0), (0x3000, 0x100, 0)],
    );
    assert_eq!(read(&gpu, RAM_COUNT, Width::Word), 0);
}

#[test]
fn given_a_polygon_exactly_on_the_planes_when_sent_then_it_is_not_clipped() {
    let gpu = list(
        TRIANGLES,
        &[(-0x1000, -0x1000, 0), (0x1000, -0x1000, 0), (0, 0x1000, 0)],
    );
    assert_eq!(indices(&gpu), vec![vec![0, 1, 2]]);
}

#[test]
fn given_a_corner_just_beyond_a_plane_when_sent_then_the_polygon_is_clipped() {
    let gpu = list(TRIANGLES, &[(0, 0, 0), (0x1001, 0, 0), (0, 0x100, 0)]);
    assert_eq!(gpu.geometry_ram().vertices.len(), 4);
}

/// A triangle with one corner past the far plane, `attr` as POLYGON_ATTR.
fn far_triangle(attr: u32) -> Gpu3d {
    let mut gpu = Gpu3d::new();
    send_command(&mut gpu, POLYGON_ATTR, &[attr]);
    begin(&mut gpu, TRIANGLES);
    vtx(&mut gpu, 0, 0, 0);
    vtx(&mut gpu, 0, 0x400, 0);
    vtx(&mut gpu, 0, 0, 0x2000);
    gpu
}

#[test]
fn given_a_polygon_crossing_the_far_plane_when_attr_bit_12_is_clear_then_it_is_hidden() {
    assert_eq!(far_triangle(0).geometry_ram().polygons.len(), 0);
}

#[test]
fn given_a_polygon_crossing_the_far_plane_when_attr_bit_12_is_set_then_it_is_stored_clipped() {
    let gpu = far_triangle(1 << 12);
    assert_eq!(
        (
            gpu.geometry_ram().polygons.len(),
            gpu.geometry_ram().vertices.len()
        ),
        (1, 4)
    );
}

#[test]
fn given_a_triangle_strip_with_a_clipped_triangle_when_sent_then_the_clipped_one_stands_alone() {
    // Triangles (v0, v1, v2) and (v2, v1, v3): the second has v3 beyond the right plane, so it is
    // stored with four fresh vertices: 3 + 4 in all, and two polygons.
    let points = [(0, 0, 0), (0x100, 0, 0), (0, 0x100, 0), (0x2000, 0x100, 0)];
    let gpu = list(TRIANGLE_STRIP, &points);
    assert_eq!(
        (
            gpu.geometry_ram().polygons.len(),
            gpu.geometry_ram().vertices.len()
        ),
        (2, 7)
    );
}

// -------------------------------------------------------------------------- RAM limits

/// `count` separate triangles in one list.
fn many(count: u32, clipped: bool) -> Gpu3d {
    let mut gpu = Gpu3d::new();
    begin(&mut gpu, TRIANGLES);
    (0..count).for_each(|_| {
        let corner = if clipped { 0x2000 } else { 0x800 };
        vtx(&mut gpu, 0, 0, 0);
        vtx(&mut gpu, corner, 0, 0);
        vtx(&mut gpu, 0, 0x800, 0);
    });
    gpu
}

#[test]
fn given_2048_triangles_when_sent_then_all_are_stored_and_no_overflow_is_raised() {
    let gpu = many(2048, false);
    assert_eq!(
        (gpu.geometry_ram().polygons.len(), gpu.ram_overflow()),
        (2048, false)
    );
}

#[test]
fn given_2049_triangles_when_sent_then_the_last_is_dropped_and_the_overflow_flag_is_raised() {
    let gpu = many(2049, false);
    assert_eq!(
        (
            gpu.geometry_ram().polygons.len(),
            gpu.geometry_ram().vertices.len(),
            gpu.ram_overflow()
        ),
        (2048, 6144, true)
    );
}

#[test]
fn given_1537_clipped_triangles_when_sent_then_vertex_ram_overflows_at_the_last() {
    // Each clipped triangle takes four vertices: 1536 x 4 = 6144 fill Vertex RAM exactly.
    let gpu = many(1537, true);
    assert_eq!(
        (
            gpu.geometry_ram().polygons.len(),
            gpu.geometry_ram().vertices.len(),
            gpu.ram_overflow()
        ),
        (1536, 6144, true)
    );
}

#[test]
fn given_an_overflow_when_disp3dcnt_is_read_then_bit_13_is_set_and_a_write_of_one_clears_it() {
    let mut gpu = many(2049, false);
    let before = read(&gpu, DISP3DCNT, Width::Half);
    write(&mut gpu, DISP3DCNT, Width::Half, 1 << 13);
    let after = read(&gpu, DISP3DCNT, Width::Half);
    assert_eq!((before, after), (1 << 13, 0));
}

#[test]
fn given_an_overflow_when_a_zero_is_written_to_bit_13_then_the_flag_stays() {
    let mut gpu = many(2049, false);
    write(&mut gpu, DISP3DCNT, Width::Half, 0);
    assert_eq!(read(&gpu, DISP3DCNT, Width::Half), 1 << 13);
}

// --------------------------------------------------------------------------- the swap

/// A list of one triangle followed by SWAP_BUFFERS with `bits` as its parameter.
fn swapped(bits: u32) -> Gpu3d {
    let mut gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0), (0, 0x800, 0)]);
    send_command(&mut gpu, SWAP, &[bits]);
    gpu
}

#[test]
fn given_a_swap_when_it_ends_then_the_polygons_are_in_the_render_set_and_the_filling_set_is_empty()
{
    let gpu = swapped(0);
    assert_eq!(
        (
            gpu.render_ram().polygons.len(),
            gpu.render_ram().vertices.len(),
            gpu.geometry_ram().polygons.len(),
            read(&gpu, RAM_COUNT, Width::Word)
        ),
        (1, 3, 0, 0)
    );
}

#[test]
fn given_two_swaps_when_the_second_ends_then_the_render_set_is_the_one_after_the_first() {
    let mut gpu = swapped(0);
    begin(&mut gpu, QUADS);
    vtxs(&mut gpu, &[(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]);
    send_command(&mut gpu, SWAP, &[0]);
    assert_eq!(
        (
            gpu.render_ram().polygons[0].count,
            gpu.render_ram().polygons.len()
        ),
        (4, 1)
    );
}

#[test]
fn given_a_swap_when_it_is_executing_then_the_polygons_are_still_in_the_filling_set() {
    let mut gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0), (0, 0x800, 0)]);
    gpu.write(GXFIFO, Width::Word, SWAP, cx_at(0));
    gpu.write(GXFIFO, Width::Word, 0, cx_at(0));
    assert_eq!(gpu.geometry_ram().polygons.len(), 1);
}

#[test]
fn given_a_swap_with_the_parameter_bits_set_when_it_ends_then_the_renderer_flags_are_kept() {
    // Not visible through a register: the debugger reads them with the render set.
    let gpu = swapped(3);
    assert_eq!(gpu.swap_bits(), 3);
}

#[test]
fn given_a_swap_inside_an_unfinished_triangle_when_it_runs_then_the_hardware_locks_up() {
    let mut gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0)]);
    send_command(&mut gpu, SWAP, &[0]);
    assert_eq!(
        (gpu.locked_up(), gpu.slot_due(0), gxstat(&gpu) & (1 << 27)),
        (true, None, 1 << 27)
    );
}

#[test]
fn given_a_locked_up_engine_when_more_commands_arrive_then_nothing_runs_and_the_writes_do_not_stall()
 {
    let mut gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0)]);
    send_command(&mut gpu, SWAP, &[0]);
    let effect = gpu.write(GXFIFO, Width::Word, MTX_PUSH.into(), cx_at(0));
    assert_eq!((effect.stall, gpu.geometry_ram().polygons.len()), (0, 0));
}

#[test]
fn given_a_locked_up_engine_when_the_fifo_is_filled_then_a_write_to_the_full_fifo_is_dropped() {
    let mut gpu = list(TRIANGLES, &[(0, 0, 0), (0x800, 0, 0)]);
    send_command(&mut gpu, SWAP, &[0]);
    pushes_raw(&mut gpu, 300);
    assert_eq!(gpu.fifo_len(), 256);
}

#[test]
fn given_a_locked_up_state_when_saved_and_loaded_then_it_is_still_locked_up() {
    let mut original = list(TRIANGLES, &[(0, 0, 0)]);
    send_command(&mut original, SWAP, &[0]);
    let bytes = postcard::to_allocvec(&original).unwrap();
    let restored: Gpu3d = postcard::from_bytes(&bytes).unwrap();
    assert_eq!((restored == original, restored.locked_up()), (true, true));
}

#[test]
fn given_polygons_in_both_sets_when_saved_and_loaded_then_they_are_kept_and_well_formed() {
    let mut original = swapped(1);
    begin(&mut original, TRIANGLES);
    vtxs(&mut original, &[(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
    let bytes = postcard::to_allocvec(&original).unwrap();
    let restored: Gpu3d = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(
        (
            restored == original,
            restored.well_formed(0),
            restored.render_ram().polygons.len()
        ),
        (true, true, 1)
    );
}

// ----------------------------------------------------------------- room for a full set

#[test]
fn given_a_cloned_gpu_when_its_ram_is_inspected_then_it_still_has_room_for_a_full_set() {
    let copy = swapped(0).clone();
    assert_eq!(
        (
            copy.geometry_ram().vertices.capacity() >= VERTEX_RAM,
            copy.render_ram().polygons.capacity() >= POLYGON_RAM
        ),
        (true, true)
    );
}
