// SPDX-License-Identifier: Apache-2.0
//! Which pixels a polygon covers, the depth test and the rear plane (`docs/hw/rasterizer.md`,
//! "Which pixels a polygon covers", "Depth", "The rear plane").

use super::*;

const EDGE_MARKING: u16 = 1 << 5;
const NONE: u16 = 0;

#[test]
fn given_an_opaque_quad_when_drawn_then_it_keeps_its_upper_left_and_drops_its_lower_right() {
    let ram = scene(&[(opaque(1), quad(10, 10, 20, 20, 0, RED))]);
    let lines = draw(&ram, NONE, &regs());
    assert_eq!(covered(&lines), rect(10, 10, 19, 19));
}

#[test]
fn given_edge_marking_on_when_an_opaque_quad_is_drawn_then_its_lower_edge_draws_but_not_its_vertical_right_one()
 {
    let ram = scene(&[(opaque(1), quad(10, 10, 20, 20, 0, RED))]);
    let lines = draw(&ram, EDGE_MARKING, &regs());
    // A vertical right edge never draws (`rasterizer.md`), edge marking or not.
    assert_eq!(covered(&lines), rect(10, 10, 19, 20));
}

#[test]
fn given_a_flat_red_quad_when_drawn_then_its_pixels_are_opaque_red() {
    let ram = scene(&[(opaque(1), quad(10, 10, 20, 20, 0, RED))]);
    let lines = draw(&ram, NONE, &regs());
    assert_eq!(
        at(&lines, 15, 15),
        Pixel3d {
            color: RED,
            alpha: 31
        }
    );
}

#[test]
fn given_a_back_facing_quad_when_only_the_front_bit_is_set_then_nothing_draws() {
    let clockwise = vec![
        vertex(10, 10, 0, RED),
        vertex(20, 10, 0, RED),
        vertex(20, 20, 0, RED),
        vertex(10, 20, 0, RED),
    ];
    let ram = scene(&[(OPAQUE | 0x80, clockwise)]);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), vec![]);
}

#[test]
fn given_a_back_facing_quad_when_the_back_bit_is_set_then_it_draws() {
    let clockwise = vec![
        vertex(10, 10, 0, RED),
        vertex(20, 10, 0, RED),
        vertex(20, 20, 0, RED),
        vertex(10, 20, 0, RED),
    ];
    let ram = scene(&[(OPAQUE | 0x40, clockwise)]);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), rect(10, 10, 19, 19));
}

#[test]
fn given_alpha_zero_when_a_quad_is_drawn_then_only_its_outline_draws() {
    let ram = scene(&[(FRONT_BACK_ONLY, quad(10, 10, 20, 20, 0, RED))]);
    let outline: Vec<_> = rect(10, 10, 20, 20)
        .into_iter()
        .filter(|&(x, y)| x == 10 || x == 20 || y == 10 || y == 20)
        .collect();
    assert_eq!(covered(&draw(&ram, NONE, &regs())), outline);
}

/// POLYGON_ATTR with alpha 0 (a wire frame) and both faces shown.
const FRONT_BACK_ONLY: u32 = 0xC0;

#[test]
fn given_a_line_segment_when_both_face_bits_are_clear_then_it_still_draws() {
    let segment = vec![
        vertex(10, 10, 0, RED),
        vertex(20, 10, 0, RED),
        vertex(20, 10, 0, RED),
    ];
    let ram = scene(&[(OPAQUE, segment)]);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), rect(10, 10, 20, 10));
}

#[test]
fn given_a_polygon_that_rounds_to_one_dot_when_drawn_then_one_pixel_draws() {
    let tiny = vec![
        vertex(30, 40, 0, GREEN),
        vertex(30, 40, 0, GREEN),
        vertex(30, 40, 0, GREEN),
    ];
    let ram = scene(&[(opaque(1), tiny)]);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), vec![(30, 40)]);
}

#[test]
fn given_a_polygon_of_zero_width_when_drawn_then_it_is_one_pixel_wide() {
    let thin = vec![
        vertex(30, 10, 0, RED),
        vertex(30, 20, 0, RED),
        vertex(30, 30, 0, RED),
        vertex(30, 40, 0, RED),
    ];
    let ram = scene(&[(opaque(1), thin)]);
    // Lines 10 to 39, as for a polygon that keeps its upper edge and drops its lower one.
    assert_eq!(covered(&draw(&ram, NONE, &regs())), rect(30, 10, 30, 39));
}

#[test]
fn given_a_polygon_of_zero_height_when_drawn_then_it_is_one_line_tall() {
    let flat = vec![
        vertex(10, 50, 0, RED),
        vertex(10, 50, 0, RED),
        vertex(20, 50, 0, RED),
        vertex(20, 50, 0, RED),
    ];
    let ram = scene(&[(opaque(1), flat)]);
    // One line tall, and like any opaque polygon it drops its right edge.
    assert_eq!(covered(&draw(&ram, NONE, &regs())), rect(10, 50, 19, 50));
}

