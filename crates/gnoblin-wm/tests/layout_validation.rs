//! One test per validation failure mode, plus the accepted path and the fallback.

use gnoblin_types::{Vec3, WindowId};
use gnoblin_wm::layout::{
    Camera, Handled, LayoutEffect, LayoutFailure, LayoutWindow, Origin, Params, ParamsError, Placement, REFUSED_CODE,
    Reply, ReplyError, Subject, Transform3, fallback, settle,
};

fn win(id: u64) -> WindowId {
    WindowId::new(id)
}

fn params(ids: &[u64]) -> Params {
    let windows = ids
        .iter()
        .map(|&id| LayoutWindow {
            id: win(id),
            width: 100.0,
            height: 50.0,
        })
        .collect();
    Params::new(800.0, 600.0, windows).expect("valid params")
}

fn handled(placements: Vec<Placement>, effects: Vec<LayoutEffect>) -> Reply {
    Reply::Handled(Handled {
        placements,
        camera: Camera::Orthographic,
        effects,
    })
}

fn refused(reply: Reply, ids: &[u64]) -> ReplyError {
    match settle(&params(ids), reply) {
        Err(LayoutFailure::RefusedReply(error)) => error,
        other => panic!("expected a refused reply, got {other:?}"),
    }
}

#[test]
fn accepts_full_cover_and_sorts_by_window() {
    let reply = handled(
        vec![
            Placement::rect(win(2), 10.0, 0.0, 100.0, 50.0),
            Placement::rect(win(1), 0.0, 0.0, 100.0, 50.0),
        ],
        vec![],
    );
    let layout = settle(&params(&[1, 2]), reply).expect("accepted");
    assert_eq!(layout.origin, Origin::Reply);
    let order: Vec<WindowId> = layout.placements.iter().map(|p| p.window).collect();
    assert_eq!(order, vec![win(1), win(2)]);
}

#[test]
fn accepts_float_effect_in_place_of_placement() {
    let reply = handled(
        vec![Placement::rect(win(1), 0.0, 0.0, 100.0, 50.0)],
        vec![LayoutEffect::Float { window: win(2) }],
    );
    let layout = settle(&params(&[1, 2]), reply).expect("accepted");
    assert_eq!(layout.effects, vec![LayoutEffect::Float { window: win(2) }]);
}

#[test]
fn refuses_unknown_placed_window() {
    let reply = handled(vec![Placement::rect(win(9), 0.0, 0.0, 1.0, 1.0)], vec![]);
    assert_eq!(refused(reply, &[1]), ReplyError::UnknownWindow(win(9)));
}

#[test]
fn refuses_unknown_float_window() {
    let reply = handled(vec![], vec![LayoutEffect::Float { window: win(9) }]);
    assert_eq!(refused(reply, &[1]), ReplyError::UnknownWindow(win(9)));
}

#[test]
fn refuses_duplicate_placement() {
    let reply = handled(
        vec![
            Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0),
            Placement::rect(win(1), 5.0, 0.0, 1.0, 1.0),
        ],
        vec![],
    );
    assert_eq!(refused(reply, &[1]), ReplyError::DuplicatePlacement(win(1)));
}

#[test]
fn refuses_duplicate_float_effect() {
    let reply = handled(
        vec![],
        vec![
            LayoutEffect::Float { window: win(1) },
            LayoutEffect::Float { window: win(1) },
        ],
    );
    assert_eq!(refused(reply, &[1]), ReplyError::DuplicateEffect(win(1)));
}

#[test]
fn refuses_missing_window() {
    let reply = handled(vec![Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0)], vec![]);
    assert_eq!(refused(reply, &[1, 2]), ReplyError::MissingWindow(win(2)));
}

#[test]
fn refuses_float_of_placed_window() {
    let reply = handled(
        vec![Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0)],
        vec![LayoutEffect::Float { window: win(1) }],
    );
    assert_eq!(refused(reply, &[1]), ReplyError::FloatedWindowPlaced(win(1)));
}

#[test]
fn refuses_non_finite_width() {
    let reply = handled(vec![Placement::rect(win(1), 0.0, 0.0, f64::NAN, 1.0)], vec![]);
    assert_eq!(
        refused(reply, &[1]),
        ReplyError::NonFinite {
            subject: Subject::Window(win(1)),
            field: "width",
        }
    );
}

#[test]
fn refuses_out_of_bound_translation() {
    let mut placement = Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0);
    placement.transform = Transform3::at(2_000_000.0, 0.0);
    assert_eq!(
        refused(handled(vec![placement], vec![]), &[1]),
        ReplyError::OutOfBounds {
            subject: Subject::Window(win(1)),
            field: "position",
        }
    );
}

#[test]
fn refuses_negative_size() {
    let reply = handled(vec![Placement::rect(win(1), 0.0, 0.0, -1.0, 1.0)], vec![]);
    assert_eq!(refused(reply, &[1]), ReplyError::NegativeSize(win(1)));
}

#[test]
fn refuses_opacity_above_one() {
    let mut placement = Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0);
    placement.opacity = 1.5;
    assert_eq!(
        refused(handled(vec![placement], vec![]), &[1]),
        ReplyError::OpacityOutOfRange(win(1))
    );
}

