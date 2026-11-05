//! Pure scene graph, visual transforms, effects, damage, the tween player and
//! cursor selection for the Gnoblin compositor.
//!
//! This crate is the scene-side half of the Renderer card in
//! `design/rust-compositor.md` and contains no GPU code: no GL, no Smithay type,
//! no output, no clock. `gnoblin-compositor` performs the draw calls this crate
//! describes, drives [`TweenPlayer::advance`] from its frame clock, and forwards
//! [`Scene::take_damage`] to the renderer.
//!
//! Every rectangle, point, transform, vector, compositing scale and output
//! identity comes from `gnoblin-types`; this crate adds only what a scene needs
//! (a node's visual properties, effects, damage, tracks and the cursor choice).
//!
//! # What it guarantees
//!
//! - **3D by default.** A node carries a three-axis offset, rotation and scale
//!   plus a draw depth, and may be a space with a perspective camera. Every node
//!   is placed by a 4x4 matrix; a flat scene is the orthographic case at depth
//!   zero and draws exactly as before. Only a non-finite or non-invertible
//!   transform is refused, with the code `unsupported-3d`
//!   ([`gnoblin_types::Unsupported3d::CODE`]) on the value that caused it.
//! - **Damage follows the transform.** Every mutation damages the projected
//!   pixel bounds of the node and its descendants before and after the change,
//!   so a move damages the pixels it left and the pixels it entered, and a
//!   removal damages the whole subtree. Ancestors are marked dirty for
//!   recomputation but add no rectangles of their own. A bounded merge and a
//!   per-output cap degrade to full damage instead of dropping rectangles, and a
//!   perspective camera degrades its outputs to full damage directly.
//! - **Every track ends exactly once.** A started track is either live or in the
//!   ended queue, and every path out of the live map produces one
//!   [`TrackEnd`]. A closed node's tracks end with
//!   [`TrackEndReason::TargetGone`], which is the plan's "every animation ends
//!   and reports `finished`" rule made structural.
//! - **One writer per component.** Two tracks may animate one node only when
//!   they write disjoint components; starting a track on an owned property ends
//!   the older one with [`TrackEndReason::Superseded`]. Tracks advance in
//!   ascending id order, so a frame does not depend on map iteration order.
//! - **Hit testing is the inverse of rendering.** [`Scene::hit_test`] maps the
//!   point through the inverse of the same matrix chain, so a hit cannot
//!   disagree with the drawn position.
//!
//! # Failure modes first
//!
//! Each entry is a way this crate could be wrong, and the test that covers it:
//!
//! 1. A track never finishes when its node closes:
//!    `tween::tests::closing_a_node_finishes_its_tracks_without_writing_to_it`,
//!    `tween::tests::finish_node_reports_a_closure_in_the_same_turn`.
//! 2. Two tracks fight over one property nondeterministically:
//!    `tween::tests::a_new_track_supersedes_the_previous_one_deterministically`,
//!    `tween::tests::a_uniform_scale_track_supersedes_an_axis_track_and_the_other_axis_survives`.
//! 3. Damage misses a child of a transformed parent:
//!    `scene::tests::damage_covers_a_child_of_a_transformed_parent`.
//! 4. A user shader uniform with the wrong type is accepted:
//!    `effect::tests::a_wrong_uniform_type_is_refused_and_the_value_is_unchanged`.
//! 5. Retarget jumps the value: `tween::tests::retarget_never_jumps_the_value`.
//! 6. A move leaves the old pixels behind:
//!    `scene::tests::damage_covers_where_a_node_was_before_it_moved`.
//! 7. Removing a node leaves its subtree on screen:
//!    `scene::tests::removing_a_node_damages_its_whole_subtree`.
//! 8. Damage silently drops rectangles past a cap, or keeps a tight rectangle
//!    under a perspective camera:
//!    `scene::tests::the_damage_cap_degrades_to_full_output_instead_of_dropping_rectangles`,
//!    `scene::tests::a_perspective_camera_falls_back_to_full_damage`.
//! 9. A 3D value is silently dropped instead of stored, or a 2D-only effect
//!    accepts depth and draws the wrong thing:
//!    `scene::tests::a_depth_transform_is_accepted_and_stored`,
//!    `scene::tests::a_depth_property_write_is_stored`,
//!    `scene::tests::a_perspective_space_is_accepted_and_a_bad_field_of_view_is_refused`,
//!    `tween::tests::a_depth_track_starts_and_reaches_its_target`,
//!    `effect::tests::a_3d_shadow_offset_is_refused_by_name`.
//! 10. Damage is computed from an untransformed rectangle:
//!     `scene::tests::damage_covers_a_child_of_a_transformed_parent`,
//!     `scene::tests::an_effect_change_damages_the_padding_a_shadow_draws_outside_the_bounds`.
//! 11. A cancelled track still applies its target, or reports twice:
//!     `tween::tests::cancel_reports_once_and_leaves_the_value`.
//! 12. A zero-duration or delayed track never settles:
//!     `tween::tests::a_zero_duration_track_completes_at_start`,
//!     `tween::tests::a_delayed_track_holds_its_start_value_until_the_delay_passes`.
//! 13. A spring or Bezier curve jumps off its endpoints:
//!     `easing::tests::every_easing_hits_both_endpoints`.
//! 14. A missing cursor theme leaves no cursor:
//!     `cursor::tests::a_missing_theme_falls_back_to_the_installed_default_theme`,
//!     `cursor::tests::with_nothing_installed_the_built_in_pointer_is_used`.
//! 15. A cursor size outside the documented range reaches the renderer, or the
//!     output scale is applied twice:
//!     `cursor::tests::the_size_is_clamped_to_the_documented_range`,
//!     `cursor::tests::the_output_scale_multiplies_the_selected_size_once`.
//! 16. A duplicate node replaces a live one:
//!     `scene::tests::a_duplicate_node_and_a_missing_parent_are_refused`.
//! 17. A non-finite value reaches geometry:
//!     `scene::tests::a_non_finite_transform_is_refused`,
//!     `property::tests::a_non_finite_value_is_refused`.
//! 18. A hit test misses a transformed node:
//!     `scene::tests::hit_testing_goes_through_the_inverse_transform`,
//!     `scene::tests::a_rotated_parent_is_hit_through_the_inverse_rotation`.
//! 19. An undeclared or reserved shader uniform is accepted:
//!     `effect::tests::an_undeclared_uniform_is_refused`,
//!     `effect::tests::a_reserved_or_malformed_name_is_refused`.
//!
//! # Model test
//!
//! `model_test::the_scene_and_the_player_keep_their_invariants` drives a scene
//! and a player through a deterministic sequence (a fixed-seed generator, no
//! time, no threads) of inserts, removals, transforms, property writes, z-order
//! changes, track starts, cancellations, retargets and ticks. After every step it
//! asserts that the damage covers every node whose projected bounds changed, and
//! at the end that every started track produced exactly one terminal report with
//! its own id. It prints its counts with `--nocapture`.
//!
//! # Deliberate choices the plan leaves open
//!
//! - **"Minimal damage set"** is a greedy, order-independent merge that costs at
//!   most a doubling of the redrawn area per merge, with a per-output cap that
//!   degrades to full damage. An exact minimum rectilinear cover is not
//!   attempted; damage is allowed to be too large and never too small.
//! - **Composition** is exclusive ownership per transform component rather than
//!   additive tracks, and the newest track wins. The plan asks for a documented,
//!   deterministic rule, not a particular one.
//! - **Retarget** starts from the value last applied to the node, at the tick
//!   boundary, and does not preserve velocity. It therefore cannot jump, and a
//!   retarget consumes no time.
//! - **A spring's** `stiffness`, `damping` and `mass` are measured in normalised
//!   progress, so a track's duration time-scales the same motion. The guide does
//!   not define spring parameters; the formula is documented on
//!   [`Easing::Spring`].
//! - **Cursor theme sizes** are matched against the logical request, because a
//!   theme declares its sizes at scale 1. The physical size is the chosen
//!   nominal size times the output scale, rounded once in integer arithmetic.