#[test]
fn given_a_polygon_covering_the_screen_when_drawn_then_every_pixel_is_covered() {
    let ram = scene(&[(opaque(1), quad(0, 0, 256, 192, 0, RED))]);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), rect(0, 0, 255, 191));
}

#[test]
fn given_a_polygon_beyond_the_screen_when_drawn_then_only_the_visible_part_draws() {
    let ram = scene(&[(opaque(1), quad(-40, -40, 8, 8, 0, RED))]);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), rect(0, 0, 7, 7));
}

#[test]
fn given_an_empty_scene_when_drawn_then_every_pixel_is_the_clear_colour() {
    let regs = RenderRegs {
        clear_color: 0x001F_7C1F | 1 << 15,
        ..RenderRegs::default()
    };
    let lines = draw(&Ram::default(), NONE, &regs);
    let wrong: Vec<_> = lines
        .iter()
        .flatten()
        .filter(|p| {
            **p != Pixel3d {
                color: 0x7C1F,
                alpha: 31,
            }
        })
        .collect();
    assert_eq!(wrong, Vec::<&Pixel3d>::new());
}

#[test]
fn given_a_clear_alpha_of_zero_when_a_scene_is_empty_then_the_rear_plane_is_transparent() {
    let lines = draw(&Ram::default(), NONE, &regs());
    assert_eq!(at(&lines, 128, 96).alpha, 0);
}

#[test]
fn given_two_overlapping_quads_when_the_far_one_comes_second_then_the_near_one_stays() {
    let ram = scene(&[
        (opaque(1), quad(10, 10, 30, 30, -1000, RED)),
        (opaque(2), quad(10, 10, 30, 30, 1000, GREEN)),
    ]);
    assert_eq!(at(&draw(&ram, NONE, &regs()), 20, 20).color, RED);
}

#[test]
fn given_two_overlapping_quads_when_the_far_one_comes_first_then_the_near_one_covers_it() {
    let ram = scene(&[
        (opaque(2), quad(10, 10, 30, 30, 1000, GREEN)),
        (opaque(1), quad(10, 10, 30, 30, -1000, RED)),
    ]);
    assert_eq!(at(&draw(&ram, NONE, &regs()), 20, 20).color, RED);
}

#[test]
fn given_two_quads_at_one_depth_when_the_second_is_drawn_then_the_first_stays() {
    let ram = scene(&[
        (opaque(1), quad(10, 10, 30, 30, 0, RED)),
        (opaque(2), quad(10, 10, 30, 30, 0, GREEN)),
    ]);
    assert_eq!(at(&draw(&ram, NONE, &regs()), 20, 20).color, RED);
}

#[test]
fn given_a_quad_behind_the_rear_plane_depth_when_drawn_then_it_does_not_draw() {
    let regs = RenderRegs {
        clear_depth: 0,
        ..RenderRegs::default()
    };
    let ram = scene(&[(opaque(1), quad(10, 10, 30, 30, 0, RED))]);
    assert_eq!(covered(&draw(&ram, NONE, &regs)), vec![]);
}

#[test]
fn given_a_polygon_whose_vertices_are_missing_when_drawn_then_nothing_draws_and_nothing_panics() {
    let mut ram = scene(&[(opaque(1), quad(10, 10, 30, 30, 0, RED))]);
    ram.vertices.truncate(2);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), vec![]);
}

#[test]
fn given_a_scene_when_drawn_through_the_executor_then_it_equals_a_plain_line_loop() {
    let ram = scene(&[
        (opaque(1), quad(10, 10, 100, 90, 0, RED)),
        (opaque(2), quad(50, 30, 200, 150, -500, GREEN)),
    ]);
    let plain = draw_plain(&ram, NONE, &regs());
    assert_eq!(draw(&ram, NONE, &regs()), plain);
}

#[test]
fn given_a_quad_with_one_repeated_vertex_when_the_face_bits_are_clear_then_it_is_not_a_line_segment()
 {
    let repeated = vec![
        vertex(10, 10, 0, RED),
        vertex(10, 30, 0, RED),
        vertex(30, 30, 0, RED),
        vertex(10, 10, 0, RED),
    ];
    let ram = scene(&[(OPAQUE, repeated)]);
    assert_eq!(covered(&draw(&ram, NONE, &regs())), vec![]);
}

#[test]
fn given_vertices_at_the_extremes_of_the_coordinate_range_when_drawn_then_nothing_panics() {
    let mut ram = scene(&[(opaque(1), quad(10, 10, 30, 30, 0, RED))]);
    ram.vertices[0].x = i32::MAX;
    ram.vertices[1].y = i32::MIN;
    ram.vertices[2].x = i32::MIN;
    ram.vertices[2].z = i32::MAX;
    assert_eq!(draw(&ram, NONE, &regs()).len(), crate::SCREEN_HEIGHT);
}