#[test]
fn refuses_non_finite_opacity() {
    let mut placement = Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0);
    placement.opacity = f64::INFINITY;
    assert_eq!(
        refused(handled(vec![placement], vec![]), &[1]),
        ReplyError::NonFinite {
            subject: Subject::Window(win(1)),
            field: "opacity",
        }
    );
}

#[test]
fn refuses_zero_scale_as_not_invertible() {
    let mut placement = Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0);
    placement.transform = Transform3::Trs {
        position: Vec3::new(0.0, 0.0, 0.0),
        rotation_degrees: Vec3::new(0.0, 0.0, 0.0),
        scale: Vec3::new(0.0, 1.0, 1.0),
    };
    assert_eq!(
        refused(handled(vec![placement], vec![]), &[1]),
        ReplyError::NotInvertible(Subject::Window(win(1)))
    );
}

#[test]
fn refuses_singular_raw_matrix() {
    let mut placement = Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0);
    placement.transform = Transform3::Matrix([0.0; 16]);
    assert_eq!(
        refused(handled(vec![placement], vec![]), &[1]),
        ReplyError::NotInvertible(Subject::Window(win(1)))
    );
}

#[test]
fn refuses_non_finite_raw_matrix() {
    let mut matrix = Transform3::IDENTITY.to_matrix();
    matrix[3] = f64::NAN;
    let mut placement = Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0);
    placement.transform = Transform3::Matrix(matrix);
    assert_eq!(
        refused(handled(vec![placement], vec![]), &[1]),
        ReplyError::NonFinite {
            subject: Subject::Window(win(1)),
            field: "matrix",
        }
    );
}

#[test]
fn refuses_perspective_field_of_view_out_of_range() {
    let reply = Reply::Handled(Handled {
        placements: vec![Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0)],
        camera: Camera::Perspective {
            fov_degrees: 180.0,
            near: 0.1,
            far: 10.0,
            pose: Transform3::IDENTITY,
        },
        effects: vec![],
    });
    assert_eq!(refused(reply, &[1]), ReplyError::FieldOfViewOutOfRange);
}

#[test]
fn refuses_non_positive_near_plane() {
    let reply = Reply::Handled(Handled {
        placements: vec![Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0)],
        camera: Camera::Perspective {
            fov_degrees: 60.0,
            near: 0.0,
            far: 10.0,
            pose: Transform3::IDENTITY,
        },
        effects: vec![],
    });
    assert_eq!(refused(reply, &[1]), ReplyError::NearPlaneInvalid);
}

#[test]
fn refuses_far_plane_not_beyond_near() {
    let reply = Reply::Handled(Handled {
        placements: vec![Placement::rect(win(1), 0.0, 0.0, 1.0, 1.0)],
        camera: Camera::Perspective {
            fov_degrees: 60.0,
            near: 5.0,
            far: 5.0,
            pose: Transform3::IDENTITY,
        },
        effects: vec![],
    });
    assert_eq!(refused(reply, &[1]), ReplyError::FarPlaneInvalid);
}

#[test]
fn refused_reply_carries_wire_code() {
    assert_eq!(REFUSED_CODE.as_str(), "refused");
    assert_eq!(ReplyError::NegativeSize(win(1)).code().as_str(), "refused");
}

#[test]
fn declined_reply_uses_fallback() {
    let layout = settle(&params(&[2, 1]), Reply::Declined).expect("fallback");
    assert_eq!(layout.origin, Origin::Fallback);
    assert_eq!(layout, fallback(&params(&[2, 1])));
    let order: Vec<WindowId> = layout.placements.iter().map(|p| p.window).collect();
    assert_eq!(order, vec![win(1), win(2)]);
    assert!(layout.effects.is_empty());
}

#[test]
fn fallback_cascades_and_is_deterministic() {
    let first = fallback(&params(&[1, 2, 3]));
    let second = fallback(&params(&[1, 2, 3]));
    assert_eq!(first, second);
    let x: Vec<f64> = first
        .placements
        .iter()
        .map(|p| match p.transform {
            Transform3::Trs { position, .. } => position.x,
            Transform3::Matrix(_) => f64::NAN,
        })
        .collect();
    assert!(x[0] < x[1] && x[1] < x[2]);
}

#[test]
fn params_reject_duplicate_window() {
    let windows = vec![
        LayoutWindow {
            id: win(1),
            width: 1.0,
            height: 1.0,
        },
        LayoutWindow {
            id: win(1),
            width: 1.0,
            height: 1.0,
        },
    ];
    assert_eq!(
        Params::new(10.0, 10.0, windows),
        Err(ParamsError::DuplicateWindow(win(1)))
    );
}

#[test]
fn params_reject_empty_output() {
    assert_eq!(Params::new(0.0, 10.0, vec![]), Err(ParamsError::EmptyOutput));
}

#[test]
fn params_reject_non_finite_output() {
    assert_eq!(
        Params::new(f64::NAN, 10.0, vec![]),
        Err(ParamsError::NonFinite("output width"))
    );
}
