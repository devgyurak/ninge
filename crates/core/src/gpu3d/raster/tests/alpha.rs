// SPDX-License-Identifier: Apache-2.0
//! Translucent polygons: blending, the alpha test, the order, the depth update and the same-ID rule
//! (`docs/hw/rasterizer.md`, "Translucency, draw order and the rear plane").

use super::*;

const ALPHA_TEST: u16 = 1 << 2;
const BLEND: u16 = 1 << 3;
const MANUAL_SORT: u8 = 1;
const UPDATE_DEPTH: u32 = 1 << 11;

/// POLYGON_ATTR of a polygon with `alpha`, ID `id`, both faces.
fn translucent(alpha: u32, id: u32) -> u32 {
    alpha << 16 | FRONT_BACK | id << 24
}
const FRONT_BACK: u32 = 0xC0;

fn pixel_of(lines: &[Line3d]) -> Pixel3d {
    at(lines, 20, 20)
}

fn with_ref(reference: u8) -> RenderRegs {
    RenderRegs {
        alpha_test_ref: reference,
        ..regs()
    }
}

#[test]
fn given_a_translucent_polygon_over_an_opaque_one_when_blending_is_on_then_the_colours_mix() {
    // Alpha 15 over opaque red: each channel (poly x 16 + buffer x 16) / 32 in 6 bits, then 5.
    let ram = scene(&[
        (opaque(1), quad(10, 10, 30, 30, 0, RED)),
        (translucent(15, 2), quad(10, 10, 30, 30, -100, GREEN)),
    ]);
    assert_eq!(
        pixel_of(&draw(&ram, BLEND, &regs())),
        Pixel3d {
            color: 15 | 15 << 5,
            alpha: 31
        }
    );
}

#[test]
fn given_blending_off_when_a_translucent_polygon_draws_over_an_opaque_one_then_it_overwrites() {
    let ram = scene(&[
        (opaque(1), quad(10, 10, 30, 30, 0, RED)),
        (translucent(15, 2), quad(10, 10, 30, 30, -100, GREEN)),
    ]);
    assert_eq!(
        pixel_of(&draw(&ram, 0, &regs())),
        Pixel3d {
            color: GREEN,
            alpha: 15
        }
    );
}

#[test]
fn given_a_rear_plane_of_alpha_zero_when_a_translucent_polygon_draws_then_it_does_not_blend() {
    let ram = scene(&[(translucent(15, 2), quad(10, 10, 30, 30, 0, GREEN))]);
    assert_eq!(
        pixel_of(&draw(&ram, BLEND, &regs())),
        Pixel3d {
            color: GREEN,
            alpha: 15
        }
    );
}

#[test]
fn given_a_translucent_polygon_listed_first_when_an_opaque_one_follows_then_the_opaque_draws_first()
{
    let ram = scene(&[
        (translucent(15, 2), quad(10, 10, 30, 30, -100, GREEN)),
        (opaque(1), quad(10, 10, 30, 30, 0, RED)),
    ]);
    assert_eq!(
        pixel_of(&draw(&ram, BLEND, &regs())),
        Pixel3d {
            color: 15 | 15 << 5,
            alpha: 31
        }
    );
}

#[test]
fn given_two_translucent_polygons_of_one_id_when_they_overlap_then_the_second_does_not_blend() {
    let ram = scene(&[
        (translucent(15, 2), quad(10, 10, 30, 30, 0, RED)),
        (translucent(15, 2), quad(10, 10, 30, 30, 0, GREEN)),
    ]);
    assert_eq!(
        pixel_of(&draw(&ram, BLEND, &regs())),
        Pixel3d {
            color: RED,
            alpha: 15
        }
    );
}

#[test]
fn given_two_translucent_polygons_of_different_ids_when_they_overlap_then_the_second_blends() {
    let ram = scene(&[
        (translucent(15, 2), quad(10, 10, 30, 30, 0, RED)),
        (translucent(15, 3), quad(10, 10, 30, 30, 0, GREEN)),
    ]);
    // Red over nothing is red (alpha 15, 6-bit 63); green blends in: R (0 x 16 + 63 x 16) / 32 = 31.
    assert_eq!(
        pixel_of(&draw(&ram, BLEND, &regs())),
        Pixel3d {
            color: 15 | 15 << 5,
            alpha: 15
        }
    );
}