mod camera;
mod color;
mod cursor;
mod damage;
mod easing;
mod effect;
mod id;
mod matrix;
mod property;
mod scene;
mod transform;
mod tween;

#[cfg(test)]
mod model_test;

pub use camera::{Camera, CameraError, Projection};
pub use color::Color;
pub use cursor::{
    CursorBackend, CursorBuild, CursorSelection, DEFAULT_CURSOR_SIZE, DEFAULT_CURSOR_THEME, InstalledTheme,
    MAX_CURSOR_SIZE, MIN_CURSOR_SIZE, select_cursor,
};
pub use damage::{Damage, OutputDamage, merge_rects};
pub use easing::{Easing, EasingError};
pub use effect::{
    Blur, EffectError, EffectExtent, Effects, MAX_BLUR_RADIUS, MAX_PASSES, MAX_UNIFORMS, Pass, ResolvedUniform, Shader,
    Shadow, UniformSchema, UniformType, UniformValue,
};
pub use gnoblin_types::{OutputId, PointF, Rect, RectF, Scale, Transform, Unsupported3d, Unsupported3dField, Vec3};
pub use id::{NodeId, TrackId};
pub use matrix::Mat4;
pub use property::{PropertyError, PropertyValue, ValueKind, VisualProperty};
pub use scene::{DEFAULT_DAMAGE_RECT_LIMIT, Hit, Node, NodeKind, NodeSpec, Scene, SceneError};
pub use transform::VisualTransform;
pub use tween::{TrackEnd, TrackEndReason, TrackError, TrackSpec, TrackStart, TweenPlayer};
