// SPDX-License-Identifier: Apache-2.0
//! Shadow polygons and the stencil (`docs/hw/rasterizer.md`, "Shadow polygons"): a mask pass of ID 0
//! that sets stencil flags where it is in front, and a render pass that draws only where the flags
//! are clear and its ID differs from the pixel's.

use super::*;

const BLEND: u16 = 1 << 3;
const BLACK: u16 = 0;
const SHADOW_MODE: u32 = 3 << 4;
const BACK_ONLY: u32 = 0x40;
const FRONT_ONLY: u32 = 0x80;
const MANUAL_SORT: u8 = 1;
const IN_FRONT: i32 = -1000;
const BEHIND: i32 = 1000;

/// A clockwise quad over pixels 10-30, lines 10-30 at depth `z`: the back face of a volume.
fn clockwise(z: i32) -> Vec<Vertex> {
    vec![
        vertex(10, 10, z, BLACK),
        vertex(30, 10, z, BLACK),
        vertex(30, 30, z, BLACK),
        vertex(10, 30, z, BLACK),
    ]
}

/// An anti-clockwise quad over the same pixels: the front face of a volume.
fn anticlockwise(z: i32) -> Vec<Vertex> {
    quad(10, 10, 30, 30, z, BLACK)
}

/// The mask pass: mode 3, ID 0, alpha 15, back surface only.
fn mask() -> u32 {
    SHADOW_MODE | BACK_ONLY | 15 << 16
}

/// The render pass of ID `id`: mode 3, alpha 15, front surface only.
fn shadow(id: u32) -> u32 {
    SHADOW_MODE | FRONT_ONLY | 15 << 16 | id << 24
}

fn floor(id: u32) -> (u32, Vec<Vertex>) {
    (opaque(id), quad(0, 0, 60, 60, 0, RED))
}

/// What a black shadow of alpha 15 makes of opaque red: 63 x 16 / 32 = 31 in 6 bits, 15 in 5.
fn shadowed() -> Pixel3d {
    Pixel3d {
        color: 15,
        alpha: 31,
    }
}

fn plain() -> Pixel3d {
    Pixel3d {
        color: RED,
        alpha: 31,
    }
}

fn result(ram: &Ram) -> Pixel3d {
    at(&draw_all(ram, BLEND, &regs(), MANUAL_SORT), 20, 20)
}

#[test]
fn given_a_volume_whose_back_is_behind_the_surface_when_both_passes_run_then_the_surface_is_shadowed()
 {
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(BEHIND)),
        (shadow(2), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), shadowed());
}

#[test]
fn given_a_volume_wholly_in_front_of_the_surface_when_both_passes_run_then_the_surface_is_not_shadowed()
 {
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(IN_FRONT)),
        (shadow(2), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), plain());
}

#[test]
fn given_a_mask_pass_alone_when_drawn_then_it_draws_no_colour() {
    let ram = scene(&[floor(1), (mask(), clockwise(IN_FRONT))]);
    assert_eq!(result(&ram), plain());
}

#[test]
fn given_a_shadow_of_the_surfaces_own_id_when_both_passes_run_then_the_surface_is_not_shadowed() {
    let ram = scene(&[
        floor(2),
        (mask(), clockwise(BEHIND)),
        (shadow(2), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), plain());
}

#[test]
fn given_a_stencil_set_when_two_render_passes_follow_one_mask_then_neither_draws() {
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(IN_FRONT)),
        (shadow(2), anticlockwise(IN_FRONT)),
        (shadow(3), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), plain());
}

/// The back-face render pass of ID `id`: mode 3, alpha 15, back surface only.
fn back_shadow(id: u32) -> u32 {
    SHADOW_MODE | BACK_ONLY | 15 << 16 | id << 24
}

#[test]
fn given_a_volume_in_front_of_the_surface_when_the_render_pass_draws_both_faces_then_neither_shadows()
 {
    // The front face and the back face of one volume look at the same flag: neither draws.
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(IN_FRONT)),
        (shadow(2), anticlockwise(IN_FRONT)),
        (back_shadow(2), clockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), plain());
}

#[test]
fn given_a_volume_around_the_surface_when_the_render_pass_draws_both_faces_then_the_surface_is_shadowed_once()
 {
    // The back face is behind the floor and fails the depth test; the front face draws.
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(BEHIND)),
        (shadow(2), anticlockwise(IN_FRONT)),
        (back_shadow(2), clockwise(BEHIND)),
    ]);
    assert_eq!(result(&ram), shadowed());
}

#[test]
fn given_a_next_volume_when_its_mask_pass_begins_then_the_flags_of_the_last_are_cleared() {
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(IN_FRONT)),
        (shadow(2), anticlockwise(IN_FRONT)),
        (mask(), clockwise(BEHIND)),
        (shadow(3), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), shadowed());
}

#[test]
fn given_two_mask_passes_in_a_row_when_the_second_begins_then_the_first_flags_stay() {
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(IN_FRONT)),
        (mask(), clockwise(BEHIND)),
        (shadow(2), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), plain());
}

#[test]
fn given_two_shadows_of_different_ids_when_they_overlap_then_the_surface_is_shaded_twice() {
    // The second shade: 31 x 16 / 32 = 15 in 6 bits, 7 in 5.
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(BEHIND)),
        (shadow(2), anticlockwise(IN_FRONT)),
        (shadow(3), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(
        result(&ram),
        Pixel3d {
            color: 7,
            alpha: 31
        }
    );
}

#[test]
fn given_two_shadows_of_one_id_when_they_overlap_then_the_surface_is_shaded_once() {
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(BEHIND)),
        (shadow(2), anticlockwise(IN_FRONT)),
        (shadow(2), anticlockwise(IN_FRONT)),
    ]);
    assert_eq!(result(&ram), shadowed());
}

#[test]
fn given_a_shadow_polygon_when_the_surface_is_opaque_and_listed_after_then_it_still_draws_last() {
    let ram = scene(&[
        (mask(), clockwise(BEHIND)),
        (shadow(2), anticlockwise(IN_FRONT)),
        floor(1),
    ]);
    assert_eq!(result(&ram), shadowed());
}

#[test]
fn given_a_shadow_that_sets_the_update_depth_bit_when_a_polygon_follows_then_the_shadow_wrote_no_depth()
 {
    // The shadow's own depth (in front) would hide the farther translucent polygon if it were kept.
    let ram = scene(&[
        floor(1),
        (mask(), clockwise(BEHIND)),
        (shadow(2) | 1 << 11, anticlockwise(IN_FRONT)),
        (3 << 24 | 15 << 16 | 0xC0, quad(10, 10, 30, 30, -500, GREEN)),
    ]);
    // The green polygon (alpha 15, ID 3) blends over the shaded red: R (0 x 16 + 31 x 16) / 32 = 15
    // in 6 bits, 7 in 5; G (63 x 16) / 32 = 31, 15 in 5.
    assert_eq!(
        result(&ram),
        Pixel3d {
            color: 7 | 15 << 5,
            alpha: 31
        }
    );
}
