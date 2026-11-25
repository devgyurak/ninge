// SPDX-License-Identifier: Apache-2.0
//! Z and W depth, the equal test and the clear depth (`docs/hw/rasterizer.md`, "Depth").

use super::super::setup::expand_clear_depth;
use super::*;

const W_BUFFER: u8 = 1 << 1;
const DEPTH_EQUAL: u32 = 1 << 14;

/// A quad over pixels 10-30 on lines 10-30 with every vertex at W = `k` x 3.0 and depth `z`.
fn quad_k(z: i32, color: u16, k: i32) -> Vec<Vertex> {
    vec![
        vertex_k(10, 10, z, color, k),
        vertex_k(10, 30, z, color, k),
        vertex_k(30, 30, z, color, k),
        vertex_k(30, 10, z, color, k),
    ]
}

#[test]
fn given_clear_depths_when_expanded_then_zero_stays_zero_one_is_200h_and_7fffh_is_the_far_end() {
    assert_eq!(
        [
            expand_clear_depth(0),
            expand_clear_depth(1),
            expand_clear_depth(0x7FFF)
        ],
        [0, 0x200, 0x00FF_FFFF]
    );
}

#[test]
fn given_equal_z_and_unequal_w_when_z_buffered_then_the_first_polygon_stays() {
    // Both have z / w = 0 (depth half way); the second one has the larger w.
    let ram = scene(&[
        (opaque(1), quad_k(0, RED, 1)),
        (opaque(2), quad_k(0, GREEN, 2)),
    ]);
    assert_eq!(at(&draw_swap(&ram, 0), 20, 20).color, RED);
}

#[test]
fn given_a_larger_w_drawn_first_when_w_buffered_then_the_smaller_w_covers_it() {
    let ram = scene(&[
        (opaque(1), quad_k(0, GREEN, 2)),
        (opaque(2), quad_k(0, RED, 1)),
    ]);
    assert_eq!(at(&draw_swap(&ram, W_BUFFER), 20, 20).color, RED);
}

#[test]
fn given_a_smaller_w_drawn_first_when_w_buffered_then_the_larger_w_does_not_cover_it() {
    let ram = scene(&[
        (opaque(1), quad_k(0, RED, 1)),
        (opaque(2), quad_k(0, GREEN, 2)),
    ]);
    assert_eq!(at(&draw_swap(&ram, W_BUFFER), 20, 20).color, RED);
}

#[test]
fn given_a_polygon_that_asks_for_equal_when_it_has_the_depth_of_the_first_then_it_draws_over_it() {
    let ram = scene(&[
        (opaque(1), quad_k(0, RED, 1)),
        (opaque(2) | DEPTH_EQUAL, quad_k(0, GREEN, 1)),
    ]);
    assert_eq!(at(&draw_swap(&ram, 0), 20, 20).color, GREEN);
}

#[test]
fn given_a_polygon_that_asks_for_less_when_it_has_the_depth_of_the_first_then_it_does_not_draw() {
    let ram = scene(&[
        (opaque(1), quad_k(0, RED, 1)),
        (opaque(2), quad_k(0, GREEN, 1)),
    ]);
    assert_eq!(at(&draw_swap(&ram, 0), 20, 20).color, RED);
}

#[test]
fn given_a_polygon_that_asks_for_equal_when_it_is_nearer_than_the_first_then_it_does_not_draw() {
    let ram = scene(&[
        (opaque(1), quad_k(0, RED, 1)),
        (opaque(2) | DEPTH_EQUAL, quad_k(-5000, GREEN, 1)),
    ]);
    assert_eq!(at(&draw_swap(&ram, 0), 20, 20).color, RED);
}

#[test]
fn given_a_depth_within_200h_when_the_polygon_asks_for_equal_then_it_draws_over_the_first() {
    // At W = 32 x 3.0 one z unit is about 21 depth units, so 23 is about 490: inside 200h.
    let ram = scene(&[
        (opaque(1), quad_k(0, RED, 32)),
        (opaque(2) | DEPTH_EQUAL, quad_k(23, GREEN, 32)),
    ]);
    assert_eq!(at(&draw_swap(&ram, 0), 20, 20).color, GREEN);
}

#[test]
fn given_a_depth_beyond_200h_when_the_polygon_asks_for_equal_then_it_does_not_draw() {
    // 40 z units are about 850 depth units: outside 200h.
    let ram = scene(&[
        (opaque(1), quad_k(0, RED, 32)),
        (opaque(2) | DEPTH_EQUAL, quad_k(40, GREEN, 32)),
    ]);
    assert_eq!(at(&draw_swap(&ram, 0), 20, 20).color, RED);
}