#[test]
fn given_a_translucent_polygon_that_updates_depth_when_a_farther_one_follows_then_the_farther_does_not_draw()
 {
    let ram = scene(&[
        (
            translucent(15, 2) | UPDATE_DEPTH,
            quad(10, 10, 30, 30, -1000, RED),
        ),
        (translucent(15, 3), quad(10, 10, 30, 30, 1000, GREEN)),
    ]);
    assert_eq!(pixel_of(&draw(&ram, BLEND, &regs())).color, RED);
}

#[test]
fn given_a_translucent_polygon_that_keeps_the_old_depth_when_a_farther_one_follows_then_it_draws() {
    let ram = scene(&[
        (translucent(15, 2), quad(10, 10, 30, 30, -1000, RED)),
        (translucent(15, 3), quad(10, 10, 30, 30, 1000, GREEN)),
    ]);
    assert_ne!(pixel_of(&draw(&ram, BLEND, &regs())).color, RED);
}

#[test]
fn given_the_alpha_test_on_when_a_pixel_has_the_reference_alpha_then_it_is_hidden() {
    let ram = scene(&[(translucent(15, 2), quad(10, 10, 30, 30, 0, GREEN))]);
    assert_eq!(pixel_of(&draw(&ram, ALPHA_TEST, &with_ref(15))).alpha, 0);
}

#[test]
fn given_the_alpha_test_on_when_a_pixel_is_above_the_reference_alpha_then_it_draws() {
    let ram = scene(&[(translucent(16, 2), quad(10, 10, 30, 30, 0, GREEN))]);
    assert_eq!(pixel_of(&draw(&ram, ALPHA_TEST, &with_ref(15))).alpha, 16);
}

#[test]
fn given_the_alpha_test_off_when_the_reference_is_set_then_it_has_no_effect() {
    let ram = scene(&[(translucent(1, 2), quad(10, 10, 30, 30, 0, GREEN))]);
    assert_eq!(pixel_of(&draw(&ram, 0, &with_ref(31))).alpha, 1);
}

#[test]
fn given_the_reference_alpha_31_when_an_opaque_polygon_draws_then_it_is_hidden() {
    let ram = scene(&[(opaque(1), quad(10, 10, 30, 30, 0, RED))]);
    assert_eq!(pixel_of(&draw(&ram, ALPHA_TEST, &with_ref(31))).alpha, 0);
}

/// A pair of overlapping translucent quads listed lower one first; the upper one starts on line 10.
fn sort_scene() -> Ram {
    scene(&[
        (translucent(16, 2), quad(10, 20, 30, 40, 0, RED)),
        (translucent(16, 3), quad(10, 10, 30, 40, 0, GREEN)),
    ])
}

#[test]
fn given_auto_sort_when_two_translucent_polygons_overlap_then_the_upper_one_draws_first() {
    // Green (top line 10) first, red (top line 20) blends over it: R (63 x 17) / 32 = 33, G (63 x 15)
    // / 32 = 29, narrowed to 16 and 14.
    let lines = draw_all(&sort_scene(), BLEND, &regs(), 0);
    assert_eq!(at(&lines, 20, 25).color, 16 | 14 << 5);
}

#[test]
fn given_manual_sort_when_two_translucent_polygons_overlap_then_they_draw_in_the_order_listed() {
    let lines = draw_all(&sort_scene(), BLEND, &regs(), MANUAL_SORT);
    assert_eq!(at(&lines, 20, 25).color, 14 | 16 << 5);
}

#[test]
fn given_a_translucent_polygon_with_blending_off_when_drawn_then_it_keeps_all_its_edges() {
    let ram = scene(&[(translucent(15, 2), quad(10, 10, 30, 30, 0, GREEN))]);
    let lines = draw(&ram, BLEND, &regs());
    assert_eq!(covered(&lines), rect(10, 10, 29, 30));
}

#[test]
fn given_a_translucent_polygon_with_blending_off_when_drawn_then_it_drops_its_lower_and_right_edges()
 {
    let ram = scene(&[(translucent(15, 2), quad(10, 10, 30, 30, 0, GREEN))]);
    assert_eq!(covered(&draw(&ram, 0, &regs())), rect(10, 10, 29, 29));
}
