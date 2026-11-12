//! Tests for `compose`: planned layers and ready swapchains to submitted layers.

use std::collections::BTreeMap;

use xr_layers::{
    EyeVisibility, ImageRect, Limits, Notice, NoticeKind, SubmittedLayer, SwapchainId, SwapchainRef, compose,
};
use xr_scene::{Eye, Fov, IDENTITY, Layer, NodeId, Placement, Pose, Projected, Space};

const LIMITS: Limits = Limits {
    max_layer_count: 16,
    reserved: 0,
    color_scale_bias: true,
};

fn placement(id: NodeId, space: Space, z: f64, opacity: f64) -> Placement {
    Placement {
        id,
        source: 100 + id,
        pose: Pose {
            position: [0.5, 1.5, z],
            orientation: IDENTITY,
        },
        size: [1.6, 0.9],
        extent: [1920.0, 1080.0],
        opacity,
        space,
        input: true,
    }
}

fn quad(id: NodeId, space: Space) -> Layer {
    Layer::Quad(placement(id, space, -1.0, 1.0))
}

/// One 1920x1080 swapchain per view, numbered `view * 10`.
fn swapchains(views: &[NodeId]) -> BTreeMap<NodeId, SwapchainRef> {
    views
        .iter()
        .map(|&view| {
            (
                view,
                SwapchainRef {
                    id: SwapchainId(view * 10),
                    width: 1920,
                    height: 1080,
                },
            )
        })
        .collect()
}

fn eyes() -> [Eye; 2] {
    let fov = Fov {
        left: -0.8,
        right: 0.7,
        up: 0.9,
        down: -0.9,
    };
    [
        Eye {
            pose: Pose {
                position: [-0.032, 1.6, 0.0],
                orientation: IDENTITY,
            },
            fov,
        },
        Eye {
            pose: Pose {
                position: [0.032, 1.6, 0.0],
                orientation: IDENTITY,
            },
            fov,
        },
    ]
}

/// A projection layer for View `id`, with the two eyes of [`eyes`].
fn projection(id: NodeId, space: Space) -> Layer {
    Layer::Projection(Projected {
        placement: placement(id, space, 0.0, 1.0),
        eyes: eyes(),
    })
}

fn views(submitted: &[SubmittedLayer]) -> Vec<NodeId> {
    submitted
        .iter()
        .map(|layer| match layer {
            SubmittedLayer::Quad(quad) => quad.view,
            SubmittedLayer::Projection(projection) => projection.view,
        })
        .collect()
}

#[test]
fn a_world_quad_keeps_the_planned_pose_size_and_swapchain() {
    let layers = [quad(2, Space::World)];
    let chains = swapchains(&[2]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);

    assert!(out.notices.is_empty());
    let [SubmittedLayer::Quad(submitted)] = out.layers.as_slice() else {
        panic!("one quad layer, got {:?}", out.layers);
    };
    assert_eq!(submitted.view, 2);
    assert_eq!(submitted.swapchain, SwapchainId(20));
    assert_eq!(submitted.space, Space::World);
    assert_eq!(submitted.pose.position, [0.5, 1.5, -1.0]);
    assert_eq!(submitted.pose.orientation, IDENTITY);
    assert_eq!(submitted.size, [1.6, 0.9]);
    assert_eq!(submitted.eye_visibility, EyeVisibility::Both);
    assert_eq!(
        submitted.rect,
        ImageRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080
        }
    );
    assert_eq!(submitted.color_scale, None, "an opaque layer needs no colour scale");
}

#[test]
fn a_head_locked_quad_is_in_view_space() {
    let layers = [quad(2, Space::View)];
    let chains = swapchains(&[2]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);
    let [SubmittedLayer::Quad(submitted)] = out.layers.as_slice() else {
        panic!("one quad layer");
    };
    assert_eq!(submitted.space, Space::View);
}

#[test]
fn layers_keep_the_plan_order() {
    let layers = [quad(5, Space::World), quad(3, Space::World), quad(9, Space::View)];
    let chains = swapchains(&[3, 5, 9]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);
    assert_eq!(views(&out.layers), vec![5, 3, 9]);
}

#[test]
fn opacity_becomes_a_colour_scale_when_the_extension_is_enabled() {
    let layers = [Layer::Quad(placement(2, Space::World, -1.0, 0.25))];
    let chains = swapchains(&[2]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);
    let [SubmittedLayer::Quad(submitted)] = out.layers.as_slice() else {
        panic!("one quad layer");
    };
    assert_eq!(submitted.color_scale, Some([0.25, 0.25, 0.25, 0.25]));
    assert!(out.notices.is_empty());
}

#[test]
fn opacity_is_reported_and_ignored_without_the_extension() {
    let limits = Limits {
        color_scale_bias: false,
        ..LIMITS
    };
    let layers = [Layer::Quad(placement(2, Space::World, -1.0, 0.5))];
    let chains = swapchains(&[2]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &limits);
    let [SubmittedLayer::Quad(submitted)] = out.layers.as_slice() else {
        panic!("the layer is still submitted");
    };
    assert_eq!(submitted.color_scale, None);
    assert_eq!(
        out.notices,
        vec![Notice {
            view: 2,
            kind: NoticeKind::OpacityIgnored
        }]
    );
}

#[test]
fn a_view_without_a_swapchain_is_skipped_and_reported() {
    let layers = [quad(2, Space::World), quad(3, Space::World)];
    let chains = swapchains(&[3]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);
    assert_eq!(views(&out.layers), vec![3]);
    assert_eq!(
        out.notices,
        vec![Notice {
            view: 2,
            kind: NoticeKind::NoSwapchain
        }]
    );
}

