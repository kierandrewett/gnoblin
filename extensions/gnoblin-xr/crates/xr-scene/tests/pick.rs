//! Tests for `pick`: ray against layers, last-drawn wins.

use xr_scene::{Hit, IDENTITY, Layer, Placement, Ray, Space, pick};

fn quad(id: u64, z: f64, input: bool) -> Layer {
    Layer::Quad(Placement {
        id,
        source: id + 100,
        pose: xr_scene::Pose {
            position: [0.0, 0.0, z],
            orientation: IDENTITY,
        },
        size: [1.0, 0.5],
        extent: [1000.0, 500.0],
        opacity: 1.0,
        space: Space::World,
        input,
    })
}

fn ray(origin: [f64; 3], direction: [f64; 3]) -> Ray {
    Ray { origin, direction }
}

fn hit_id(hit: Option<Hit>) -> Option<u64> {
    hit.map(|h| h.id)
}

#[test]
fn ray_through_quad_centre_gives_half_extent() {
    let layers = [quad(7, -2.0, true)];
    let hit = pick(&layers, &ray([0.0, 0.0, 0.0], [0.0, 0.0, -1.0])).expect("hit");
    assert_eq!(hit.id, 7);
    assert_eq!(hit.source, 107, "the hit carries the window node the View shows");
    assert!(
        (hit.u - 500.0).abs() < 1e-9 && (hit.v - 250.0).abs() < 1e-9,
        "({}, {})",
        hit.u,
        hit.v
    );
    assert!((hit.point[2] + 2.0).abs() < 1e-9);
}

#[test]
fn ray_through_corner_maps_to_pixel_corner() {
    // Top-left in metres is (-0.5, 0.25) relative to the centre: u = 0, v = 0 with v down.
    let layers = [quad(7, -2.0, true)];
    let hit = pick(&layers, &ray([-0.5, 0.25, 0.0], [0.0, 0.0, -1.0])).expect("hit");
    assert!(hit.u.abs() < 1e-9 && hit.v.abs() < 1e-9, "({}, {})", hit.u, hit.v);
}

#[test]
fn nearer_later_layer_wins() {
    // Layer 2 is drawn after layer 1 and sits in front of it.
    let layers = [quad(1, -3.0, true), quad(2, -1.0, true)];
    assert_eq!(hit_id(pick(&layers, &ray([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]))), Some(2));
}

#[test]
fn later_layer_wins_even_when_farther() {
    // No depth test: the compositor draws in submission order, so the last drawn is what the user sees.
    let layers = [quad(1, -1.0, true), quad(2, -3.0, true)];
    assert_eq!(hit_id(pick(&layers, &ray([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]))), Some(2));
}

#[test]
fn ray_that_misses_gives_none() {
    let layers = [quad(7, -2.0, true)];
    assert!(pick(&layers, &ray([5.0, 0.0, 0.0], [0.0, 0.0, -1.0])).is_none());
}

#[test]
fn ray_parallel_to_plane_gives_none() {
    let layers = [quad(7, -2.0, true)];
    assert!(pick(&layers, &ray([-5.0, 0.0, -2.0], [1.0, 0.0, 0.0])).is_none());
}

#[test]
fn layer_with_input_false_is_skipped() {
    // The later layer does not take input, so the earlier one under it is hit.
    let layers = [quad(1, -2.0, true), quad(2, -2.0, false)];
    assert_eq!(hit_id(pick(&layers, &ray([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]))), Some(1));
}

#[test]
fn quad_is_one_sided() {
    // A ray from behind the quad (+Z side is the front) does not hit it.
    let layers = [quad(7, -2.0, true)];
    assert!(pick(&layers, &ray([0.0, 0.0, -5.0], [0.0, 0.0, 1.0])).is_none());
}

#[test]
fn quad_behind_the_origin_is_not_hit() {
    let layers = [quad(7, -2.0, true)];
    assert!(pick(&layers, &ray([0.0, 0.0, -5.0], [0.0, 0.0, -1.0])).is_none());
}

#[test]
fn projection_layers_are_not_picked() {
    let layers = [Layer::Projection(Placement {
        id: 9,
        source: 109,
        pose: xr_scene::Pose {
            position: [0.0, 0.0, -2.0],
            orientation: IDENTITY,
        },
        size: [1.0, 0.5],
        extent: [1000.0, 500.0],
        opacity: 1.0,
        space: Space::World,
        input: true,
    })];
    assert!(pick(&layers, &ray([0.0, 0.0, 0.0], [0.0, 0.0, -1.0])).is_none());
}