#[test]
fn a_projection_layer_has_one_eye_per_half_of_the_image() {
    let layers = [projection(4, Space::World)];
    let chains = swapchains(&[4]);
    let eyes = eyes();
    let out = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);
    let [SubmittedLayer::Projection(projection)] = out.layers.as_slice() else {
        panic!("one projection layer, got {:?}", out.layers);
    };
    assert_eq!(projection.swapchain, SwapchainId(40));
    let [left, right] = &projection.eyes;
    assert_eq!(left.pose.position, [-0.032, 1.6, 0.0]);
    assert_eq!(right.pose.position, [0.032, 1.6, 0.0]);
    assert_eq!(left.fov, eyes[0].fov);
    assert_eq!(
        left.rect,
        ImageRect {
            x: 0,
            y: 0,
            width: 960,
            height: 1080
        }
    );
    assert_eq!(
        right.rect,
        ImageRect {
            x: 960,
            y: 0,
            width: 960,
            height: 1080
        }
    );
}

#[test]
fn an_odd_width_gives_the_spare_column_to_the_right_eye() {
    let layers = [projection(4, Space::World)];
    let odd = SwapchainRef {
        id: SwapchainId(1),
        width: 1921,
        height: 10,
    };
    let out = compose(&layers, &|_| Some(odd), &LIMITS);
    let [SubmittedLayer::Projection(projection)] = out.layers.as_slice() else {
        panic!("one projection layer");
    };
    let [left, right] = &projection.eyes;
    assert_eq!((left.rect.x, left.rect.width), (0, 960));
    assert_eq!((right.rect.x, right.rect.width), (960, 961));
}

#[test]
fn view_layers_beyond_the_budget_are_dropped_and_reported() {
    // Budget 3: two world layers and two head-locked ones. The second head-locked layer does not fit.
    let limits = Limits {
        max_layer_count: 3,
        ..LIMITS
    };
    let layers = [
        quad(1, Space::World),
        quad(2, Space::World),
        quad(3, Space::View),
        quad(4, Space::View),
    ];
    let chains = swapchains(&[1, 2, 3, 4]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &limits);
    assert_eq!(views(&out.layers), vec![1, 2, 3]);
    assert_eq!(
        out.notices,
        vec![Notice {
            view: 4,
            kind: NoticeKind::LayerBudget
        }]
    );
}

#[test]
fn world_layers_take_the_budget_before_head_locked_ones() {
    // The head-locked layer comes first in tree order, but world content keeps the budget.
    let limits = Limits {
        max_layer_count: 2,
        ..LIMITS
    };
    let layers = [quad(9, Space::View), quad(1, Space::World), quad(2, Space::World)];
    let chains = swapchains(&[1, 2, 9]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &limits);
    assert_eq!(views(&out.layers), vec![1, 2]);
    assert_eq!(
        out.notices,
        vec![Notice {
            view: 9,
            kind: NoticeKind::LayerBudget
        }]
    );
}

#[test]
fn world_layers_beyond_the_budget_are_dropped_last_first() {
    let limits = Limits {
        max_layer_count: 2,
        ..LIMITS
    };
    let layers = [quad(1, Space::World), quad(2, Space::World), quad(3, Space::World)];
    let chains = swapchains(&[1, 2, 3]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &limits);
    assert_eq!(views(&out.layers), vec![1, 2]);
    assert_eq!(
        out.notices,
        vec![Notice {
            view: 3,
            kind: NoticeKind::LayerBudget
        }]
    );
}

#[test]
fn the_reserved_layers_come_off_the_budget() {
    let limits = Limits {
        max_layer_count: 3,
        reserved: 2,
        ..LIMITS
    };
    let layers = [quad(1, Space::World), quad(2, Space::World)];
    let chains = swapchains(&[1, 2]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &limits);
    assert_eq!(views(&out.layers), vec![1]);
}

#[test]
fn a_budget_of_zero_submits_nothing() {
    let limits = Limits {
        max_layer_count: 0,
        ..LIMITS
    };
    let layers = [quad(1, Space::World)];
    let chains = swapchains(&[1]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &limits);
    assert!(out.layers.is_empty());
    assert_eq!(out.notices.len(), 1);
}

#[test]
fn views_without_swapchains_do_not_use_the_budget() {
    let limits = Limits {
        max_layer_count: 1,
        ..LIMITS
    };
    let layers = [quad(1, Space::World), quad(2, Space::World)];
    let chains = swapchains(&[2]);
    let out = compose(&layers, &|view| chains.get(&view).copied(), &limits);
    assert_eq!(views(&out.layers), vec![2]);
    assert_eq!(out.notices.len(), 1);
    assert_eq!(out.notices[0].kind, NoticeKind::NoSwapchain);
}

#[test]
fn notice_names_are_the_reported_strings() {
    assert_eq!(NoticeKind::LayerBudget.name(), "layer-budget");
    assert_eq!(NoticeKind::NoSwapchain.name(), "no-swapchain");
    assert_eq!(NoticeKind::OpacityIgnored.name(), "opacity-ignored");
}

#[test]
fn the_same_input_gives_the_same_output() {
    let layers = [quad(1, Space::World), quad(2, Space::View)];
    let chains = swapchains(&[1, 2]);
    let first = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);
    let second = compose(&layers, &|view| chains.get(&view).copied(), &LIMITS);
    assert_eq!(first, second);
}
