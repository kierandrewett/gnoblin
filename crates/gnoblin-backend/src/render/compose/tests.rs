use gnoblin_types::{Point, WindowId};

use super::motion::{FLASH_MS, FLASH_PEAK};
use super::*;
use gnoblin_render::Actor;

fn content() -> Content {
    Content {
        src: RectF::new(0.0, 0.0, 40.0, 50.0),
        size: Size::new(40, 50),
        transform: Transform::Normal,
    }
}

/// A composer with the desktop's motion set, as the std library sets it: these tests are about what the compositor's own motion does, and
/// a composer nobody set motion on lands every change at once (`MotionPolicy::default`).
fn stock_composer() -> Composer {
    let mut composer = Composer::new();
    composer.set_stock_motion();
    composer
}

fn screen() -> (Composer, OutputId) {
    let mut composer = stock_composer();
    let output = OutputId::new(0);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    // The output's first frame is not this test's subject: it is asked for and spent.
    composer.begin_frame();
    composer.frame_asked(output);
    composer.end_frame();
    (composer, output)
}

/// The damage the next frame for `output` carries, or `None` when nothing
/// changed.
fn damage_of(composer: &mut Composer, output: OutputId) -> Option<Vec<Rect>> {
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    let changed = composer.compose(output, &mut frame);
    composer.end_frame();
    changed.then(|| frame.damage().to_vec())
}

#[test]
fn a_window_without_a_buffer_is_never_drawn() {
    let (mut composer, output) = screen();
    composer.insert_window(WindowId::new(1), Rect::new(10, 10, 100, 100));
    composer.place_window(WindowId::new(1), Rect::new(50, 50, 100, 100));
    assert_eq!(
        damage_of(&mut composer, output),
        None,
        "a window with no committed content has no pixels to damage"
    );
}

#[test]
fn a_window_with_content_but_no_buffer_has_no_layer() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(composer.compose(output, &mut frame));
    composer.end_frame();
    assert!(
        frame.layers().is_empty(),
        "the node exists and damages its rectangle, but nothing can be drawn"
    );
}

// The composer's nodes are compositor-owned, so they live in the compositor's id range. A
// shell node may hold any id below the base, and must never block a window's node.
#[test]
fn every_node_the_composer_creates_has_a_compositor_id() {
    let (mut composer, output) = screen();
    for raw in 1..=3 {
        let window = WindowId::new(raw);
        composer.insert_window(window, Rect::new(10 * raw as i32, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
    }
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    // The compositor's root and the output's screen are nodes too; the windows are the rest.
    let screen = composer.screen_node(output);
    let ids: Vec<NodeId> = composer.with_scene(|scene| {
        let root = scene.root();
        scene
            .node_ids()
            .filter(|id| Some(*id) != root && Some(*id) != screen)
            .collect()
    });
    assert_eq!(ids.len(), 3, "one node per window with content");
    for id in ids {
        assert!(
            id.get() >= gnoblin_render::scene_ids::COMPOSITOR_ID_BASE,
            "composer node {id} is in the shell's id range"
        );
    }
}

#[test]
fn a_shell_node_on_the_composers_first_id_does_not_block_a_window() {
    let (mut composer, _output) = screen();
    let shell = NodeId::new(1);
    composer.with_scene(|scene| {
        scene
            .insert_as(
                gnoblin_render::Actor::Shell,
                shell,
                NodeSpec::new(Rect::new(0, 0, 10, 10)),
            )
            .expect("the shell may insert a node below the base");
    });
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.with_scene(|scene| {
        assert!(scene.contains(shell), "the shell's node is untouched");
        assert_eq!(
            scene.node_ids().count(),
            4,
            "the shell, the root, the screen and the window each have a node"
        );
    });
}

#[test]
fn a_window_node_id_is_never_reused_within_a_session() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    let first = node_of(&composer, window);
    composer.remove_window(window);
    let again = WindowId::new(2);
    composer.insert_window(again, Rect::new(10, 10, 100, 100));
    composer.set_window_content(again, Some(content()));
    let second = node_of(&composer, again);
    assert!(second.get() > first.get(), "{second} reuses or precedes {first}");
}

// A secure mark redraws the window, so a capture reads the new mark rather than a
// frame drawn before it. Setting the same mark again changes nothing and damages nothing.
#[test]
fn a_secure_mark_damages_the_window_only_when_it_changes() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    assert!(damage_of(&mut composer, output).is_some(), "the new node is drawn once");
    assert_eq!(
        damage_of(&mut composer, output),
        None,
        "a steady window has nothing to draw"
    );

    composer.set_window_secure(window, true);
    assert!(
        damage_of(&mut composer, output).is_some(),
        "marking secure redraws the window"
    );
    composer.set_window_secure(window, true);
    assert_eq!(damage_of(&mut composer, output), None, "the same mark damages nothing");

    composer.set_window_secure(window, false);
    assert!(
        damage_of(&mut composer, output).is_some(),
        "clearing the mark redraws the window"
    );
}

// The shell marks a window secure by writing `secure` on its node. The composer follows the node: the mark
// takes effect when the shell writes it, and clears when the shell writes false.
#[test]
fn a_secure_value_the_shell_wrote_on_the_node_marks_the_window() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    assert!(damage_of(&mut composer, output).is_some(), "the new node is drawn once");

    composer.follow_shell_secure();
    assert_eq!(
        damage_of(&mut composer, output),
        None,
        "a node with no secure value keeps the window's mark"
    );

    composer.with_scene(|scene| {
        scene
            .set_value(node, "secure", TreeValue::Bool(true))
            .expect("the node takes it")
    });
    composer.follow_shell_secure();
    assert!(
        damage_of(&mut composer, output).is_some(),
        "the shell's mark redraws the window for capture"
    );
    composer.follow_shell_secure();
    assert_eq!(damage_of(&mut composer, output), None, "the same mark damages nothing");

    composer.with_scene(|scene| {
        scene
            .set_value(node, "secure", TreeValue::Bool(false))
            .expect("the node takes it")
    });
    composer.follow_shell_secure();
    assert!(
        damage_of(&mut composer, output).is_some(),
        "the shell clearing the mark redraws the window"
    );
}

#[test]
fn an_output_resize_damages_the_whole_output() {
    let (mut composer, output) = screen();
    composer.set_output(output, Rect::new(0, 0, 1024, 768));
    assert_eq!(
        damage_of(&mut composer, output),
        Some(vec![Rect::new(0, 0, 1024, 768)]),
        "the new buffer holds no pixels, so everything is redrawn"
    );
}

#[test]
fn an_output_that_did_not_change_damages_nothing() {
    let (mut composer, output) = screen();
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    assert_eq!(damage_of(&mut composer, output), None);
}

#[test]
fn closing_a_window_damages_the_pixels_it_held() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    damage_of(&mut composer, output);
    composer.set_window_content(window, None);
    let rects = damage_of(&mut composer, output).expect("the rectangle it held");
    assert!(
        rects.iter().any(|rect| rect.contains(Point::new(20, 30))),
        "the rectangle the window held is damaged: {rects:?}"
    );
}

/// A picture the size of `content()`, for a closing window to keep.
fn picture() -> Arc<Image> {
    let bytes = vec![0xff; 40 * 50 * 4];
    Arc::new(Image::new(Size::new(40, 50), 40 * 4, bytes).expect("a picture the size of its window"))
}

/// Adds a View at (10, 20), 40 by 30, under `parent` or the root, sourced from `source`.
fn add_view(composer: &mut Composer, view: NodeId, parent: Option<NodeId>, source: Option<NodeId>) {
    composer.with_scene(|scene| {
        let mut spec = NodeSpec::new(Rect::new(10, 20, 40, 30));
        spec.parent = parent;
        scene.insert(view, spec).expect("a view node");
        scene
            .set_class(view, Some("view".to_owned()))
            .expect("the view's class");
        scene.set_source(view, source).expect("the view's source");
    });
}

/// Adds the root of a view's source subtree: a plain node at `rect`.
fn add_source(composer: &mut Composer, source: NodeId, rect: Rect) {
    composer.with_scene(|scene| scene.insert(source, NodeSpec::new(rect)).expect("a source node"));
}

/// The placement of each View picture the next frame for `output` carries.
fn view_layers(composer: &mut Composer, output: OutputId) -> Vec<Rect> {
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(surface) if matches!(surface.source, SurfaceSource::Snapshot(_)) => Some(surface.dst),
            _ => None,
        })
        .collect()
}

#[test]
fn a_view_with_a_source_draws_its_picture_where_it_sits() {
    let (mut composer, output) = screen();
    let source = NodeId::new(2);
    add_source(&mut composer, source, Rect::new(0, 0, 40, 30));
    add_view(&mut composer, NodeId::new(1), None, Some(source));
    assert_eq!(view_layers(&mut composer, output), vec![Rect::new(10, 20, 40, 30)]);
}

#[test]
fn a_view_without_a_source_draws_no_picture() {
    let (mut composer, output) = screen();
    add_view(&mut composer, NodeId::new(1), None, None);
    assert!(view_layers(&mut composer, output).is_empty());
}

#[test]
fn a_view_inside_its_own_source_draws_no_picture() {
    let (mut composer, output) = screen();
    let source = NodeId::new(2);
    add_source(&mut composer, source, Rect::new(0, 0, 40, 30));
    add_view(&mut composer, NodeId::new(1), Some(source), Some(source));
    assert!(view_layers(&mut composer, output).is_empty());
}

#[test]
fn a_view_redraws_only_when_its_source_changes() {
    let (mut composer, output) = screen();
    let source = NodeId::new(2);
    add_source(&mut composer, source, Rect::new(0, 0, 40, 30));
    add_view(&mut composer, NodeId::new(1), None, Some(source));
    assert!(
        damage_of(&mut composer, output).is_some(),
        "a new view damages its pixels"
    );
    assert!(
        damage_of(&mut composer, output).is_none(),
        "an unchanged view draws nothing new"
    );
    composer.with_scene(|scene| {
        scene
            .set_bounds(source, Rect::new(0, 0, 20, 20))
            .expect("the source resizes")
    });
    assert!(
        damage_of(&mut composer, output).is_some(),
        "a source change re-renders the view"
    );
}

#[test]
fn a_view_of_a_window_redraws_when_the_window_node_is_marked_dirty() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(0, 0, 40, 50));
    composer.set_window_content(window, Some(content()));
    let node = composer.window_node(window).expect("the window's scene node");
    add_view(&mut composer, NodeId::new(1), None, Some(node));
    assert!(
        damage_of(&mut composer, output).is_some(),
        "a new view damages its pixels"
    );
    assert!(
        damage_of(&mut composer, output).is_none(),
        "an unchanged view draws nothing new"
    );
    composer.with_scene(|scene| scene.mark_dirty(node).expect("the window node"));
    assert!(
        damage_of(&mut composer, output).is_some(),
        "a new window buffer re-renders the view"
    );
}

#[test]
fn a_window_with_no_picture_to_keep_is_removed_at_once() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.close_window(window);
    assert!(
        !composer.windows.contains_key(&window),
        "a window with no buffer to read has nothing to fade"
    );
}

#[test]
fn a_closing_window_is_drawn_from_its_picture() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.close_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Remove,
        }),
    );
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(composer.compose(output, &mut frame));
    composer.end_frame();
    let drawn = frame.layers().iter().any(|layer| {
        matches!(
            layer,
            Layer::Surface(Surface {
                source: SurfaceSource::Snapshot(_),
                ..
            })
        )
    });
    assert!(
        drawn,
        "the closing window is drawn from the picture it kept, not a buffer it no longer has"
    );
}

#[test]
fn a_closing_window_is_repainted_as_its_fade_runs() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    damage_of(&mut composer, output);
    composer.close_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Remove,
        }),
    );
    damage_of(&mut composer, output);
    composer.advance_motion(MOTION_MS / 2);
    let rects = damage_of(&mut composer, output).expect("the fade repaints the pixels it covers");
    assert!(
        rects.iter().any(|rect| rect.contains(Point::new(20, 30))),
        "the fading window's rectangle is repainted: {rects:?}"
    );
}

#[test]
fn a_closing_window_is_removed_once_its_fade_ends() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.close_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Remove,
        }),
    );
    composer.advance_motion(MOTION_MS / 2);
    assert!(composer.windows.contains_key(&window), "the window is still fading");
    composer.advance_motion(MOTION_MS);
    assert!(
        !composer.windows.contains_key(&window),
        "the window is gone once its fade ends"
    );
}

// A client that unmaps (commits a null buffer) keeps its window. Its last picture fades
// out, then only the node goes: the window stays known, so a re-map needs no new identity.
#[test]
fn an_unmapped_window_fades_then_loses_its_node() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.unmap_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(composer.compose(output, &mut frame));
    composer.end_frame();
    assert!(
        frame.layers().iter().any(|layer| matches!(
            layer,
            Layer::Surface(Surface {
                source: SurfaceSource::Snapshot(_),
                ..
            })
        )),
        "the unmapped window fades from the picture it kept"
    );
    composer.advance_motion(MOTION_MS / 2);
    assert!(composer.window_matrix(window).is_some(), "still fading");
    composer.advance_motion(MOTION_MS);
    assert!(composer.windows.contains_key(&window), "the window is still known");
    assert!(
        composer.window_matrix(window).is_none(),
        "its node is gone once the fade ends"
    );
    assert!(composer.tracks.is_empty(), "no track outlives the fade");
}

/// The opacity of the snapshot surface a frame draws for a closing window, after a fade
/// tick that moves the node so the frame has damage to draw.
fn closing_snapshot_opacity(surface_alpha: Option<f32>) -> f32 {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.unmap_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.advance_motion(MOTION_MS / 2);
    if let Some(factor) = surface_alpha {
        composer.set_window_surface_alpha(window, factor);
    }
    composer.advance_motion(MOTION_MS / 4);
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(composer.compose(output, &mut frame), "the fade tick damages the frame");
    composer.end_frame();
    frame
        .layers()
        .iter()
        .find_map(|layer| match layer {
            Layer::Surface(Surface {
                source: SurfaceSource::Snapshot(_),
                opacity,
                ..
            }) => Some(*opacity),
            _ => None,
        })
        .expect("a closing window draws its snapshot")
}

#[test]
fn a_surface_factor_set_during_a_close_fade_does_not_change_the_fade() {
    assert_eq!(
        closing_snapshot_opacity(Some(0.5)),
        closing_snapshot_opacity(None),
        "a closing window keeps the opacity its fade drew, as its placement and shell already do"
    );
}

#[test]
fn an_unmapped_window_with_no_picture_is_hidden_at_once() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.unmap_with_snapshot(window, None);
    assert!(composer.window_matrix(window).is_none(), "nothing to fade, so no node");
    assert!(composer.windows.contains_key(&window));
}

// A minimised window fades out like an unmapped one, but keeps its picture and content so
// that restoring it can ease back. Its node goes when the fade ends; the window stays known.
#[test]
fn a_minimised_window_fades_out_then_loses_its_node_and_keeps_its_content() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.hide_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    assert!(
        !composer.windows[&window].visible,
        "the window is hidden from the start"
    );
    composer.advance_motion(MOTION_MS / 2);
    assert!(composer.window_matrix(window).is_some(), "still fading");
    composer.advance_motion(MOTION_MS * 2);
    assert!(composer.windows.contains_key(&window), "the window is still known");
    assert!(
        composer.window_matrix(window).is_none(),
        "its node is gone once the fade ends"
    );
    assert!(
        composer.windows[&window].content.is_some(),
        "its content is kept to restore"
    );
    assert!(composer.tracks.is_empty(), "no track outlives the fade");
}

// Showing a minimised window mid-fade stops the fade on the node it kept: the node eases
// back to full opacity and is not removed when the old fade would have ended.
#[test]
fn a_minimised_window_restored_mid_fade_eases_back_to_opaque() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    let node = composer.windows[&window].node.expect("a node");
    composer.hide_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.advance_motion(MOTION_MS / 2);
    composer.set_window_visible(window, true);
    assert!(composer.windows[&window].closing.is_none(), "the fade is stopped");
    assert_eq!(composer.windows[&window].node, Some(node), "the same node");
    composer.advance_motion(MOTION_MS * 2);
    assert!(composer.windows.contains_key(&window));
    assert_eq!(
        composer.windows[&window].node,
        Some(node),
        "the node survives the old fade's end"
    );
    assert!(composer.tracks.is_empty(), "the stopped fade left no track");
    assert_eq!(
        composer.scene.transform(node).map(|transform| transform.opacity),
        Some(1.0),
        "the window is opaque again"
    );
}

// With motion reduced there is no picture to fade from, so the window is hidden at once.
#[test]
fn a_minimised_window_with_reduced_motion_is_hidden_at_once() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_reduced_motion(true);
    composer.set_window_visible(window, false);
    assert!(composer.window_matrix(window).is_none(), "no node once hidden");
    assert!(composer.windows[&window].content.is_some(), "the content is kept");
}

// A client that keeps committing while its window is minimised must not revive it: only a
// window the workspace shows again can stop its fade.
#[test]
fn a_commit_during_a_minimise_fade_does_not_revive_the_window() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.hide_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.advance_motion(MOTION_MS / 2);
    composer.set_window_content(window, Some(content()));
    assert!(composer.windows[&window].closing.is_some(), "the fade continues");
    composer.advance_motion(MOTION_MS * 2);
    assert!(composer.window_matrix(window).is_none(), "the window stays hidden");
}

// A window hidden while it is being destroyed still goes: the destroy's end removes it.
#[test]
fn a_destroy_during_a_minimise_fade_removes_the_window_when_it_ends() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.hide_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.close_window(window);
    composer.advance_motion(MOTION_MS * 2);
    assert!(
        !composer.windows.contains_key(&window),
        "the destroyed window is forgotten"
    );
}

// A window that changes size while minimised is drawn at its new size once shown again.
#[test]
fn a_window_resized_while_minimised_is_drawn_at_its_new_size_when_restored() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.hide_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    let bigger = Content {
        src: RectF::new(0.0, 0.0, 80.0, 90.0),
        size: Size::new(80, 90),
        transform: Transform::Normal,
    };
    composer.set_window_content(window, Some(bigger));
    composer.advance_motion(MOTION_MS * 2);
    assert!(composer.window_matrix(window).is_none(), "still hidden");
    composer.set_window_visible(window, true);
    assert_eq!(
        drawn_pose(&composer, window).0,
        Rect::new(20, 30, 80, 90),
        "the new size"
    );
}

// Mapping again during the fade cancels it: the node is the same one, it eases back to
// full opacity, and the fade's end does not remove the window.
#[test]
fn a_window_that_maps_again_while_fading_cancels_the_fade() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    let node = composer.windows[&window].node.expect("a node");
    composer.unmap_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.advance_motion(MOTION_MS / 2);
    composer.set_window_content(window, Some(content()));
    assert!(composer.windows[&window].closing.is_none(), "the picture is dropped");
    assert_eq!(composer.windows[&window].node, Some(node), "the same node");
    composer.advance_motion(MOTION_MS * 2);
    assert!(
        composer.windows.contains_key(&window),
        "the old fade's end keeps the window"
    );
    assert_eq!(composer.windows[&window].node, Some(node));
    assert!(composer.tracks.is_empty(), "the cancelled fade left no track");
    assert_eq!(
        composer.scene.transform(node).map(|transform| transform.opacity),
        Some(1.0),
        "the window is opaque again"
    );
}

// A destroy that arrives during an unmap fade must not cut the fade short: the picture is
// still on screen, so it finishes and then the window is removed.
#[test]
fn a_destroy_during_an_unmap_fade_finishes_the_fade_then_removes() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.unmap_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.advance_motion(MOTION_MS / 2);
    composer.close_window(window);
    assert!(composer.window_matrix(window).is_some(), "the fade is not cut short");
    composer.advance_motion(MOTION_MS);
    assert!(!composer.windows.contains_key(&window), "removed once the fade ends");
    assert!(composer.tracks.is_empty());
}

// Unmapped with nothing to fade, then destroyed: the entry is removed, not kept forever.
#[test]
fn an_unmapped_window_destroyed_later_is_forgotten() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.unmap_with_snapshot(window, None);
    composer.close_window(window);
    assert!(
        !composer.windows.contains_key(&window),
        "a destroyed window is forgotten"
    );
    assert!(composer.order.is_empty(), "and it leaves the stacking order");
}

// An X11 window is inserted again while its unmap fade runs. The fade stops, and since the
// window has no picture yet it keeps no node, so nothing is drawn as a ghost.
#[test]
fn an_unmapped_window_inserted_again_while_fading_stops_the_fade() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.unmap_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Hide,
        }),
    );
    composer.advance_motion(MOTION_MS / 2);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    assert!(!composer.is_fading(window), "the fade stops");
    assert!(composer.tracks.is_empty(), "the stopped fade left no track");
    assert_eq!(composer.windows[&window].node, None, "no picture, so no node");
    composer.advance_motion(MOTION_MS * 2);
    assert!(composer.windows.contains_key(&window), "the window is still known");
    assert_eq!(composer.windows[&window].node, None, "and still drawn as nothing");
}
// A client that unmaps a window with no picture to keep hides it through the public entry
// point, and an X11 window that maps again reuses the same entry: one slot, one entry, no
// leak from the repeated inserts.
#[test]
fn unmap_and_remap_without_a_picture_keeps_one_entry() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.unmap_window(window);
    assert!(composer.windows.contains_key(&window), "the window stays known");
    assert!(!composer.is_fading(window), "there was no picture to fade");
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    assert_eq!(composer.order, vec![window], "one stacking slot");
    assert_eq!(composer.windows.len(), 1, "one entry");
}

// An unmap that arrives after a destroy must not turn the destroy into a hide: the window
// still leaves when its fade ends.
#[test]
fn an_unmap_during_a_destroy_fade_does_not_hide_the_window() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.close_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Remove,
        }),
    );
    composer.unmap_window(window);
    assert_eq!(
        composer.windows[&window].closing.as_ref().map(|closing| closing.on_end),
        Some(CloseEnd::Remove),
        "the destroy still decides the end"
    );
    composer.advance_motion(MOTION_MS * 2);
    assert!(
        !composer.windows.contains_key(&window),
        "the destroyed window is removed"
    );
}

// A commit that lands after a destroy fades the old picture and does not bring the window
// back: the client cannot show a window it has destroyed.
#[test]
fn a_commit_during_a_destroy_fade_does_not_revive_the_window() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.close_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Remove,
        }),
    );
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        composer.windows[&window].closing.as_ref().map(|closing| closing.on_end),
        Some(CloseEnd::Remove),
        "the fade is not cancelled"
    );
    composer.advance_motion(MOTION_MS * 2);
    assert!(
        !composer.windows.contains_key(&window),
        "the window is removed at the fade's end"
    );
    assert!(composer.tracks.is_empty(), "no track outlives its window");
}

#[test]
fn moving_a_window_damages_both_rectangles() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    damage_of(&mut composer, output);
    composer.place_window(window, Rect::new(200, 300, 40, 50));
    let rects = damage_of(&mut composer, output).expect("both rectangles");
    assert_eq!(rects.len(), 2, "the old and the new rectangle: {rects:?}");
    assert!(rects.contains(&Rect::new(20, 30, 40, 50)));
    assert!(rects.contains(&Rect::new(200, 300, 40, 50)));
}

#[test]
fn moving_a_translucent_rounded_window_damages_both_rectangles() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_window_opacity(window, 0.5);
    composer.set_window_corner_radius(window, 4.0);
    damage_of(&mut composer, output);
    composer.place_window(window, Rect::new(200, 300, 40, 50));
    let rects = damage_of(&mut composer, output).expect("both rectangles");
    assert_eq!(rects.len(), 2, "the old and the new rectangle: {rects:?}");
    assert!(rects.contains(&Rect::new(20, 30, 40, 50)));
    assert!(rects.contains(&Rect::new(200, 300, 40, 50)));
}

#[test]
fn a_commit_damages_the_window_it_drew_into() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    damage_of(&mut composer, output);
    // The same content again: the node does not move, the client still drew.
    composer.set_window_content(window, Some(content()));
    let rects = damage_of(&mut composer, output).expect("a commit damages the window");
    assert!(
        rects.iter().any(|rect| rect.contains(Point::new(30, 40))),
        "a commit damages the window's rectangle: {rects:?}"
    );
}

#[test]
fn a_second_output_damages_its_own_coordinates() {
    let mut composer = stock_composer();
    let first = OutputId::new(0);
    let second = OutputId::new(1);
    composer.set_output(first, Rect::new(0, 0, 800, 600));
    composer.set_output(second, Rect::new(800, 0, 800, 600));
    composer.begin_frame();
    composer.frame_asked(first);
    composer.frame_asked(second);
    composer.end_frame();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(810, 20, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(composer.compose(second, &mut frame));
    assert_eq!(
        frame.damage(),
        &[Rect::new(10, 20, 40, 50)],
        "a frame is output-local, so the output's origin is subtracted"
    );
    let mut first_frame = OutputFrame::new();
    assert!(
        !composer.compose(first, &mut first_frame),
        "the first output did not change"
    );
    composer.end_frame();
}

#[test]
fn a_hidden_window_keeps_no_pixels_to_move() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_window_visible(window, false);
    damage_of(&mut composer, output);
    composer.place_window(window, Rect::new(200, 300, 40, 50));
    assert_eq!(
        damage_of(&mut composer, output),
        None,
        "a hidden window has no pixels to move"
    );
}

#[test]
fn a_rotated_window_is_not_drawn_as_a_bounding_box() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_window_rotation(window, 90.0);
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(composer.compose(output, &mut frame));
    assert!(
        frame.layers().is_empty(),
        "the rotation is refused rather than drawn at the wrong angle"
    );
    assert!(
        frame.damage().iter().any(|rect| rect.contains(Point::new(30, 40))),
        "the scene still damages the rotated bounds"
    );
    composer.end_frame();
}

#[test]
fn opacity_is_clamped_rather_than_poisoned() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(0, 0, 10, 10));
    composer.set_window_opacity(window, f32::NAN);
    assert_eq!(composer.windows[&window].transform.opacity, 1.0);
    composer.set_window_opacity(window, 4.0);
    assert_eq!(composer.windows[&window].transform.opacity, 1.0);
    composer.set_window_opacity(window, -1.0);
    assert_eq!(composer.windows[&window].transform.opacity, 0.0);
}

#[test]
fn shell_opacity_multiplies_the_placement_opacity_and_survives_placement() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(100, 100, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_reduced_motion(true);
    composer.set_window_opacity(window, 0.5);
    composer.set_shell_opacity(window, 0.5).expect("0.5 is in range");
    assert_eq!(
        drawn_pose(&composer, window).1,
        0.25,
        "the shell opacity multiplies the placement's"
    );
    composer.set_window_opacity(window, 0.8);
    assert_eq!(
        drawn_pose(&composer, window).1,
        0.4,
        "a placement write does not clobber the shell opacity"
    );
}

#[test]
fn shell_offset_and_scale_survive_placement_and_reset_restores_the_window() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(100, 100, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_reduced_motion(true);
    composer.set_shell_offset(window, 10.0, -20.0).expect("in range");
    assert_eq!(drawn_pose(&composer, window).0, Rect::new(110, 80, 40, 50));
    composer.place_window_animated(window, Rect::new(200, 200, 40, 50), gnoblin_types::Transform::IDENTITY);
    assert_eq!(
        drawn_pose(&composer, window).0,
        Rect::new(210, 180, 40, 50),
        "a placement moves the window and the shell offset rides on top"
    );
    composer.set_shell_scale(window, 2.0).expect("in range");
    assert_eq!(
        drawn_pose(&composer, window).0,
        Rect::new(190, 155, 80, 100),
        "a shell scale of two is about the window's centre"
    );
    composer.set_shell_opacity(window, 0.5).expect("in range");
    composer.reset_shell_visuals(window);
    assert_eq!(
        drawn_pose(&composer, window),
        (Rect::new(200, 200, 40, 50), 1.0),
        "reset puts the window back at its placement, fully opaque"
    );
}

#[test]
fn shell_visual_values_out_of_range_are_refused_and_leave_the_window_unchanged() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(0, 0, 10, 10));
    assert_eq!(
        composer.set_shell_opacity(window, 1.5),
        Err(ShellVisualError::OutOfRange)
    );
    assert_eq!(
        composer.set_shell_opacity(window, f32::NAN),
        Err(ShellVisualError::OutOfRange)
    );
    assert_eq!(
        composer.set_shell_scale(window, 0.001),
        Err(ShellVisualError::OutOfRange)
    );
    assert_eq!(
        composer.set_shell_scale(window, 10.5),
        Err(ShellVisualError::OutOfRange)
    );
    assert_eq!(
        composer.set_shell_offset(window, 100_001.0, 0.0),
        Err(ShellVisualError::OutOfRange)
    );
    assert_eq!(
        composer.set_shell_offset(window, 0.0, f64::NAN),
        Err(ShellVisualError::OutOfRange)
    );
    let node = &composer.windows[&window];
    assert_eq!(
        (node.shell_opacity, node.shell_scale, node.shell_offset),
        (1.0, 1.0, (0.0, 0.0))
    );
    assert!(
        composer.set_shell_opacity(window, 0.0).is_ok(),
        "the documented bounds are accepted"
    );
    assert!(composer.set_shell_scale(window, 10.0).is_ok());
    assert!(composer.set_shell_offset(window, -100_000.0, 100_000.0).is_ok());
}

/// The rectangle and opacity `window` is drawn at: its node's placed rectangle and the opacity of its
/// transform, which are what the frame's layer for the window is built from.
fn drawn_pose(composer: &Composer, window: WindowId) -> (Rect, f32) {
    let pose = composer.pose(window).expect("a window with content has a node");
    let node = composer.window_node(window).expect("a window with content has a node");
    let bounds = composer
        .scene
        .content_bounds(node)
        .expect("a node has bounds")
        .to_pixel();
    (bounds, pose.transform.opacity)
}

/// Starts a linear track of `property` on `window`'s node, lasting `duration_ms`.
fn track_window(
    composer: &mut Composer,
    window: WindowId,
    property: VisualProperty,
    to: PropertyValue,
    duration_ms: u32,
) {
    let node = composer.windows[&window].node.expect("a drawn window has a node");
    let spec = TrackSpec {
        from: TrackStart::Current,
        duration_ms,
        easing: Easing::Linear,
        ..TrackSpec::new(node, property, to)
    };
    composer
        .motion
        .start(&mut composer.scene, spec)
        .expect("the track starts");
}

#[test]
fn a_tweened_opacity_is_the_opacity_the_frame_draws() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    // A mapped window fades in from transparent; let that fade land so the tween
    // under test starts from an opaque window.
    composer.advance_motion(200);
    track_window(
        &mut composer,
        window,
        VisualProperty::Opacity,
        PropertyValue::Number(0.5),
        200,
    );
    composer.advance_motion(100);
    assert_eq!(
        drawn_pose(&composer, window),
        (Rect::new(20, 30, 40, 50), 0.75),
        "the frame draws the tween's pose, halfway from 1.0 to 0.5, not its target"
    );
}

#[test]
fn a_window_that_maps_fades_in_from_transparent() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        drawn_pose(&composer, window),
        (Rect::new(20, 30, 40, 50), 0.0),
        "a newly mapped window starts transparent"
    );
    composer.advance_motion(100);
    let (_, halfway) = drawn_pose(&composer, window);
    assert!(
        halfway > 0.0 && halfway < 1.0,
        "the fade is part way through at half its duration, got {halfway}"
    );
    composer.advance_motion(100);
    assert_eq!(
        drawn_pose(&composer, window).1,
        1.0,
        "the fade lands on the window's opacity exactly"
    );
}

#[test]
fn a_window_that_maps_with_reduced_motion_is_opaque_at_once() {
    let (mut composer, _output) = screen();
    composer.set_reduced_motion(true);
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        drawn_pose(&composer, window),
        (Rect::new(20, 30, 40, 50), 1.0),
        "reduced motion writes the target at once"
    );
}

#[test]
fn a_composer_nobody_set_motion_on_is_instant() {
    let composer = Composer::new();
    assert_eq!(composer.motion_policy(), MotionPolicy::INSTANT);
    assert_eq!(MotionPolicy::default(), MotionPolicy::INSTANT);
}

#[test]
fn a_core_default_composer_maps_a_window_opaque_and_in_place_at_once_without_any_setting() {
    let mut composer = Composer::new();
    let output = OutputId::new(0);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    assert_eq!(drawn_pose(&composer, window), (Rect::new(20, 30, 40, 50), 1.0));
}

#[test]
fn an_instant_motion_policy_maps_a_window_opaque_and_in_place_at_once() {
    let (mut composer, _output) = screen();
    composer.setting_set(Setting::Motion(MotionPolicy::INSTANT));
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        drawn_pose(&composer, window),
        (Rect::new(20, 30, 40, 50), 1.0),
        "a zero map fade writes the target at once, with motion not reduced"
    );
}

#[test]
fn an_instant_motion_policy_lands_a_placement_at_once() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.setting_set(Setting::Motion(MotionPolicy::INSTANT));
    composer.place_window_animated(window, Rect::new(200, 120, 40, 50), gnoblin_types::Transform::IDENTITY);
    assert_eq!(
        drawn_pose(&composer, window),
        (Rect::new(200, 120, 40, 50), 1.0),
        "a zero retarget length moves the window to its target in the same frame"
    );
}

#[test]
fn a_placement_takes_the_policys_retarget_length() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.setting_set(Setting::Motion(MotionPolicy {
        retarget_ms: 100,
        ..MotionPolicy::default()
    }));
    composer.place_window_animated(window, Rect::new(220, 30, 40, 50), gnoblin_types::Transform::IDENTITY);
    composer.advance_motion(50);
    let (halfway, _) = drawn_pose(&composer, window);
    assert!(
        halfway.x > 20 && halfway.x < 220,
        "part way after half the length: {halfway:?}"
    );
    composer.advance_motion(50);
    assert_eq!(drawn_pose(&composer, window).0.x, 220, "landed after the whole 100 ms");
}

#[test]
fn a_closing_window_fades_for_the_policys_unmap_length() {
    let (mut composer, _) = screen();
    composer.setting_set(Setting::Motion(MotionPolicy {
        unmap_fade_ms: 400,
        ..MotionPolicy::default()
    }));
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.advance_motion(MOTION_MS);
    composer.close_with_snapshot(
        window,
        Some(Closing {
            source: SurfaceSource::Snapshot(picture()),
            content: content(),
            on_end: CloseEnd::Remove,
        }),
    );
    composer.advance_motion(MOTION_MS);
    assert!(
        composer.windows.contains_key(&window),
        "still fading after the default length"
    );
    composer.advance_motion(MOTION_MS);
    assert!(
        !composer.windows.contains_key(&window),
        "gone after the 400 ms it was given"
    );
}

/// The opacity `window`'s surface is drawn at: the node's, which holds every factor the
/// window draws with, so this is what the frame draws.
fn drawn_opacity(composer: &Composer, window: WindowId) -> f32 {
    drawn_pose(composer, window).1
}

#[test]
fn the_four_opacity_factors_multiply_into_the_drawn_opacity() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(100, 100, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_reduced_motion(true);
    composer.set_window_opacity(window, 0.5);
    composer.set_shell_opacity(window, 0.5).expect("0.5 is in range");
    composer.set_window_surface_alpha(window, 0.5);
    composer.set_window_rule_opacity(window, 0.5);
    assert_eq!(
        drawn_opacity(&composer, window),
        0.0625,
        "the placement, shell, surface and rule opacities multiply"
    );
}

#[test]
fn a_factor_change_lands_at_once_and_a_tween_runs_under_the_factors() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    // Let the fade-in land so the window starts opaque.
    composer.advance_motion(200);
    composer.set_window_surface_alpha(window, 0.5);
    assert_eq!(drawn_opacity(&composer, window), 0.5, "a factor change lands at once");
    composer.set_window_rule_opacity(window, 0.5);
    assert_eq!(
        drawn_opacity(&composer, window),
        0.25,
        "the rule's factor lands at once"
    );
    track_window(
        &mut composer,
        window,
        VisualProperty::Opacity,
        PropertyValue::Number(0.0),
        200,
    );
    composer.advance_motion(100);
    assert_eq!(
        drawn_opacity(&composer, window),
        0.125,
        "halfway through the tween, under the factors"
    );
}

#[test]
fn a_window_that_maps_under_a_factor_fades_in_to_the_factored_opacity() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_surface_alpha(window, 0.5);
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        drawn_opacity(&composer, window),
        0.0,
        "a newly mapped window starts transparent"
    );
    composer.advance_motion(100);
    let halfway = drawn_opacity(&composer, window);
    assert!(
        halfway > 0.0 && halfway < 0.5,
        "the fade is part way through, got {halfway}"
    );
    composer.advance_motion(100);
    assert_eq!(
        drawn_opacity(&composer, window),
        0.5,
        "the fade lands on the factored opacity exactly"
    );
}

#[test]
fn a_tweened_offset_moves_the_drawn_rectangle() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    track_window(
        &mut composer,
        window,
        VisualProperty::OffsetX,
        PropertyValue::Number(220.0),
        200,
    );
    assert_eq!(
        drawn_pose(&composer, window).0.x,
        20,
        "before the tween the window is at its placement"
    );
    composer.advance_motion(100);
    assert_eq!(
        drawn_pose(&composer, window).0,
        Rect::new(120, 30, 40, 50),
        "halfway along the tween the window is drawn halfway from its placement to its target"
    );
    composer.advance_motion(100);
    assert_eq!(
        drawn_pose(&composer, window).0,
        Rect::new(220, 30, 40, 50),
        "the tween lands on its target"
    );
}

#[test]
fn a_scaled_window_is_drawn_at_its_scaled_bounds() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(100, 100, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_reduced_motion(true);
    let mut base = gnoblin_types::Transform::IDENTITY;
    base.scale = Vec3::new(2.0, 2.0, 1.0);
    composer.place_window_animated(window, Rect::new(100, 100, 40, 50), base);
    assert_eq!(
        drawn_pose(&composer, window),
        (Rect::new(80, 75, 80, 100), 1.0),
        "a scale of two is about the window's centre, so the rectangle grows by a quarter each side"
    );
}

#[test]
fn a_scale_tween_passes_through_intermediate_sizes() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(100, 100, 40, 50));
    composer.set_window_content(window, Some(content()));
    let mut base = gnoblin_types::Transform::IDENTITY;
    base.scale = Vec3::new(2.0, 2.0, 1.0);
    composer.place_window_animated(window, Rect::new(100, 100, 40, 50), base);
    composer.advance_motion(100);
    let (dst, _) = drawn_pose(&composer, window);
    assert!(
        dst.width > 40 && dst.width < 80,
        "mid-tween the window is between its two sizes, not at either: {dst:?}"
    );
    let centre_x = dst.x + i32::try_from(dst.width / 2).expect("a width fits in i32");
    assert!(
        (centre_x - 120).abs() <= 1,
        "the tween scales about the centre, which stays at x 120: {dst:?}"
    );
    composer.advance_motion(10_000);
    assert_eq!(
        drawn_pose(&composer, window).0,
        Rect::new(80, 75, 80, 100),
        "the tween lands on the scaled rectangle"
    );
}

#[test]
fn a_zero_or_negative_or_non_finite_scale_is_refused_and_the_window_keeps_its_size() {
    for bad in [
        Vec3::new(0.0, 2.0, 1.0),
        Vec3::new(f64::NAN, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ] {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 100, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_reduced_motion(true);
        let mut base = gnoblin_types::Transform::IDENTITY;
        base.scale = bad;
        composer.place_window_animated(window, Rect::new(100, 100, 40, 50), base);
        assert_eq!(
            drawn_pose(&composer, window),
            (Rect::new(100, 100, 40, 50), 1.0),
            "a scale the scene cannot draw is refused, so the window keeps its size"
        );
        let kept = composer.windows[&window].transform.base.scale;
        assert_eq!((kept.x, kept.y), (1.0, 1.0), "the previous scale is kept");
    }
}

#[test]
fn an_open_tween_draws_monotone_frames_from_transparent_to_opaque() {
    let (mut composer, _output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_window_opacity(window, 0.0);
    track_window(
        &mut composer,
        window,
        VisualProperty::Opacity,
        PropertyValue::Number(1.0),
        200,
    );
    let mut opacities = Vec::new();
    for _ in 0..5 {
        opacities.push(drawn_pose(&composer, window).1);
        composer.advance_motion(50);
    }
    assert_eq!(
        opacities,
        vec![0.0, 0.25, 0.5, 0.75, 1.0],
        "a linear open tween steps evenly"
    );
    assert!(
        opacities.windows(2).all(|pair| pair[0] <= pair[1]),
        "every frame is at least as opaque as the one before"
    );
}

#[test]
fn the_scene_paints_windows_in_the_stacking_order_above_the_compositor_root() {
    let (mut composer, _) = screen();
    let first = WindowId::new(1);
    let second = WindowId::new(2);
    composer.insert_window(first, Rect::new(0, 0, 10, 10));
    composer.set_window_content(first, Some(content()));
    composer.insert_window(second, Rect::new(5, 5, 10, 10));
    composer.set_window_content(second, Some(content()));
    composer.set_stacking(&[first, second]);
    let first_node = node_of(&composer, first);
    let second_node = node_of(&composer, second);
    let roots = composer.scene.root_ids().to_vec();
    let root = composer.scene.root().expect("the compositor root is set");
    assert_eq!(roots.first(), Some(&root), "the root paints first: {roots:?}");
    assert_eq!(
        roots.last(),
        Some(&first_node),
        "the topmost window paints last: {roots:?}"
    );
    assert_eq!(roots.len(), 3, "the root and both windows: {roots:?}");
    assert_eq!(
        roots.get(1),
        Some(&second_node),
        "the window below comes next: {roots:?}"
    );
}

#[test]
fn a_stacking_change_damages_only_the_windows_that_moved_in_the_order() {
    let (mut composer, output) = screen();
    let first = WindowId::new(1);
    let second = WindowId::new(2);
    composer.insert_window(first, Rect::new(0, 0, 10, 10));
    composer.set_window_content(first, Some(content()));
    composer.insert_window(second, Rect::new(5, 5, 10, 10));
    composer.set_window_content(second, Some(content()));
    damage_of(&mut composer, output);
    // The order already matches the insertion order, so nothing changes.
    composer.set_stacking(&[second, first]);
    assert_eq!(
        damage_of(&mut composer, output),
        None,
        "an unchanged stacking order damages nothing"
    );
    composer.set_stacking(&[first, second]);
    assert!(
        damage_of(&mut composer, output).is_some(),
        "a changed order damages the windows whose overlap changed"
    );
}

#[test]
fn a_frame_with_no_damage_is_not_drawn() {
    let (mut composer, output) = screen();
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    frame.push_damage(Rect::new(0, 0, 4, 4));
    assert!(!composer.compose(output, &mut frame));
    composer.end_frame();
    assert!(frame.is_empty(), "a frame with no damage keeps no stale description");
}

#[test]
fn damage_is_taken_once_per_iteration() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(0, 0, 10, 10));
    composer.set_window_content(window, Some(content()));
    composer.begin_frame();
    let mut first = OutputFrame::new();
    let mut second = OutputFrame::new();
    assert!(composer.compose(output, &mut first));
    assert!(
        composer.compose(output, &mut second),
        "a second output's call in the same iteration still sees its damage"
    );
    composer.end_frame();
    let mut third = OutputFrame::new();
    assert!(!composer.compose(output, &mut third), "the next iteration starts empty");
}

/// Whether some damage rectangle covers `rect` in full.
fn covers(damage: &[Rect], rect: Rect) -> bool {
    let right = i64::from(rect.x) + i64::from(rect.width);
    let bottom = i64::from(rect.y) + i64::from(rect.height);
    damage.iter().any(|outer| {
        let outer_right = i64::from(outer.x) + i64::from(outer.width);
        let outer_bottom = i64::from(outer.y) + i64::from(outer.height);
        outer.x <= rect.x && outer.y <= rect.y && outer_right >= right && outer_bottom >= bottom
    })
}

#[test]
fn moving_the_cursor_damages_both_rectangles() {
    let (mut composer, output) = screen();
    let before = Rect::new(10, 10, 24, 24);
    let after = Rect::new(100, 40, 24, 24);
    composer.set_cursor(Some(before), false);
    assert!(
        damage_of(&mut composer, output).is_some(),
        "a cursor that appears is drawn"
    );
    composer.set_cursor(Some(after), false);
    let damage = damage_of(&mut composer, output).expect("a moved cursor is drawn");
    assert!(covers(&damage, before), "the old cursor is erased");
    assert!(covers(&damage, after), "the new cursor is painted");
}

#[test]
fn a_cursor_that_stays_put_damages_nothing() {
    let (mut composer, output) = screen();
    composer.set_cursor(Some(Rect::new(10, 10, 24, 24)), false);
    assert!(damage_of(&mut composer, output).is_some());
    composer.set_cursor(Some(Rect::new(10, 10, 24, 24)), false);
    assert!(damage_of(&mut composer, output).is_none(), "no motion, no damage");
}

#[test]
fn a_shape_change_at_the_same_place_redraws_the_cursor() {
    let (mut composer, output) = screen();
    let at = Rect::new(10, 10, 24, 24);
    composer.set_cursor(Some(at), false);
    assert!(damage_of(&mut composer, output).is_some());
    composer.set_cursor(Some(at), true);
    let damage = damage_of(&mut composer, output).expect("a new shape is drawn");
    assert!(covers(&damage, at));
}

#[test]
fn damage_of_an_output_nobody_asked_a_frame_of_is_kept_for_the_next_iteration() {
    let (mut composer, output) = screen();
    let first = Rect::new(10, 10, 24, 24);
    let second = Rect::new(100, 40, 24, 24);
    let third = Rect::new(300, 200, 24, 24);
    composer.set_cursor(Some(first), false);
    assert!(damage_of(&mut composer, output).is_some(), "the first cursor is drawn");
    // The pointer moves while no frame is asked of the output (a nested session whose parent is behind): what
    // changed is not drawn and nobody is told of it again.
    composer.set_cursor(Some(second), false);
    composer.begin_frame();
    composer.end_frame();
    composer.set_cursor(Some(third), false);
    let damage = damage_of(&mut composer, output).expect("the next frame is drawn");
    assert!(
        covers(&damage, first),
        "the cursor that is still on the screen is erased: {damage:?}"
    );
    assert!(
        covers(&damage, second),
        "and so is the one that was never drawn: {damage:?}"
    );
    assert!(covers(&damage, third), "and the new one is painted: {damage:?}");
}

#[test]
fn damage_of_an_output_that_was_asked_a_frame_of_is_spent() {
    let (mut composer, output) = screen();
    composer.set_cursor(Some(Rect::new(10, 10, 24, 24)), false);
    assert!(damage_of(&mut composer, output).is_some());
    assert_eq!(
        damage_of(&mut composer, output),
        None,
        "a frame that was drawn is not drawn again"
    );
}

#[test]
fn a_frame_asked_another_way_spends_the_damage_too() {
    let (mut composer, output) = screen();
    composer.set_cursor(Some(Rect::new(10, 10, 24, 24)), false);
    // The lock's cover and a mirror describe the output without `compose`; they say so.
    composer.begin_frame();
    composer.frame_asked(output);
    composer.end_frame();
    assert_eq!(damage_of(&mut composer, output), None);
}

#[test]
fn a_global_rectangle_becomes_local() {
    assert_eq!(
        local(Rect::new(810, 20, 40, 50), Rect::new(800, 0, 800, 600)),
        Rect::new(10, 20, 40, 50)
    );
    assert_eq!(
        local(Rect::new(-30, -10, 40, 50), Rect::new(-100, 0, 800, 600)),
        Rect::new(70, -10, 40, 50)
    );
}

/// The solid layers `output`'s next frame draws, as their rectangle and alpha.
fn solids_of(composer: &mut Composer, output: OutputId) -> Vec<(Rect, f32)> {
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Solid(solid) => Some((solid.dst, solid.colour.a)),
            Layer::Surface(_) => None,
            Layer::Overlay(_) => None,
        })
        .collect()
}

#[test]
fn a_flash_fades_out_over_its_duration() {
    let (mut composer, output) = screen();
    composer.flash_window(None);
    let first = solids_of(&mut composer, output);
    assert_eq!(first, vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)]);

    composer.advance_motion(FLASH_MS / 2);
    let halfway = solids_of(&mut composer, output);
    assert_eq!(halfway.len(), 1);
    assert!(
        (halfway[0].1 - FLASH_PEAK / 2.0).abs() < 1e-4,
        "half the peak after half the time"
    );
    assert!(composer.flash_running());

    composer.advance_motion(FLASH_MS);
    assert!(!composer.flash_running(), "the flash ends once its duration has passed");
    assert!(
        solids_of(&mut composer, output).is_empty(),
        "an ended flash draws nothing"
    );
}

#[test]
fn a_flash_damages_every_output_it_covers() {
    let (mut composer, output) = screen();
    composer.flash_window(None);
    let damage = damage_of(&mut composer, output).expect("a flash changes the output");
    assert!(
        covers(&damage, Rect::new(0, 0, 800, 600)),
        "the whole output is repainted"
    );
}

#[test]
fn the_lock_node_is_a_compositor_node_that_publishes_only_a_state_change() {
    let mut composer = stock_composer();
    let lock = composer.add_lock_node().expect("a lock node");
    let root = composer.with_scene(|scene| scene.root_ids()[0]);
    let (parent, class, state) = composer.with_scene(|scene| {
        (
            scene.parent(lock),
            scene.class(lock).map(str::to_owned),
            scene.text(lock, "state").map(str::to_owned),
        )
    });
    assert_eq!(parent, Some(root));
    assert_eq!(class.as_deref(), Some("lock"));
    assert_eq!(state.as_deref(), Some("not-locked"), "a new session is not locked");
    assert_eq!(composer.add_lock_node(), Some(lock), "one lock node");
    composer.take_tree_changes();

    composer.set_lock_state("locked");
    let changes = composer.take_tree_changes();
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Text(text) }
            if *node == lock && key == "state" && text == "locked"
    )));
    composer.set_lock_state("locked");
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged state is not republished"
    );
    assert!(
        composer
            .with_scene(|scene| scene.remove_as(Actor::Shell, lock))
            .is_err(),
        "the shell may not destroy the compositor's lock node"
    );
}

/// The lock asks for the lock screen once when it leaves `not-locked`: the shell is told to
/// engage, whether the lock passes through `pending` or goes straight to `locked`, and a
/// repeated publish does not ask again.
#[test]
fn the_lock_node_asks_the_shell_for_its_lock_screen_once_when_the_lock_leaves_not_locked() {
    let mut composer = stock_composer();
    let lock = composer.add_lock_node().expect("a lock node");
    assert!(composer.take_lock_requested().is_empty(), "no lock yet, nothing asked");

    composer.set_lock_state("pending");
    assert_eq!(composer.take_lock_requested(), vec![lock]);
    assert!(composer.take_lock_requested().is_empty(), "taken once");

    composer.set_lock_state("pending");
    assert!(
        composer.take_lock_requested().is_empty(),
        "an unchanged state asks nothing"
    );

    composer.set_lock_state("locked");
    assert!(composer.take_lock_requested().is_empty(), "locking asks nothing");
}

#[test]
fn the_lock_node_asks_once_when_the_lock_goes_straight_to_locked() {
    let mut composer = stock_composer();
    let lock = composer.add_lock_node().expect("a lock node");

    composer.set_lock_state("locked");
    assert_eq!(composer.take_lock_requested(), vec![lock]);

    composer.set_lock_state("not-locked");
    assert!(composer.take_lock_requested().is_empty(), "unlocking asks nothing");
}

/// F22: the lock unlocks while the shell is gone. The lock node follows the session-lock
/// state with no shell attached and no frame draining the changes, so the state the
/// shell reads on attach is the session's, not one the shell has to answer for.
#[test]
fn the_lock_node_returns_to_not_locked_when_the_session_unlocks_with_no_shell() {
    let mut composer = stock_composer();
    let lock = composer.add_lock_node().expect("a lock node");
    composer.set_lock_state("pending");
    composer.set_lock_state("locked");
    composer.take_tree_changes();

    composer.set_lock_state("not-locked");
    let changes = composer.take_tree_changes();
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Text(text) }
            if *node == lock && key == "state" && text == "not-locked"
    )));
    assert_eq!(
        composer
            .with_scene(|scene| scene.text(lock, "state").map(str::to_owned))
            .as_deref(),
        Some("not-locked"),
        "the scene reads the unlocked state"
    );
}

#[test]
fn the_main_seat_is_a_compositor_node_under_the_root() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    let root = composer.with_scene(|scene| scene.root_ids()[0]);
    let (parent, class, name) = composer.with_scene(|scene| {
        (
            scene.parent(seat),
            scene.class(seat).map(str::to_owned),
            scene.text(seat, "name").map(str::to_owned),
        )
    });
    let shell_remove = composer.with_scene(|scene| scene.remove_as(Actor::Shell, seat).is_err());
    assert_eq!(parent, Some(root));
    assert_eq!(class.as_deref(), Some("seat"));
    assert_eq!(name.as_deref(), Some("seat0"));
    assert!(shell_remove, "the shell may not destroy the compositor's seat");
    assert_eq!(composer.add_main_seat("seat0"), Some(seat), "one main seat");
    assert!(composer.take_tree_changes().iter().any(|change| matches!(
        change,
        TreeChange::Created { node, class, .. } if *node == seat && class == "seat"
    )));
}

#[test]
fn the_main_seat_publishes_its_window_management_policy() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    composer.with_scene(|scene| {
        for key in ["edge_tiling", "focus_mode", "auto_raise_delay", "right_click"] {
            assert_eq!(scene.value(seat, key), None, "{key}: the default is not stored yet");
        }
    });

    let mut policy = MainSeatPolicy {
        edge_tiling: true,
        drag_constraint: "minimum_strip",
        focus_mode: "hover",
        focus_new_windows: "prevent",
        raise_on_click: true,
        auto_raise: true,
        auto_raise_delay: 250,
        focus_change_on_pointer_rest: false,
        double_click: "toggle-maximize",
        middle_click: "lower",
        right_click: "menu",
        workspaces: vec!["1:Workspace 1".to_owned(), "2:Workspace 2".to_owned()],
    };
    composer.publish_main_seat_policy(&policy);
    composer.with_scene(|scene| {
        assert_eq!(scene.value(seat, "edge_tiling"), Some(&TreeValue::Bool(true)));
        assert_eq!(
            scene.value(seat, "workspaces"),
            Some(&TreeValue::ListText(vec![
                "1:Workspace 1".to_owned(),
                "2:Workspace 2".to_owned()
            ]))
        );
        assert_eq!(scene.text(seat, "drag_constraint"), Some("minimum_strip"));
        assert_eq!(scene.text(seat, "focus_mode"), Some("hover"));
        assert_eq!(scene.text(seat, "focus_new_windows"), Some("prevent"));
        assert_eq!(scene.value(seat, "raise_on_click"), Some(&TreeValue::Bool(true)));
        assert_eq!(scene.value(seat, "auto_raise"), Some(&TreeValue::Bool(true)));
        assert_eq!(scene.value(seat, "auto_raise_delay"), Some(&TreeValue::Int(250)));
        assert_eq!(
            scene.value(seat, "focus_change_on_pointer_rest"),
            Some(&TreeValue::Bool(false))
        );
        assert_eq!(scene.text(seat, "double_click"), Some("toggle-maximize"));
        assert_eq!(scene.text(seat, "middle_click"), Some("lower"));
        assert_eq!(scene.text(seat, "right_click"), Some("menu"));
    });
    let changes = composer.take_tree_changes();
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Bool(true) } if *node == seat && key == "edge_tiling"
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Int(250) } if *node == seat && key == "auto_raise_delay"
    )));

    composer.publish_main_seat_policy(&policy);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged policy is not republished"
    );

    policy.auto_raise_delay = 900;
    composer.publish_main_seat_policy(&policy);
    let changes = composer.take_tree_changes();
    assert_eq!(changes.len(), 1, "a reload publishes only the value it changed");
    assert!(matches!(
        &changes[0],
        TreeChange::Set { node, key, value: TreeValue::Int(900) } if *node == seat && key == "auto_raise_delay"
    ));
}

/// The main seat carries the reduced-motion setting the composer honours, so the stock shell
/// reads the same answer the compositor applies. Only a change is published.
#[test]
fn the_main_seat_publishes_reduced_motion_and_republishes_it_when_it_changes() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    let policy = MainSeatPolicy {
        edge_tiling: false,
        drag_constraint: "work_area",
        focus_mode: "click",
        focus_new_windows: "prevent",
        raise_on_click: true,
        auto_raise: false,
        auto_raise_delay: 500,
        focus_change_on_pointer_rest: false,
        double_click: "toggle-maximize",
        middle_click: "lower",
        right_click: "menu",
        workspaces: Vec::new(),
    };
    composer.publish_main_seat_policy(&policy);
    composer.with_scene(|scene| {
        assert_eq!(scene.value(seat, "reduced_motion"), Some(&TreeValue::Bool(false)));
    });
    composer.take_tree_changes();

    composer.set_reduced_motion(true);
    composer.publish_main_seat_policy(&policy);
    let changes = composer.take_tree_changes();
    assert_eq!(changes.len(), 1, "only the setting changed: {changes:?}");
    assert!(matches!(
        &changes[0],
        TreeChange::Set { node, key, value: TreeValue::Bool(true) } if *node == seat && key == "reduced_motion"
    ));

    composer.publish_main_seat_policy(&policy);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged setting is not republished"
    );
}

#[test]
fn the_main_seat_publishes_the_pointer_only_when_it_moves() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    composer.take_tree_changes();
    composer.set_main_seat_pointer(3.0, 4.0);
    let changes = composer.take_tree_changes();
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Double(v) } if *node == seat && key == "pointer_x" && *v == 3.0
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Double(v) } if *node == seat && key == "pointer_y" && *v == 4.0
    )));
    composer.set_main_seat_pointer(3.0, 4.0);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged pointer is not republished"
    );
}

#[test]
fn the_main_seat_publishes_its_devices_only_when_the_list_changes() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    composer.take_tree_changes();
    let list = vec!["1:Desk Keyboard:keyboard".to_owned(), "2:Mouse:pointer".to_owned()];
    composer.publish_main_seat_devices(list.clone());
    let changes = composer.take_tree_changes();
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::ListText(items) } if *node == seat && key == "device" && *items == list
    )));
    composer.with_scene(|scene| assert_eq!(scene.value(seat, "device"), Some(&TreeValue::ListText(list.clone()))));
    composer.publish_main_seat_devices(list);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged device list is not republished"
    );
    composer.publish_main_seat_devices(vec!["2:Mouse:pointer".to_owned()]);
    assert_eq!(composer.take_tree_changes().len(), 1, "a removal is published");
}

#[test]
fn the_main_seat_publishes_the_screen_under_the_pointer() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    composer.set_output(OutputId::new(0), Rect::new(0, 0, 800, 600));
    composer.set_output(OutputId::new(1), Rect::new(800, 0, 800, 600));
    let first = composer.screen_node(OutputId::new(0)).expect("the first screen");
    let second = composer.screen_node(OutputId::new(1)).expect("the second screen");
    let screen = |composer: &mut Composer| composer.with_scene(|scene| scene.reference(seat, "screen"));
    assert_eq!(
        screen(&mut composer),
        Some(first),
        "the first screen before any pointer move"
    );
    composer.set_main_seat_pointer(900.0, 100.0);
    assert_eq!(screen(&mut composer), Some(second), "the screen the pointer moved onto");
    assert!(composer.take_tree_changes().iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value: TreeValue::Node(Some(target)) }
            if *node == seat && key == "screen" && *target == second
    )));
    composer.set_main_seat_pointer(900.0, 100.0);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged screen is not republished"
    );
    composer.remove_output(OutputId::new(1));
    assert_eq!(
        screen(&mut composer),
        Some(first),
        "the first screen once the pointer's screen is gone"
    );
    composer.remove_output(OutputId::new(0));
    assert_eq!(screen(&mut composer), None, "no screen once every output is gone");
}

#[test]
fn the_main_seat_publishes_the_focused_window_node() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    composer.take_tree_changes();
    composer.set_main_seat_focus(Some(window));
    assert_eq!(composer.main_seat_focus(), Some(node));
    assert!(composer.take_tree_changes().iter().any(|change| matches!(
        change,
        TreeChange::Set { node: set, key, value: TreeValue::Node(Some(target)) }
            if *set == seat && key == "focus" && *target == node
    )));
    composer.set_main_seat_focus(None);
    assert_eq!(composer.main_seat_focus(), None);
    assert!(composer.take_tree_changes().iter().any(|change| matches!(
        change,
        TreeChange::Set { node: set, key, value: TreeValue::Node(None) } if *set == seat && key == "focus"
    )));
}

#[test]
fn a_flash_over_a_window_covers_only_that_window() {
    let (mut composer, output) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(100, 80, 200, 150));
    composer.flash_window(Some(window));
    assert_eq!(
        solids_of(&mut composer, output),
        vec![(Rect::new(100, 80, 200, 150), FLASH_PEAK)],
        "the flash is drawn over the window's rectangle alone"
    );
    composer.advance_motion(1);
    let damage = damage_of(&mut composer, output).expect("the fade repaints the window");
    assert!(covers(&damage, Rect::new(100, 80, 200, 150)));
    assert!(
        !covers(&damage, Rect::new(0, 0, 800, 600)),
        "the rest of the output is untouched"
    );
}

#[test]
fn a_flash_over_an_unknown_window_falls_back_to_every_output() {
    let (mut composer, output) = screen();
    composer.flash_window(Some(WindowId::new(99)));
    assert_eq!(
        solids_of(&mut composer, output),
        vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)]
    );
}

#[test]
fn a_reduced_motion_flash_is_drawn_once_at_its_peak() {
    let (mut composer, output) = screen();
    composer.set_reduced_motion(true);
    composer.flash_window(None);
    composer.advance_motion(FLASH_MS * 2);
    assert!(
        composer.flash_running(),
        "a reduced flash waits for the frame that draws it"
    );
    assert_eq!(
        solids_of(&mut composer, output),
        vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)],
        "reduced motion does not fade the flash"
    );
    assert!(!composer.flash_running(), "the frame that drew it ends it");
    assert!(solids_of(&mut composer, output).is_empty());
}

#[test]
fn a_new_flash_replaces_a_running_one() {
    let (mut composer, output) = screen();
    composer.flash_window(None);
    composer.advance_motion(FLASH_MS / 2);
    composer.flash_window(None);
    let restarted = solids_of(&mut composer, output);
    assert_eq!(
        restarted,
        vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)],
        "a bell restarts the fade"
    );
}

/// The window's node, which it has once content arrives.
fn node_of(composer: &Composer, window: WindowId) -> NodeId {
    composer.windows[&window].node.expect("the window has a node")
}

/// The x and y the scene draws the node at.
fn offset_of(composer: &Composer, node: NodeId) -> (f64, f64) {
    let transform = composer.scene.transform(node).expect("the node has a transform");
    (transform.base.offset.x, transform.base.offset.y)
}

#[test]
fn without_a_shell_the_window_manager_places_the_window() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.place_window(window, Rect::new(200, 120, 100, 100));
    let node = node_of(&composer, window);
    assert_eq!(offset_of(&composer, node), (200.0, 120.0));
}

#[test]
fn while_the_shell_holds_the_tree_the_composer_publishes_no_x_or_y() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    composer.with_scene(|scene| scene.set_holder_attached(true));
    composer.follow_placement();
    composer.take_tree_changes();
    // The window manager moves the window while the shell holds the tree: the shell's x and y
    // in the mirror must not be overwritten with the window manager's.
    composer.place_window(window, Rect::new(200, 120, 100, 100));
    let held = composer.take_tree_changes();
    assert_eq!(set_number(&held, node, "x"), None, "no x is published during the hold");
    assert_eq!(set_number(&held, node, "y"), None, "no y is published during the hold");
    // Once the shell lets go, the placed position is published, because the tree no longer
    // holds the value it last saw.
    composer.with_scene(|scene| scene.set_holder_attached(false));
    composer.set_reduced_motion(true);
    composer.follow_placement();
    let released = composer.take_tree_changes();
    assert_eq!(set_number(&released, node, "x"), Some(200.0));
    assert_eq!(set_number(&released, node, "y"), Some(120.0));
}

#[test]
fn a_shell_holding_the_tree_keeps_the_position_it_set() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    composer.with_scene(|scene| scene.set_holder_attached(true));
    composer.follow_placement();
    // The shell moves the node; the window manager's placement must not move it back.
    composer.with_scene(|scene| {
        let mut transform = *scene.transform(node).expect("the node has a transform");
        transform.base.offset.x = 333.0;
        transform.base.offset.y = 77.0;
        scene
            .set_transform(node, transform)
            .expect("the shell may set its node's transform");
    });
    composer.place_window(window, Rect::new(200, 120, 100, 100));
    assert_eq!(offset_of(&composer, node), (333.0, 77.0));
}

#[test]
fn the_window_manager_places_the_window_again_when_the_shell_lets_go() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    composer.with_scene(|scene| scene.set_holder_attached(true));
    composer.follow_placement();
    composer.place_window(window, Rect::new(200, 120, 100, 100));
    composer.with_scene(|scene| scene.set_holder_attached(false));
    // Reduced motion lands the placement at once, so the node's offset is the placed one.
    composer.set_reduced_motion(true);
    composer.follow_placement();
    assert_eq!(offset_of(&composer, node), (200.0, 120.0));
}

#[test]
fn a_window_gets_a_window_node_with_its_title_and_app_id() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_text(window, "Alpha", "org.gnoblin.Alpha");
    composer.set_window_content(window, Some(content()));
    let changes = composer.take_tree_changes();
    let node = node_of(&composer, window);
    assert!(
        changes.iter().any(|change| matches!(
            change,
            TreeChange::Created { node: created, class, .. } if *created == node && class == "window"
        )),
        "the node is created as a window: {changes:?}"
    );
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node: set, key, value: TreeValue::Text(text) }
            if *set == node && key == "title" && text == "Alpha"
    )));
    assert!(changes.iter().any(|change| matches!(
        change,
        TreeChange::Set { node: set, key, value: TreeValue::Text(text) }
            if *set == node && key == "app_id" && text == "org.gnoblin.Alpha"
    )));
}

/// The node a `parent_window` set named last, over the changes taken.
fn parent_published(changes: &[TreeChange], node: NodeId) -> Option<Option<NodeId>> {
    changes.iter().rev().find_map(|change| match change {
        TreeChange::Set {
            node: set,
            key,
            value: TreeValue::Node(target),
        } if *set == node && key == "parent_window" => Some(*target),
        _ => None,
    })
}

#[test]
fn a_dialog_publishes_its_parents_node_once_both_have_nodes() {
    let (mut composer, _) = screen();
    let (parent, dialog) = (WindowId::new(1), WindowId::new(2));
    composer.insert_window(parent, Rect::new(10, 10, 100, 100));
    composer.insert_window(dialog, Rect::new(20, 20, 50, 50));
    // The relation is known before either window has content: nothing is published until the nodes exist.
    composer.set_window_parent(dialog, Some(parent));
    composer.set_window_content(dialog, Some(content()));
    let dialog_node = node_of(&composer, dialog);
    let changes = composer.take_tree_changes();
    assert_eq!(
        parent_published(&changes, dialog_node),
        None,
        "the parent has no node yet"
    );
    composer.set_window_content(parent, Some(content()));
    let parent_node = node_of(&composer, parent);
    let changes = composer.take_tree_changes();
    assert_eq!(
        parent_published(&changes, dialog_node),
        Some(Some(parent_node)),
        "the dialog hears of its parent when the parent's node appears: {changes:?}"
    );
    assert_eq!(
        composer.scene.reference(dialog_node, "parent_window"),
        Some(parent_node)
    );
}

#[test]
fn a_dialog_with_a_node_and_a_parent_with_a_node_publishes_at_once_and_clears_with_the_parent() {
    let (mut composer, _) = screen();
    let (parent, dialog) = (WindowId::new(1), WindowId::new(2));
    for window in [parent, dialog] {
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
    }
    let (parent_node, dialog_node) = (node_of(&composer, parent), node_of(&composer, dialog));
    composer.take_tree_changes();
    composer.set_window_parent(dialog, Some(parent));
    let changes = composer.take_tree_changes();
    assert_eq!(parent_published(&changes, dialog_node), Some(Some(parent_node)));
    composer.set_window_parent(dialog, Some(parent));
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged relation publishes nothing"
    );
    composer.remove_window(parent);
    let changes = composer.take_tree_changes();
    assert_eq!(
        parent_published(&changes, dialog_node),
        Some(None),
        "a dialog whose parent closed has no parent: {changes:?}"
    );
    assert_eq!(composer.scene.reference(dialog_node, "parent_window"), None);
}

#[test]
fn a_window_marked_from_a_launcher_publishes_the_mark_once_it_has_a_node() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_from_launcher(window);
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    let changes = composer.take_tree_changes();
    let marks = |changes: &[TreeChange]| {
        changes
            .iter()
            .filter(|change| {
                matches!(change, TreeChange::Set { node: set, key, value: TreeValue::Bool(true) }
                    if *set == node && key == "from_launcher")
            })
            .count()
    };
    assert_eq!(marks(&changes), 1, "{changes:?}");
    composer.set_window_from_launcher(window);
    assert_eq!(marks(&composer.take_tree_changes()), 0, "the mark is published once");
}

#[test]
fn an_output_publishes_its_identity_mode_scale_and_primary_flag_only_when_they_change() {
    let mut composer = stock_composer();
    let output = OutputId::new(7);
    let info = ScreenInfo {
        make: "Gnoblin".to_owned(),
        model: "HEADLESS-1".to_owned(),
        mode: "800x600@60".to_owned(),
        scale: 1.0,
        supersample: 1.0,
        primary: true,
        connector: "HEADLESS-1".to_owned(),
        identity: String::new(),
        mirror_of: String::new(),
        enabled: true,
    };
    composer.set_output_info(output, &info);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an output with no screen node publishes nothing"
    );
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    composer.take_tree_changes();
    composer.set_output_info(output, &info);
    let node = composer.screen_node(output).expect("the output has a screen node");
    assert_eq!(composer.scene.text(node, "make"), Some("Gnoblin"));
    assert_eq!(composer.scene.text(node, "model"), Some("HEADLESS-1"));
    assert_eq!(composer.scene.text(node, "mode"), Some("800x600@60"));
    assert_eq!(composer.scene.number(node, "output_scale"), Some(1.0));
    assert_eq!(composer.scene.number(node, "supersample"), Some(1.0));
    assert_eq!(composer.scene.value(node, "primary"), Some(&TreeValue::Bool(true)));
    composer.take_tree_changes();
    composer.set_output_info(output, &info);
    assert!(composer.take_tree_changes().is_empty());
    let demoted = ScreenInfo { primary: false, ..info };
    composer.set_output_info(output, &demoted);
    assert_eq!(
        composer.take_tree_changes(),
        vec![TreeChange::Set {
            node,
            key: "primary".to_owned(),
            value: TreeValue::Bool(false),
        }]
    );

    let supersampled = ScreenInfo {
        supersample: 2.0,
        ..demoted
    };
    composer.set_output_info(output, &supersampled);
    assert_eq!(
        composer.take_tree_changes(),
        vec![TreeChange::Set {
            node,
            key: "supersample".to_owned(),
            value: TreeValue::Double(2.0),
        }],
        "the factor is published on the screen node when it changes"
    );
}

#[test]
fn a_title_change_is_published_only_when_it_changes() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.set_window_text(window, "Alpha", "app");
    composer.take_tree_changes();
    composer.set_window_text(window, "Alpha", "app");
    assert!(
        composer.take_tree_changes().is_empty(),
        "the same text publishes nothing"
    );
    composer.set_window_text(window, "Beta", "app");
    let changes = composer.take_tree_changes();
    assert_eq!(changes.len(), 1, "only the title changed: {changes:?}");
}

#[test]
fn a_window_state_is_published_only_when_it_changes() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.set_window_text(window, "Alpha", "app");
    composer.take_tree_changes();
    composer.set_window_state(window, [true, false, false, false]);
    assert_eq!(
        composer.take_tree_changes().len(),
        4,
        "focused, fullscreen, maximised and minimised are each set"
    );
    composer.set_window_state(window, [true, false, false, false]);
    assert!(
        composer.take_tree_changes().is_empty(),
        "the same state publishes nothing"
    );
    composer.set_window_state(window, [false, true, false, false]);
    assert_eq!(
        composer.take_tree_changes().len(),
        4,
        "a changed state publishes all four"
    );
}

#[test]
fn while_a_shell_holds_the_tree_only_focus_is_published_and_the_flags_are_read_from_the_node() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.take_tree_changes();
    let node = composer.window_node(window).expect("the window has a node");
    composer.with_scene(|scene| scene.set_holder_attached(true));
    assert_eq!(
        composer.shell_window_flags(window),
        Some([false; 3]),
        "nothing is set yet"
    );

    // The shell writes the three flags on the node itself.
    composer.with_scene(|scene| {
        for key in ["fullscreen", "maximised"] {
            scene
                .set_value(node, key, TreeValue::Bool(true))
                .expect("the node takes the value");
        }
    });
    assert_eq!(
        composer.shell_window_flags(window),
        Some([true, true, false]),
        "the flags are read from the node"
    );

    composer.set_window_state(window, [true, true, true, false]);
    let changes = composer.take_tree_changes();
    assert_eq!(changes.len(), 1, "only focus is published: {changes:?}");
    assert!(
        matches!(&changes[0], TreeChange::Set { key, value: TreeValue::Bool(true), .. } if key == "focused"),
        "{changes:?}"
    );

    composer.with_scene(|scene| scene.set_holder_attached(false));
    assert_eq!(
        composer.shell_window_flags(window),
        None,
        "with no shell the node holds nothing"
    );
}

/// The number a `set` of `key` on `node` wrote in `changes`, if one did.
fn set_number(changes: &[TreeChange], node: NodeId, key: &str) -> Option<f64> {
    changes.iter().find_map(|change| match change {
        TreeChange::Set {
            node: set,
            key: set_key,
            value: TreeValue::Double(value),
        } if *set == node && set_key == key => Some(*value),
        _ => None,
    })
}

#[test]
fn a_window_is_announced_ready_once_when_it_first_has_a_node() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    assert!(composer.take_tree_ready().is_empty(), "no content, no node, no ready");
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    assert_eq!(composer.take_tree_ready(), vec![node]);
    composer.set_window_content(window, Some(content()));
    assert!(composer.take_tree_ready().is_empty(), "a commit announces nothing new");
}

#[test]
fn a_window_node_publishes_its_position_and_size() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    let changes = composer.take_tree_changes();
    let node = node_of(&composer, window);
    assert_eq!(set_number(&changes, node, "width"), Some(40.0), "{changes:?}");
    assert_eq!(set_number(&changes, node, "height"), Some(50.0), "{changes:?}");
    assert_eq!(set_number(&changes, node, "x"), Some(10.0), "{changes:?}");
    assert_eq!(set_number(&changes, node, "y"), Some(10.0), "{changes:?}");
}

#[test]
fn a_resize_and_a_move_publish_the_new_size_and_position() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.take_tree_changes();
    let node = node_of(&composer, window);
    let bigger = Content {
        src: RectF::new(0.0, 0.0, 60.0, 70.0),
        size: Size::new(60, 70),
        transform: Transform::Normal,
    };
    composer.set_window_content(window, Some(bigger));
    let resized = composer.take_tree_changes();
    assert_eq!(set_number(&resized, node, "width"), Some(60.0), "{resized:?}");
    assert_eq!(set_number(&resized, node, "height"), Some(70.0), "{resized:?}");
    composer.place_window(window, Rect::new(200, 120, 60, 70));
    let moved = composer.take_tree_changes();
    assert_eq!(set_number(&moved, node, "x"), Some(200.0), "{moved:?}");
    assert_eq!(set_number(&moved, node, "y"), Some(120.0), "{moved:?}");
    composer.place_window(window, Rect::new(200, 120, 60, 70));
    assert!(
        composer.take_tree_changes().is_empty(),
        "the same place publishes nothing"
    );
}

#[test]
fn removing_a_window_publishes_its_node_as_destroyed() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(10, 10, 100, 100));
    composer.set_window_content(window, Some(content()));
    composer.take_tree_changes();
    let node = node_of(&composer, window);
    composer.remove_window(window);
    let changes = composer.take_tree_changes();
    assert!(
        changes.iter().any(|change| matches!(
            change,
            TreeChange::Destroyed { node: destroyed, .. } if *destroyed == node
        )),
        "the destruction is published: {changes:?}"
    );
}

/// A shell node inserted after a window sits above it in the scene, so the ray picks the shell
/// node, in its local space.
#[test]
fn a_shell_node_above_a_window_is_the_pick_in_its_local_space() {
    let (mut composer, _) = screen();
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(0, 0, 100, 100));
    composer.set_window_content(window, Some(content()));
    let shell = NodeId::new(1);
    composer.with_scene(|scene| {
        scene
            .insert_as(
                gnoblin_render::Actor::Shell,
                shell,
                NodeSpec::new(Rect::new(0, 0, 50, 50)),
            )
            .expect("the shell may insert a node below the base");
    });
    let (node, local) = composer
        .shell_node_at(PointF::new(10.0, 20.0))
        .expect("the shell node is topmost at the point");
    assert_eq!(node, shell);
    assert_eq!(local, PointF::new(10.0, 20.0));
}

/// A window inserted after a shell node sits above it, so the ray picks no shell node and the
/// pointer stays with the window.
#[test]
fn a_window_above_a_shell_node_leaves_no_pick() {
    let (mut composer, _) = screen();
    let shell = NodeId::new(1);
    composer.with_scene(|scene| {
        scene
            .insert_as(
                gnoblin_render::Actor::Shell,
                shell,
                NodeSpec::new(Rect::new(0, 0, 50, 50)),
            )
            .expect("the shell may insert a node below the base");
    });
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(0, 0, 100, 100));
    composer.set_window_content(window, Some(content()));
    assert!(composer.shell_node_at(PointF::new(10.0, 20.0)).is_none());
}

/// A node the compositor created that no batch has published yet is not picked (F29): the
/// pick falls through to the shell node under it, and once the batch has taken the creation
/// the node is picked.
#[test]
fn a_node_not_yet_in_a_batch_is_not_picked() {
    let (mut composer, _) = screen();
    let below = NodeId::new(1);
    let above = NodeId::new(2);
    composer.with_scene(|scene| {
        for node in [below, above] {
            scene
                .insert_as(
                    gnoblin_render::Actor::Shell,
                    node,
                    NodeSpec::new(Rect::new(0, 0, 50, 50)),
                )
                .expect("the shell may insert a node below the base");
        }
    });
    composer.tree_changes.push(TreeChange::Created {
        node: above,
        class: "shell".to_owned(),
        index: 0,
    });
    let (node, _) = composer
        .shell_node_at(PointF::new(10.0, 20.0))
        .expect("the published node under the unpublished one is picked");
    assert_eq!(node, below);
    composer.take_tree_changes();
    let (node, _) = composer
        .shell_node_at(PointF::new(10.0, 20.0))
        .expect("the node is picked once its batch has taken it");
    assert_eq!(node, above);
}

/// A View a shell makes: a node of class `view` at `bounds` above everything inserted so far, showing `source`.
fn view_showing(composer: &mut Composer, id: u64, bounds: Rect, source: Option<NodeId>) -> NodeId {
    let view = NodeId::new(id);
    composer.with_scene(|scene| {
        scene
            .insert_as(gnoblin_render::Actor::Shell, view, NodeSpec::new(bounds))
            .expect("the shell may insert a node below the base");
        scene.set_class(view, Some("view".to_owned())).expect("a class");
        scene.set_source(view, source).expect("a source");
    });
    view
}

/// A window at the output's origin, and its node. The node is the size of the window's content, 40 by 50, and its
/// picture is what a View of it shows.
fn window_to_show(composer: &mut Composer, id: u64) -> (WindowId, NodeId) {
    // A window that maps fades in from transparent, and a node that draws nothing takes no pointer.
    composer.set_reduced_motion(true);
    let window = WindowId::new(id);
    composer.insert_window(window, Rect::new(0, 0, 100, 100));
    composer.set_window_content(window, Some(content()));
    let node = composer.window_node(window).expect("the window has a node");
    (window, node)
}

/// A ray over a forwarding View lands on the window the View shows, at the point of the picture under it, and not
/// on the View: the shell has no pick, and the window pick is the source's window.
#[test]
fn a_ray_over_a_forwarding_view_lands_on_its_source_window_at_the_point_of_the_picture() {
    let (mut composer, _) = screen();
    let (window, node) = window_to_show(&mut composer, 1);
    // The View is 100 by 100 at (200, 0): its pixel is the point less its origin, and the picture shows the
    // window node's rectangle at the source plane's origin.
    view_showing(&mut composer, 10, Rect::new(200, 0, 100, 100), Some(node));
    let over = PointF::new(230.0, 40.0);
    assert!(
        composer.shell_node_at(over).is_none(),
        "the shell is sent nothing for a forwarding View"
    );
    let (landed, local) = composer
        .window_at(over, &|_| true)
        .expect("the window is under the picture's pixel");
    assert_eq!(landed, window);
    assert_eq!(
        local,
        PointF::new(30.0, 40.0),
        "the point of the picture, in the window's space"
    );
}

/// `capture` lands on the View: the shell's pick, in the View's plane, and no window is under the ray. Setting the
/// mode back to `forward` (or to anything that is not `capture`) forwards again.
#[test]
fn a_ray_over_a_capturing_view_lands_on_the_view_and_no_window() {
    let (mut composer, _) = screen();
    let (_window, node) = window_to_show(&mut composer, 1);
    let view = view_showing(&mut composer, 10, Rect::new(200, 0, 100, 100), Some(node));
    composer.with_scene(|scene| {
        scene
            .set_value(view, "input_mode", TreeValue::Text("capture".to_owned()))
            .expect("the mode is stored");
    });
    let over = PointF::new(230.0, 40.0);
    let (picked, local) = composer.shell_node_at(over).expect("the View is the shell's");
    assert_eq!(picked, view);
    assert_eq!(local, over);
    assert!(
        composer.window_at(over, &|_| true).is_none(),
        "the client is not under a capturing View"
    );
    composer.with_scene(|scene| {
        scene
            .set_value(view, "input_mode", TreeValue::Text("forward".to_owned()))
            .expect("the mode is stored");
    });
    assert!(composer.shell_node_at(over).is_none());
    assert!(composer.window_at(over, &|_| true).is_some());
}

/// `none` is a miss: a View without input passes the ray to what lies under it, a window or a shell node.
#[test]
fn a_ray_over_a_view_without_input_goes_to_what_lies_under_it() {
    let (mut composer, _) = screen();
    let (shown, source) = window_to_show(&mut composer, 1);
    let below = WindowId::new(2);
    composer.insert_window(below, Rect::new(200, 0, 100, 100));
    composer.set_window_content(below, Some(content()));
    let view = view_showing(&mut composer, 10, Rect::new(200, 0, 100, 100), Some(source));
    let over = PointF::new(230.0, 40.0);
    assert_eq!(
        composer.window_at(over, &|_| true).map(|(window, _)| window),
        Some(shown)
    );
    composer.with_scene(|scene| scene.set_input(view, false).expect("input is unset"));
    assert_eq!(
        composer.window_at(over, &|_| true).map(|(window, _)| window),
        Some(below),
        "the window under the View takes the ray"
    );
    assert!(composer.shell_node_at(over).is_none());
}

/// A View with no source has nothing to forward to, so it is the shell's like any other node; a View whose source
/// does not show anything that takes input at the point swallows the ray.
#[test]
fn a_view_with_no_source_is_the_shells_and_a_pixel_of_nothing_lands_nowhere() {
    let (mut composer, _) = screen();
    let (_, node) = window_to_show(&mut composer, 1);
    let empty = view_showing(&mut composer, 10, Rect::new(200, 0, 100, 100), None);
    let (picked, _) = composer
        .shell_node_at(PointF::new(230.0, 40.0))
        .expect("a View with no source is a node");
    assert_eq!(picked, empty);

    // The second View is 200 wide and its picture shows the 40 wide window at its left edge only.
    view_showing(&mut composer, 11, Rect::new(400, 0, 200, 100), Some(node));
    let blank = PointF::new(550.0, 40.0);
    assert!(composer.shell_node_at(blank).is_none());
    assert!(
        composer.window_at(blank, &|_| true).is_none(),
        "nothing in the picture takes the ray"
    );
    assert!(composer.window_at(PointF::new(420.0, 40.0), &|_| true).is_some());
}

/// A ray into a View whose source holds a shell node is that node's: the shell hears of the node, in the node's
/// own plane, and a View that shows itself through another ends the ray.
#[test]
fn a_ray_through_a_view_to_a_shell_node_is_the_nodes_and_a_ring_of_views_ends() {
    let (mut composer, _) = screen();
    let inner = NodeId::new(5);
    composer.with_scene(|scene| {
        scene
            .insert_as(
                gnoblin_render::Actor::Shell,
                inner,
                NodeSpec::new(Rect::new(0, 0, 60, 60)),
            )
            .expect("the shell may insert a node below the base");
    });
    let view = view_showing(&mut composer, 10, Rect::new(300, 100, 100, 100), Some(inner));
    let (picked, local) = composer
        .shell_node_at(PointF::new(320.0, 130.0))
        .expect("the node in the picture is the shell's");
    assert_eq!(picked, inner);
    assert_eq!(local, PointF::new(20.0, 30.0));
    assert!(
        composer.shell_node_at(PointF::new(390.0, 190.0)).is_none(),
        "the part of the picture the node does not cover is nothing"
    );

    // Two Views that show each other: the ray passes into each in turn and ends.
    let other = view_showing(&mut composer, 11, Rect::new(300, 100, 100, 100), Some(view));
    composer.with_scene(|scene| scene.set_source(view, Some(other)).expect("a source"));
    assert!(composer.shell_node_at(PointF::new(320.0, 130.0)).is_none());
}

/// The camera a View is drawn with decides where the picture's pixel is in the source: an orthographic camera
/// moved along the plane shows another part of it, so the same pixel lands on a different point of the window.
#[test]
fn the_point_a_ray_lands_on_follows_the_views_camera() {
    let (mut composer, _) = screen();
    let (window, node) = window_to_show(&mut composer, 1);
    let view = view_showing(&mut composer, 10, Rect::new(200, 0, 100, 100), Some(node));
    let camera = NodeId::new(11);
    composer.with_scene(|scene| {
        scene
            .insert_as(
                gnoblin_render::Actor::Shell,
                camera,
                NodeSpec::new(Rect::new(0, 0, 0, 0)),
            )
            .expect("the shell may insert a node below the base");
        scene.set_class(camera, Some("camera".to_owned())).expect("a class");
        scene.set_camera(view, Some(camera)).expect("a camera");
        let mut transform = *scene.transform(camera).expect("the camera has a transform");
        // The image's centre is the camera's position: (80, 50) puts the picture's origin at (30, 0).
        transform.base.offset = Vec3 {
            x: 80.0,
            y: 50.0,
            z: 100.0,
        };
        scene.set_transform(camera, transform).expect("the camera is placed");
    });
    let (landed, local) = composer
        .window_at(PointF::new(205.0, 40.0), &|_| true)
        .expect("the picture shows the window there");
    assert_eq!(landed, window);
    assert_eq!(
        local,
        PointF::new(35.0, 40.0),
        "pixel 5 of a picture whose origin is 30 is plane point 35"
    );
}

#[test]
fn a_new_output_is_a_screen_node_under_the_root_with_its_geometry() {
    let mut composer = stock_composer();
    let output = OutputId::new(7);
    composer.set_output(output, Rect::new(800, 0, 1920, 1080));
    let node = composer.screen_node(output).expect("the output has a screen node");
    assert_eq!(composer.scene.class(node), Some("screen"));
    assert_eq!(composer.scene.number(node, "x"), Some(800.0));
    assert_eq!(composer.scene.number(node, "width"), Some(1920.0));
    assert_eq!(composer.scene.number(node, "height"), Some(1080.0));
    assert_eq!(composer.scene.text(node, "name"), Some("output-7"));
    let changes = composer.take_tree_changes();
    assert!(
        changes.iter().any(|change| matches!(
            change,
            TreeChange::Created { node: created, class, .. } if *created == node && class == "screen"
        )),
        "the screen is announced: {changes:?}"
    );
}

#[test]
fn a_moved_output_publishes_only_the_geometry_that_changed() {
    let mut composer = stock_composer();
    let output = OutputId::new(7);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    composer.take_tree_changes();
    composer.set_output(output, Rect::new(0, 0, 1024, 600));
    let node = composer.screen_node(output).expect("the output keeps its screen node");
    assert_eq!(composer.scene.number(node, "width"), Some(1024.0));
    let changes = composer.take_tree_changes();
    assert_eq!(
        changes,
        vec![TreeChange::Set {
            node,
            key: "width".to_owned(),
            value: TreeValue::Double(1024.0),
        }]
    );
}

#[test]
fn a_removed_output_destroys_its_screen_node() {
    let mut composer = stock_composer();
    let output = OutputId::new(7);
    composer.set_output(output, Rect::new(0, 0, 800, 600));
    let node = composer.screen_node(output).expect("the output has a screen node");
    composer.take_tree_changes();
    composer.remove_output(output);
    assert_eq!(composer.screen_node(output), None);
    assert!(!composer.scene.contains(node));
    let changes = composer.take_tree_changes();
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, TreeChange::Destroyed { node: gone, .. } if *gone == node)),
        "the screen's destruction is announced: {changes:?}"
    );
}

/// A paint list of one opaque rectangle that fills `width` by `height` from the origin.
fn fill_list(width: f32, height: f32, red: u8) -> Vec<u8> {
    use gnoblin_paint::{PaintList, PaintOp, Rgba8};
    gnoblin_paint::encode(&PaintList {
        ops: vec![PaintOp::Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
            radius: 0.0,
            fill: Rgba8::new(red, 0, 0, 255),
            stroke: Rgba8::TRANSPARENT,
            stroke_width: 0.0,
        }],
    })
}

/// Adds a Canvas node at `rect` under `parent` or the root, holding `content`.
fn add_canvas(composer: &mut Composer, canvas: NodeId, parent: Option<NodeId>, rect: Rect, content: Option<Vec<u8>>) {
    composer.with_scene(|scene| {
        let mut spec = NodeSpec::new(rect);
        spec.parent = parent;
        scene.insert(canvas, spec).expect("a canvas node");
        scene
            .set_class(canvas, Some("canvas".to_owned()))
            .expect("the canvas's class");
        if let Some(content) = content {
            scene
                .set_value(canvas, "content", TreeValue::Bytes(content))
                .expect("the canvas's content");
        }
    });
}

/// The snapshot layers the next frame for `output` carries: where each lands, how opaque, and its picture.
fn canvas_layers(composer: &mut Composer, output: OutputId) -> Vec<(Rect, f32, Arc<Image>)> {
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(surface) => match &surface.source {
                SurfaceSource::Snapshot(image) => Some((surface.dst, surface.opacity, Arc::clone(image))),
                SurfaceSource::Buffer(_) | SurfaceSource::Dmabuf(_) | SurfaceSource::View(_) => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_canvas_draws_its_painted_content_where_it_sits() {
    let (mut composer, output) = screen();
    let canvas = NodeId::new(1);
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(5, 6, 20, 10),
        Some(fill_list(20.0, 10.0, 200)),
    );
    let layers = canvas_layers(&mut composer, output);
    assert_eq!(layers.len(), 1);
    let (dst, opacity, image) = &layers[0];
    assert_eq!(*dst, Rect::new(5, 6, 20, 10));
    assert_eq!(*opacity, 1.0);
    assert_eq!(image.size(), Size::new(20, 10));
    assert_eq!(image.pixel(Point::new(3, 3)), Some([200, 0, 0, 255]));
}

#[test]
fn a_canvas_without_content_or_not_drawn_has_no_layer() {
    let (mut composer, output) = screen();
    add_canvas(&mut composer, NodeId::new(1), None, Rect::new(0, 0, 10, 10), None);
    add_canvas(
        &mut composer,
        NodeId::new(2),
        None,
        Rect::new(0, 0, 10, 10),
        Some(fill_list(10.0, 10.0, 9)),
    );
    composer.with_scene(|scene| scene.set_visible(NodeId::new(2), false).expect("the canvas hides"));
    assert!(canvas_layers(&mut composer, output).is_empty());
    composer.with_scene(|scene| scene.set_visible(NodeId::new(2), true).expect("the canvas shows"));
    assert_eq!(canvas_layers(&mut composer, output).len(), 1);
}

#[test]
fn a_canvas_is_painted_again_only_when_its_content_or_size_changes() {
    let (mut composer, output) = screen();
    let canvas = NodeId::new(1);
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(0, 0, 8, 8),
        Some(fill_list(8.0, 8.0, 10)),
    );
    composer.begin_frame();
    let first = composer.canvas_picture(canvas).expect("painted");
    composer.end_frame();
    let _ = damage_of(&mut composer, output);
    assert!(
        Arc::ptr_eq(&first, &composer.canvas_picture(canvas).expect("painted")),
        "an unchanged canvas keeps its picture"
    );
    composer.with_scene(|scene| {
        scene
            .set_value(canvas, "content", TreeValue::Bytes(fill_list(8.0, 8.0, 20)))
            .expect("new content");
    });
    composer.begin_frame();
    let second = composer.canvas_picture(canvas).expect("painted again");
    composer.end_frame();
    assert!(!Arc::ptr_eq(&first, &second), "new content paints a new picture");
    assert_eq!(second.pixel(Point::new(1, 1)), Some([20, 0, 0, 255]));
    composer.with_scene(|scene| {
        scene
            .set_bounds(canvas, Rect::new(0, 0, 12, 9))
            .expect("the canvas resizes")
    });
    composer.begin_frame();
    let third = composer.canvas_picture(canvas).expect("painted again");
    composer.end_frame();
    assert_eq!(third.size(), Size::new(12, 9), "a resize paints at the new size");
}

#[test]
fn writing_a_canvas_content_damages_the_canvas_and_nothing_else() {
    let (mut composer, output) = screen();
    let canvas = NodeId::new(1);
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(30, 40, 20, 10),
        Some(fill_list(20.0, 10.0, 1)),
    );
    assert!(
        damage_of(&mut composer, output).is_some(),
        "a new canvas damages its pixels"
    );
    assert!(
        damage_of(&mut composer, output).is_none(),
        "an unchanged canvas draws nothing new"
    );
    composer.with_scene(|scene| {
        scene
            .set_value(canvas, "content", TreeValue::Bytes(fill_list(20.0, 10.0, 2)))
            .expect("new content");
    });
    let damage = damage_of(&mut composer, output).expect("a repaint is damage");
    assert_eq!(damage, vec![Rect::new(30, 40, 20, 10)]);
}

#[test]
fn canvases_draw_in_tree_order_children_over_their_parent() {
    let (mut composer, output) = screen();
    let (parent, child, sibling) = (NodeId::new(1), NodeId::new(2), NodeId::new(3));
    add_canvas(
        &mut composer,
        parent,
        None,
        Rect::new(0, 0, 10, 10),
        Some(fill_list(10.0, 10.0, 1)),
    );
    add_canvas(
        &mut composer,
        sibling,
        None,
        Rect::new(0, 0, 30, 30),
        Some(fill_list(30.0, 30.0, 3)),
    );
    add_canvas(
        &mut composer,
        child,
        Some(parent),
        Rect::new(2, 2, 4, 4),
        Some(fill_list(4.0, 4.0, 2)),
    );
    let drawn: Vec<(Rect, u8)> = canvas_layers(&mut composer, output)
        .iter()
        .map(|(dst, _, image)| (*dst, image.pixel(Point::new(0, 0)).expect("a pixel")[0]))
        .collect();
    assert_eq!(
        drawn,
        vec![
            (Rect::new(0, 0, 10, 10), 1),
            (Rect::new(2, 2, 4, 4), 2),
            (Rect::new(0, 0, 30, 30), 3)
        ],
        "a parent, then its child placed in the parent's space, then the next sibling"
    );
}

#[test]
fn a_canvas_composes_the_opacity_of_its_parents() {
    let (mut composer, output) = screen();
    let (parent, child) = (NodeId::new(1), NodeId::new(2));
    add_canvas(&mut composer, parent, None, Rect::new(0, 0, 10, 10), None);
    add_canvas(
        &mut composer,
        child,
        Some(parent),
        Rect::new(0, 0, 10, 10),
        Some(fill_list(10.0, 10.0, 5)),
    );
    composer.with_scene(|scene| {
        scene
            .set_property(parent, VisualProperty::Opacity, PropertyValue::Number(0.5))
            .expect("the parent fades");
        scene
            .set_property(child, VisualProperty::Opacity, PropertyValue::Number(0.5))
            .expect("the child fades");
    });
    let layers = canvas_layers(&mut composer, output);
    assert_eq!(layers.len(), 1);
    assert!(
        (layers[0].1 - 0.25).abs() < 1e-6,
        "opacity multiplies down the tree: {}",
        layers[0].1
    );
}

#[test]
fn a_view_of_a_canvas_draws_the_canvas_pixels() {
    let (mut composer, output) = screen();
    let canvas = NodeId::new(2);
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(0, 0, 40, 30),
        Some(fill_list(40.0, 30.0, 77)),
    );
    add_view(&mut composer, NodeId::new(1), None, Some(canvas));
    let _ = view_layers(&mut composer, output);
    let pictures = composer.view_pictures();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].1.pixel(Point::new(5, 5)), Some([77, 0, 0, 255]));
}

/// A camera node (3) that the View (1) of a 40 by 30 canvas draws through: `projection`, positioned at
/// (20, 15, 100) in front of the canvas, with `numbers` set on top of the schema's defaults.
fn view_through_camera(projection: &str, numbers: &[(&str, f64)]) -> Composer {
    let (mut composer, output) = screen();
    let canvas = NodeId::new(2);
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(0, 0, 40, 30),
        Some(fill_list(40.0, 30.0, 77)),
    );
    add_view(&mut composer, NodeId::new(1), None, Some(canvas));
    let camera = NodeId::new(3);
    composer.with_scene(|scene| {
        scene
            .insert(camera, NodeSpec::new(Rect::new(0, 0, 0, 0)))
            .expect("a camera node");
        scene
            .set_class(camera, Some("camera".to_owned()))
            .expect("the camera's class");
        scene
            .set_text(camera, "projection", projection.to_owned())
            .expect("the camera's projection");
        for (key, value) in numbers {
            scene.set_number(camera, key, *value).expect("a camera number");
        }
        let mut transform = *scene.transform(camera).expect("the camera's transform");
        transform.base.offset = Vec3 {
            x: 20.0,
            y: 15.0,
            z: 100.0,
        };
        scene.set_transform(camera, transform).expect("the camera's pose");
        scene
            .set_camera(NodeId::new(1), Some(camera))
            .expect("the view's camera");
    });
    let _ = view_layers(&mut composer, output);
    composer
}

/// The pixel of the picture a View draws at `(x, y)`, or `None` when it has none.
fn view_pixel(composer: &Composer, view: NodeId, x: i32, y: i32) -> Option<[u8; 4]> {
    let (_, picture) = composer.view_pictures().into_iter().find(|(node, _)| *node == view)?;
    picture.pixel(Point::new(x, y))
}

#[test]
fn a_view_through_a_stereo_camera_draws_each_eye_in_its_half_from_its_side() {
    // The eyes are 63 units apart (0.063 m at 1 mm a unit), 100 units from the canvas. The left eye, at x = -11.5,
    // sees the canvas at the right of its half; the right eye, at x = 51.5, sees it at the left of its half.
    let composer = view_through_camera("stereo", &[]);
    let view = NodeId::new(1);
    assert_eq!(
        view_pixel(&composer, view, 19, 15),
        Some(CANVAS_PIXEL),
        "left eye, right edge of its half"
    );
    assert_eq!(
        view_pixel(&composer, view, 0, 15),
        Some(BACKGROUND_PIXEL),
        "left eye, left edge of its half"
    );
    assert_eq!(
        view_pixel(&composer, view, 20, 15),
        Some(CANVAS_PIXEL),
        "right eye, left edge of its half"
    );
    assert_eq!(
        view_pixel(&composer, view, 39, 15),
        Some(BACKGROUND_PIXEL),
        "right eye, right edge of its half"
    );
}

#[test]
fn the_eye_separation_follows_ipd_and_metres() {
    // No separation: both eyes are at x = 20, and see the canvas centred in their half.
    let composer = view_through_camera("stereo", &[("ipd", 0.0)]);
    let view = NodeId::new(1);
    assert_eq!(view_pixel(&composer, view, 10, 15), Some(CANVAS_PIXEL));
    assert_eq!(view_pixel(&composer, view, 30, 15), Some(CANVAS_PIXEL));
    assert_eq!(view_pixel(&composer, view, 0, 15), view_pixel(&composer, view, 20, 15));
    // The same 0.063 m in metres of 0.002 is 31.5 units: half the displacement, so the canvas fills more of
    // each half than at 63 units.
    let wide = view_through_camera("stereo", &[]);
    let narrow = view_through_camera("stereo", &[("metres", 0.002)]);
    let red = |composer: &Composer| {
        (0..20)
            .filter(|x| view_pixel(composer, view, *x, 15) == Some(CANVAS_PIXEL))
            .count()
    };
    assert!(red(&narrow) > red(&wide), "{} vs {}", red(&narrow), red(&wide));
}

#[test]
fn a_stereo_camera_with_an_unusable_frustum_or_scale_draws_a_blank_picture() {
    let view = NodeId::new(1);
    for numbers in [
        vec![
            ("fov_left", 30.0),
            ("fov_right", -30.0),
            ("fov_up", 30.0),
            ("fov_down", -30.0),
        ],
        vec![("fov", 0.0)],
        vec![("metres", 0.0)],
        vec![("ipd", -1.0)],
    ] {
        let composer = view_through_camera("stereo", &numbers);
        for x in [0, 10, 19, 20, 30, 39] {
            assert_eq!(
                view_pixel(&composer, view, x, 15),
                Some(BACKGROUND_PIXEL),
                "{numbers:?} at {x}"
            );
        }
    }
}

#[test]
fn an_off_axis_perspective_camera_shifts_the_view_and_a_bad_frustum_blanks_it() {
    let view = NodeId::new(1);
    // A frustum that reaches 50 degrees left and 5 right puts the canvas (straight ahead) at the picture's right.
    let shifted = view_through_camera(
        "perspective",
        &[
            ("fov_left", -50.0),
            ("fov_right", 5.0),
            ("fov_up", 30.0),
            ("fov_down", -30.0),
        ],
    );
    assert_eq!(view_pixel(&shifted, view, 39, 15), Some(CANVAS_PIXEL));
    assert_eq!(view_pixel(&shifted, view, 0, 15), Some(BACKGROUND_PIXEL));
    let blank = view_through_camera(
        "perspective",
        &[
            ("fov_left", 5.0),
            ("fov_right", -50.0),
            ("fov_up", 30.0),
            ("fov_down", -30.0),
        ],
    );
    assert_eq!(view_pixel(&blank, view, 39, 15), Some(BACKGROUND_PIXEL));
}

/// What a canvas filled with red 77 looks like under a View, and what the neutral grey of a secure node is.
const CANVAS_PIXEL: [u8; 4] = [77, 0, 0, 255];
const MASK_PIXEL: [u8; 4] = [128, 128, 128, 255];
const BACKGROUND_PIXEL: [u8; 4] = [0, 0, 0, 255];

/// A View (node 1) of a canvas (node 2) filled with red 77, drawn once, as `secure` or not.
fn view_of_a_canvas(secure: bool) -> (Composer, OutputId) {
    let (mut composer, output) = screen();
    let canvas = NodeId::new(2);
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(0, 0, 40, 30),
        Some(fill_list(40.0, 30.0, 77)),
    );
    add_view(&mut composer, NodeId::new(1), None, Some(canvas));
    if secure {
        mark_secure(&mut composer, canvas, true);
    }
    (composer, output)
}

/// Writes the `secure` value a shell writes on a node.
fn mark_secure(composer: &mut Composer, node: NodeId, secure: bool) {
    composer.with_scene(|scene| {
        scene
            .set_value(node, "secure", TreeValue::Bool(secure))
            .expect("the node's secure value")
    });
}

/// The pixel at (5, 5) of the picture capture reads for a View, or `None` when it has none.
fn captured_pixel(composer: &Composer, view: NodeId) -> Option<[u8; 4]> {
    let (_, picture) = composer.view_pictures().into_iter().find(|(node, _)| *node == view)?;
    picture.pixel(Point::new(5, 5))
}

/// What the next frame for `output` draws for each View, at the rectangle `add_view` gives it: whether the layer
/// is secure, and its pixel at (5, 5).
fn drawn_views(composer: &mut Composer, output: OutputId) -> Vec<(bool, Option<[u8; 4]>)> {
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(Surface {
                source: SurfaceSource::Snapshot(image),
                secure,
                dst,
                ..
            }) if *dst == Rect::new(10, 20, 40, 30) => Some((*secure, image.pixel(Point::new(5, 5)))),
            _ => None,
        })
        .collect()
}

#[test]
fn a_secure_node_in_a_views_source_is_masked_in_its_capture_picture_and_not_on_the_display() {
    let (mut composer, output) = view_of_a_canvas(true);
    let drawn = drawn_views(&mut composer, output);
    assert_eq!(
        drawn,
        vec![(true, Some(CANVAS_PIXEL))],
        "the display shows the canvas, and its layer is secure so an output capture replaces the whole View"
    );
    assert_eq!(
        captured_pixel(&composer, NodeId::new(1)),
        Some(MASK_PIXEL),
        "a capture of the View reads the mask in the secure node's place"
    );
}

#[test]
fn a_view_with_no_secure_node_is_captured_as_drawn_and_is_not_a_secure_layer() {
    let (mut composer, output) = view_of_a_canvas(false);
    assert_eq!(drawn_views(&mut composer, output), vec![(false, Some(CANVAS_PIXEL))]);
    assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(CANVAS_PIXEL));
}

#[test]
fn omit_leaves_a_secure_node_out_of_a_views_capture_picture() {
    let (mut composer, output) = view_of_a_canvas(true);
    composer.set_secure_treatment(SecureTreatment::Omit);
    let _ = drawn_views(&mut composer, output);
    assert_eq!(
        captured_pixel(&composer, NodeId::new(1)),
        Some(BACKGROUND_PIXEL),
        "nothing is drawn where the secure node is, as an output capture omits a secure layer"
    );
}

#[test]
fn the_children_of_a_secure_node_are_masked_with_it() {
    let (mut composer, output) = screen();
    let (parent, canvas) = (NodeId::new(3), NodeId::new(2));
    add_source(&mut composer, parent, Rect::new(0, 0, 40, 30));
    add_canvas(
        &mut composer,
        canvas,
        Some(parent),
        Rect::new(0, 0, 40, 30),
        Some(fill_list(40.0, 30.0, 77)),
    );
    add_view(&mut composer, NodeId::new(1), None, Some(parent));
    mark_secure(&mut composer, parent, true);
    let _ = drawn_views(&mut composer, output);
    assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(MASK_PIXEL));
}

#[test]
fn a_node_that_becomes_secure_is_masked_in_the_next_picture_and_one_that_stops_is_shown() {
    let (mut composer, output) = view_of_a_canvas(false);
    let _ = drawn_views(&mut composer, output);
    assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(CANVAS_PIXEL));
    mark_secure(&mut composer, NodeId::new(2), true);
    let _ = drawn_views(&mut composer, output);
    assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(MASK_PIXEL));
    mark_secure(&mut composer, NodeId::new(2), false);
    let _ = drawn_views(&mut composer, output);
    assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(CANVAS_PIXEL));
}

#[test]
fn a_secure_mark_is_drawn_at_once_though_the_view_is_paced() {
    let (mut composer, output) = paced_view(1);
    assert_eq!(view_red(&composer), Some(1), "the first picture is drawn at once");
    // A second of the slot is far from over, so a plain change would wait.
    composer.advance_motion(20);
    mark_secure(&mut composer, PACED_CANVAS, true);
    let _ = damage_of(&mut composer, output);
    assert_eq!(
        view_red(&composer),
        Some(128),
        "the capture picture holds the mask, not the pixels drawn before the mark"
    );
}

#[test]
fn a_view_of_a_view_never_carries_a_secure_nodes_pixels() {
    let (mut composer, output) = screen();
    let (outer, inner, holder, canvas) = (NodeId::new(1), NodeId::new(3), NodeId::new(4), NodeId::new(2));
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(0, 0, 40, 30),
        Some(fill_list(40.0, 30.0, 77)),
    );
    mark_secure(&mut composer, canvas, true);
    add_source(&mut composer, holder, Rect::new(0, 0, 40, 30));
    add_view(&mut composer, inner, Some(holder), Some(canvas));
    add_view(&mut composer, outer, None, Some(holder));
    let _ = drawn_views(&mut composer, output);
    assert_eq!(
        captured_pixel(&composer, inner),
        Some(MASK_PIXEL),
        "the inner View's own capture is masked"
    );
    let outer_pixel = captured_pixel(&composer, outer).expect("the outer View has a picture");
    assert_ne!(
        outer_pixel, CANVAS_PIXEL,
        "the outer View holds none of the secure node's pixels"
    );
}

const PACED_CANVAS: NodeId = NodeId::new(2);
const PACED_VIEW: NodeId = NodeId::new(1);

/// A View of a canvas filled with red 1, drawn once, with its `fps` set to `fps`.
fn paced_view(fps: u64) -> (Composer, OutputId) {
    let (mut composer, output) = screen();
    add_canvas(
        &mut composer,
        PACED_CANVAS,
        None,
        Rect::new(0, 0, 40, 30),
        Some(fill_list(40.0, 30.0, 1)),
    );
    add_view(&mut composer, PACED_VIEW, None, Some(PACED_CANVAS));
    composer.with_scene(|scene| {
        scene
            .set_value(PACED_VIEW, "fps", TreeValue::Uint(fps))
            .expect("the view's fps")
    });
    let _ = damage_of(&mut composer, output);
    (composer, output)
}

/// Repaints the source canvas red `red`, as a live source changing between two frames.
fn repaint(composer: &mut Composer, red: u8) {
    composer.with_scene(|scene| {
        scene
            .set_value(PACED_CANVAS, "content", TreeValue::Bytes(fill_list(40.0, 30.0, red)))
            .expect("the canvas's content")
    });
}

/// The red of the View's picture, or `None` when it has none.
fn view_red(composer: &Composer) -> Option<u8> {
    let (_, picture) = composer
        .view_pictures()
        .into_iter()
        .find(|(view, _)| *view == PACED_VIEW)?;
    picture.pixel(Point::new(5, 5)).map(|pixel| pixel[0])
}

#[test]
fn a_view_with_fps_redraws_at_most_once_per_slot_though_its_source_changes_every_frame() {
    let (mut composer, output) = paced_view(10);
    assert_eq!(view_red(&composer), Some(1), "the first picture is drawn at once");
    let mut drawn = Vec::new();
    for step in 1..=25u8 {
        composer.advance_motion(20);
        repaint(&mut composer, step + 1);
        let before = view_red(&composer);
        let _ = damage_of(&mut composer, output);
        if view_red(&composer) != before {
            drawn.push((step, view_red(&composer)));
        }
    }
    // Ten frames a second is a slot every 100 ms: steps 5, 10, 15, 20 and 25 of 20 ms, each drawing the
    // source as it is then.
    assert_eq!(
        drawn,
        vec![
            (5, Some(6)),
            (10, Some(11)),
            (15, Some(16)),
            (20, Some(21)),
            (25, Some(26))
        ]
    );
}

#[test]
fn damage_in_a_slot_is_coalesced_and_the_last_state_is_drawn_at_the_next() {
    let (mut composer, output) = paced_view(10);
    composer.advance_motion(10);
    repaint(&mut composer, 10);
    let _ = damage_of(&mut composer, output);
    composer.advance_motion(40);
    repaint(&mut composer, 20);
    let _ = damage_of(&mut composer, output);
    assert_eq!(view_red(&composer), Some(1), "nothing is redrawn inside the slot");
    composer.advance_motion(49);
    let _ = damage_of(&mut composer, output);
    assert_eq!(view_red(&composer), Some(1), "99 ms is still inside the slot");
    // No write at the slot itself: the pending redraw still happens, with the last state.
    composer.advance_motion(1);
    assert!(
        damage_of(&mut composer, output).is_some(),
        "the redraw damages the View's pixels"
    );
    assert_eq!(view_red(&composer), Some(20), "the last state is what the slot draws");
    // Nothing is pending afterwards.
    composer.advance_motion(500);
    assert!(damage_of(&mut composer, output).is_none(), "the slot drew everything");
}

#[test]
fn a_view_with_fps_zero_redraws_on_every_change() {
    let (mut composer, output) = paced_view(0);
    for red in 2..=6u8 {
        repaint(&mut composer, red);
        let _ = damage_of(&mut composer, output);
        assert_eq!(
            view_red(&composer),
            Some(red),
            "no time has passed and the View followed"
        );
    }
    let _ = damage_of(&mut composer, output);
    composer.advance_motion(1000);
    assert!(
        damage_of(&mut composer, output).is_none(),
        "an unchanged View draws nothing"
    );
}

#[test]
fn a_view_whose_source_is_idle_is_not_redrawn_at_its_slot() {
    let (mut composer, output) = paced_view(10);
    for _ in 0..5 {
        composer.advance_motion(100);
        assert!(
            damage_of(&mut composer, output).is_none(),
            "fps caps redraws, it does not force them"
        );
    }
}

#[test]
fn lowering_fps_to_zero_draws_the_pending_state_at_once() {
    let (mut composer, output) = paced_view(1);
    composer.advance_motion(10);
    repaint(&mut composer, 9);
    let _ = damage_of(&mut composer, output);
    assert_eq!(view_red(&composer), Some(1), "one frame a second holds the picture");
    composer.with_scene(|scene| {
        scene
            .set_value(PACED_VIEW, "fps", TreeValue::Uint(0))
            .expect("the view's fps")
    });
    let _ = damage_of(&mut composer, output);
    assert_eq!(view_red(&composer), Some(9), "on demand draws what was waiting");
}

#[test]
fn a_view_that_resizes_is_drawn_at_once_whatever_its_fps() {
    let (mut composer, output) = paced_view(1);
    composer.with_scene(|scene| {
        scene
            .set_bounds(PACED_VIEW, Rect::new(10, 20, 20, 10))
            .expect("the view resizes")
    });
    let _ = damage_of(&mut composer, output);
    let (_, picture) = composer
        .view_pictures()
        .into_iter()
        .find(|(view, _)| *view == PACED_VIEW)
        .expect("the view's picture");
    assert_eq!(
        picture.size(),
        Size::new(20, 10),
        "a picture of the wrong size is never kept"
    );
}

/// An `fps` too large to mean anything is paced to the one-millisecond clock the loop runs on: the slot is never
/// zero (no division by it, no redraw inside one tick) and never a panic, including at the largest value a `uint`
/// holds.
#[test]
fn an_enormous_fps_is_a_one_millisecond_slot_and_neither_panics_nor_draws_twice_in_one() {
    for fps in [1000, 1_000_000, u64::from(u32::MAX) + 1, u64::MAX] {
        let (mut composer, output) = paced_view(fps);
        repaint(&mut composer, 5);
        let _ = damage_of(&mut composer, output);
        assert_eq!(
            view_red(&composer),
            Some(1),
            "fps {fps}: inside the millisecond the picture holds"
        );
        composer.advance_motion(1);
        let _ = damage_of(&mut composer, output);
        assert_eq!(
            view_red(&composer),
            Some(5),
            "fps {fps}: the next millisecond draws the change"
        );
        repaint(&mut composer, 6);
        let _ = damage_of(&mut composer, output);
        assert_eq!(
            view_red(&composer),
            Some(5),
            "fps {fps}: one draw a millisecond, not one a change"
        );
    }
}

/// A node's `effects` list is the effect chain of the layer that draws it, for a canvas and for a View, and
/// clearing the list clears the chain.
#[test]
fn a_nodes_effects_list_is_the_effect_chain_of_its_layer() {
    let (mut composer, output) = screen();
    let canvas = NodeId::new(2);
    add_canvas(
        &mut composer,
        canvas,
        None,
        Rect::new(0, 0, 40, 30),
        Some(fill_list(40.0, 30.0, 77)),
    );
    add_view(&mut composer, NodeId::new(1), None, Some(canvas));
    let chains = |composer: &mut Composer| -> Vec<Effects> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(surface) => Some(surface.effects.clone()),
                _ => None,
            })
            .collect()
    };
    assert!(
        chains(&mut composer).iter().all(Effects::is_empty),
        "no effects list, no passes"
    );
    for node in [canvas, NodeId::new(1)] {
        composer.with_scene(|scene| scene.set_effects(node, blur_and_shadow()).expect("the effects"));
    }
    let drawn = chains(&mut composer);
    assert_eq!(drawn.len(), 2, "the canvas and the View each draw one layer");
    assert!(
        drawn.iter().all(|chain| *chain == blur_and_shadow()),
        "each layer carries its node's list, in order: {drawn:?}"
    );
    for node in [canvas, NodeId::new(1)] {
        composer.with_scene(|scene| scene.set_effects(node, Effects::new()).expect("the effects"));
    }
    assert!(
        chains(&mut composer).iter().all(Effects::is_empty),
        "an emptied list draws no passes"
    );
}

#[test]
fn a_canvas_with_content_that_does_not_decode_draws_nothing() {
    let (mut composer, output) = screen();
    add_canvas(
        &mut composer,
        NodeId::new(1),
        None,
        Rect::new(0, 0, 10, 10),
        Some(b"junk".to_vec()),
    );
    assert!(canvas_layers(&mut composer, output).is_empty());
}

#[test]
fn a_canvas_too_large_to_paint_draws_nothing() {
    let (mut composer, output) = screen();
    add_canvas(
        &mut composer,
        NodeId::new(1),
        None,
        Rect::new(0, 0, 9000, 9000),
        Some(fill_list(10.0, 10.0, 1)),
    );
    assert!(canvas_layers(&mut composer, output).is_empty());
}

#[test]
fn a_zero_size_canvas_draws_nothing_and_one_pixel_is_enough() {
    let (mut composer, output) = screen();
    add_canvas(
        &mut composer,
        NodeId::new(1),
        None,
        Rect::new(4, 4, 0, 0),
        Some(fill_list(10.0, 10.0, 1)),
    );
    add_canvas(
        &mut composer,
        NodeId::new(2),
        None,
        Rect::new(4, 4, 10, 0),
        Some(fill_list(10.0, 10.0, 1)),
    );
    assert!(canvas_layers(&mut composer, output).is_empty(), "no area, no picture");
    composer.with_scene(|scene| {
        scene
            .set_bounds(NodeId::new(2), Rect::new(4, 4, 1, 1))
            .expect("the canvas resizes")
    });
    let layers = canvas_layers(&mut composer, output);
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].2.size(), Size::new(1, 1));
}

#[test]
fn a_canvas_is_painted_at_the_scale_of_the_output_it_is_on_and_drawn_over_its_logical_rectangle() {
    let (mut composer, output) = screen();
    composer.set_output_scale(output, Scale::integral(2));
    add_canvas(
        &mut composer,
        NodeId::new(1),
        None,
        Rect::new(5, 6, 20, 10),
        Some(fill_list(20.0, 10.0, 200)),
    );
    let layers = canvas_layers(&mut composer, output);
    assert_eq!(layers.len(), 1);
    let (dst, _, image) = &layers[0];
    assert_eq!(*dst, Rect::new(5, 6, 20, 10), "the rectangle is logical");
    assert_eq!(
        image.size(),
        Size::new(40, 20),
        "the picture has two pixels per logical pixel"
    );
    assert_eq!(
        image.pixel(Point::new(39, 19)),
        Some([200, 0, 0, 255]),
        "the fill covers it all"
    );

    composer.set_output_scale(output, Scale::new(3, 2));
    let layers = canvas_layers(&mut composer, output);
    assert_eq!(
        layers[0].2.size(),
        Size::new(30, 15),
        "a fractional scale repaints at its own size"
    );

    composer.set_output_scale(output, Scale::IDENTITY);
    let layers = canvas_layers(&mut composer, output);
    assert_eq!(layers[0].2.size(), Size::new(20, 10), "and one scale repaints at one");
}

#[test]
fn a_canvas_whose_scaled_picture_is_too_large_is_painted_at_one() {
    let (mut composer, output) = screen();
    composer.set_output_scale(output, Scale::integral(2));
    add_canvas(
        &mut composer,
        NodeId::new(1),
        None,
        Rect::new(0, 0, 3000, 3000),
        Some(fill_list(3000.0, 3000.0, 9)),
    );
    let layers = canvas_layers(&mut composer, output);
    assert_eq!(layers.len(), 1, "nine million pixels still draw");
    assert_eq!(layers[0].2.size(), Size::new(3000, 3000));
}

/// Whether each canvas layer the next frame carries is secure, in paint order.
fn canvas_layers_secure(composer: &mut Composer, output: OutputId) -> Vec<bool> {
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    composer.compose(output, &mut frame);
    composer.end_frame();
    frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(Surface {
                source: SurfaceSource::Snapshot(_),
                secure,
                ..
            }) => Some(*secure),
            _ => None,
        })
        .collect()
}

#[test]
fn a_secure_canvas_and_its_children_are_secure_layers_and_a_mark_redraws_them() {
    let (mut composer, output) = screen();
    let parent = NodeId::new(1);
    let child = NodeId::new(2);
    add_canvas(
        &mut composer,
        parent,
        None,
        Rect::new(0, 0, 30, 30),
        Some(fill_list(30.0, 30.0, 1)),
    );
    add_canvas(
        &mut composer,
        child,
        Some(parent),
        Rect::new(5, 5, 10, 10),
        Some(fill_list(10.0, 10.0, 2)),
    );
    assert_eq!(canvas_layers_secure(&mut composer, output), vec![false, false]);
    assert_eq!(damage_of(&mut composer, output), None, "nothing changed");

    mark_secure(&mut composer, parent, true);
    assert_eq!(
        canvas_layers_secure(&mut composer, output),
        vec![true, true],
        "a secure canvas's children are secure with it"
    );
    mark_secure(&mut composer, parent, false);
    mark_secure(&mut composer, child, true);
    assert_eq!(
        canvas_layers_secure(&mut composer, output),
        vec![false, true],
        "a secure child leaves its parent as it was"
    );
    mark_secure(&mut composer, child, false);
    composer.begin_frame();
    let mut frame = OutputFrame::new();
    assert!(
        composer.compose(output, &mut frame),
        "clearing the mark redraws the frame a capture reads"
    );
    composer.end_frame();
    let secure: Vec<bool> = frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(surface) => Some(surface.secure),
            _ => None,
        })
        .collect();
    assert_eq!(secure, vec![false, false]);
}

/// The `skipped` texts the composer queued for `node`, last first, or `None` when it queued none.
fn queued_skipped(composer: &mut Composer, node: NodeId) -> Option<Vec<String>> {
    composer
        .take_tree_changes()
        .into_iter()
        .rev()
        .find_map(|change| match change {
            TreeChange::Set {
                node: changed,
                key,
                value: TreeValue::ListText(texts),
            } if changed == node && key == "skipped" => Some(texts),
            _ => None,
        })
}

#[test]
fn an_effect_on_a_node_placed_in_3d_is_reported_skipped_and_cleared_when_it_is_flat() {
    use gnoblin_render::{BlurParams, Shadow};
    let (mut composer, _output) = screen();
    let node = NodeId::new(0x4000);
    let mut chain = Effects::new();
    chain
        .push(Pass::Blur(BlurParams {
            radius: 4.0,
            passes: 2,
            ..BlurParams::default()
        }))
        .expect("a blur");
    chain
        .push(Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 4.0, 0.0),
            blur: 12.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: false,
            inset: false,
        }))
        .expect("a shadow");
    composer.with_scene(|scene| {
        scene
            .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("the node");
        scene.set_effects(node, chain).expect("the effects");
    });
    composer.take_tree_changes();

    composer.publish_skipped_effects();
    assert_eq!(queued_skipped(&mut composer, node), None, "a flat node skips nothing");

    composer.with_scene(|scene| {
        scene
            .set_property(node, VisualProperty::RotationX, PropertyValue::Number(30.0))
            .expect("a tilt");
    });
    composer.publish_skipped_effects();
    let want = vec![
        "blur: unsupported-effect-on-3d".to_owned(),
        "shadow: unsupported-effect-on-3d".to_owned(),
    ];
    assert_eq!(queued_skipped(&mut composer, node), Some(want.clone()));
    assert_eq!(
        composer.with_scene(|scene| scene.value(node, "skipped").cloned()),
        Some(TreeValue::ListText(want)),
        "the node itself carries the list a snapshot reads"
    );

    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        None,
        "an unchanged list is not republished"
    );

    composer.with_scene(|scene| {
        scene
            .set_property(node, VisualProperty::RotationX, PropertyValue::Number(0.0))
            .expect("flat again");
    });
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        Some(Vec::new()),
        "a node that is flat again gets the empty list back"
    );
}

/// A blur and a shadow, in that order.
fn blur_and_shadow() -> Effects {
    use gnoblin_render::{BlurParams, Shadow};
    let mut chain = Effects::new();
    chain
        .push(Pass::Blur(BlurParams {
            radius: 4.0,
            passes: 2,
            ..BlurParams::default()
        }))
        .expect("a blur");
    chain
        .push(Pass::Shadow(Shadow {
            offset: Vec3::new(0.0, 4.0, 0.0),
            blur: 12.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: false,
            inset: false,
        }))
        .expect("a shadow");
    chain
}

#[test]
fn an_effect_the_renderer_cannot_draw_is_reported_skipped_by_kind() {
    let (mut composer, _output) = screen();
    let node = NodeId::new(0x4000);
    composer.with_scene(|scene| {
        scene
            .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("the node");
        scene.set_effects(node, blur_and_shadow()).expect("the effects");
    });
    composer.take_tree_changes();

    composer.set_effect_kinds(&["shadow"]);
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        Some(vec!["blur: unsupported-by-renderer".to_owned()]),
        "a renderer that draws shadows only skips the blur"
    );

    composer.set_effect_kinds(&[]);
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        Some(vec![
            "blur: unsupported-by-renderer".to_owned(),
            "shadow: unsupported-by-renderer".to_owned(),
        ]),
        "a renderer that draws none skips both, in list order"
    );
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        None,
        "an unchanged list is not republished"
    );

    composer.set_effect_kinds(Pass::NAMES);
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        Some(Vec::new()),
        "a renderer that draws both skips nothing"
    );
}

/// An inset shadow, a background and a filter, in that order.
fn inset_background_and_filter() -> Effects {
    use gnoblin_render::{FilterParams, Shadow};
    let mut chain = Effects::new();
    chain
        .push(Pass::Shadow(Shadow {
            offset: Vec3::ZERO,
            blur: 6.0,
            spread: 0.0,
            opacity: 1.0,
            color: Color::BLACK,
            frame: false,
            inset: true,
        }))
        .expect("an inset shadow");
    chain
        .push(Pass::Background { color: Color::WHITE })
        .expect("a background");
    chain
        .push(Pass::Filter(FilterParams {
            blur: 3.0,
            ..FilterParams::default()
        }))
        .expect("a filter");
    chain
}

#[test]
fn an_inset_shadow_a_background_and_a_filter_are_skipped_by_a_renderer_that_does_not_list_them() {
    use crate::render::renderer::RendererKind;
    let (mut composer, _output) = screen();
    let node = NodeId::new(0x4000);
    composer.with_scene(|scene| {
        scene
            .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("the node");
        scene
            .set_effects(node, inset_background_and_filter())
            .expect("the effects");
    });
    composer.take_tree_changes();

    // No renderer lists the three kinds yet, so each one names all of them, in list order.
    let want: Vec<String> = ["inset-shadow", "background", "filter"]
        .into_iter()
        .map(|kind| format!("{kind}: unsupported-by-renderer"))
        .collect();
    for renderer in [RendererKind::Software, RendererKind::Gles, RendererKind::Vulkan] {
        assert!(
            !["inset-shadow", "background", "filter"]
                .iter()
                .any(|kind| renderer.effect_kinds().contains(kind)),
            "{} lists a kind it does not draw yet",
            renderer.name()
        );
        composer.set_effect_kinds(renderer.effect_kinds());
        composer.publish_skipped_effects();
        let published = queued_skipped(&mut composer, node);
        if renderer == RendererKind::Software {
            assert_eq!(published, Some(want.clone()), "the first publication names every kind");
        } else {
            assert_eq!(published, None, "{}: the same list is not republished", renderer.name());
        }
    }
    composer.set_effect_kinds(Pass::NAMES);
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        Some(Vec::new()),
        "a renderer that draws every kind skips nothing"
    );
}

#[test]
fn an_effect_with_both_reasons_names_both_on_one_entry() {
    let (mut composer, _output) = screen();
    let node = NodeId::new(0x4000);
    composer.with_scene(|scene| {
        scene
            .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("the node");
        scene.set_effects(node, blur_and_shadow()).expect("the effects");
        scene
            .set_property(node, VisualProperty::RotationX, PropertyValue::Number(30.0))
            .expect("a tilt");
    });
    composer.take_tree_changes();

    composer.set_effect_kinds(&["shadow"]);
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        Some(vec![
            "blur: unsupported-by-renderer, unsupported-effect-on-3d".to_owned(),
            "shadow: unsupported-effect-on-3d".to_owned(),
        ])
    );
}

#[test]
fn the_main_seat_publishes_the_effect_kinds_the_renderer_draws() {
    let mut composer = stock_composer();
    composer.set_effect_kinds(&[]);
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    assert_eq!(
        composer.with_scene(|scene| scene.value(seat, "effect_kinds").cloned()),
        Some(TreeValue::ListText(Vec::new())),
        "a kind set before the seat exists reaches the seat when it is made"
    );
    composer.take_tree_changes();

    composer.set_effect_kinds(Pass::NAMES);
    let want = TreeValue::ListText(Pass::NAMES.iter().map(|kind| (*kind).to_owned()).collect());
    assert!(composer.take_tree_changes().iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value } if *node == seat && key == "effect_kinds" && *value == want
    )));
    composer.set_effect_kinds(Pass::NAMES);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged list is not republished"
    );
}

#[test]
fn a_shader_pass_on_the_software_renderer_is_accepted_and_listed_skipped() {
    use gnoblin_render::ShaderPass;
    let (mut composer, _output) = screen();
    let node = NodeId::new(0x4000);
    composer.with_scene(|scene| {
        scene
            .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("the node");
        let mut chain = blur_and_shadow();
        chain
            .push(Pass::Shader(ShaderPass::new("fn effect() {}", &[], true)))
            .expect("a shader");
        scene.set_effects(node, chain).expect("the effects stay in the list");
    });
    composer.take_tree_changes();

    composer.set_effect_kinds(crate::render::renderer::RendererKind::Software.effect_kinds());
    composer.publish_skipped_effects();
    assert_eq!(
        queued_skipped(&mut composer, node),
        Some(vec![
            "blur: unsupported-by-renderer".to_owned(),
            "shader: unsupported-by-renderer".to_owned(),
        ]),
        "the software renderer has no compiler: the shader is left undrawn and named, like the blur"
    );
}

#[test]
fn the_main_seat_publishes_whether_user_shaders_are_accepted() {
    let mut composer = stock_composer();
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    assert_eq!(
        composer.with_scene(|scene| scene.value(seat, "user_shaders").cloned()),
        Some(TreeValue::Bool(true)),
        "shaders are accepted unless the setting switches them off"
    );
    let mut composer = stock_composer();
    composer.set_user_shaders(false);
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    assert_eq!(
        composer.with_scene(|scene| scene.value(seat, "user_shaders").cloned()),
        Some(TreeValue::Bool(false)),
        "a setting made before the seat exists reaches the seat when it is made"
    );
    composer.take_tree_changes();
    composer.set_user_shaders(true);
    assert!(composer.take_tree_changes().iter().any(|change| matches!(
        change,
        TreeChange::Set { node, key, value } if *node == seat && key == "user_shaders" && *value == TreeValue::Bool(true)
    )));
    composer.set_user_shaders(true);
    assert!(
        composer.take_tree_changes().is_empty(),
        "an unchanged value is not republished"
    );
}

// ---- the frame-cost watchdog of the shader pass (design 37.2) ---------------------------------------------------------------

mod shader_watchdog {
    use std::time::Duration;

    use gnoblin_render::ShaderPass;
    use gnoblin_render_frame::shader::watchdog::{DISABLE_FOR, ShaderRefusals};
    use gnoblin_render_frame::shader::{FRAMES_OVER_BUDGET, SHADER_FRAME_BUDGET, STRIKES_TO_REFUSE};

    use super::*;

    const SOURCE: &str = "fn effect() { /* a shader a test says is slow */ }";

    /// What a renderer says about a frame it drew. The fake draws nothing: it reports what the test makes up.
    #[derive(Clone, Copy)]
    struct Drew {
        /// What each shader source of the frame cost, if the renderer measured one.
        cost: Option<Duration>,
        /// Whether the device was lost in the draw.
        lost: bool,
        /// A source the renderer could not build a program for, with its reason.
        unbuildable: Option<&'static str>,
    }

    const SLOW: Drew = Drew {
        cost: Some(Duration::from_millis(80)),
        lost: false,
        unbuildable: None,
    };
    const FAST: Drew = Drew {
        cost: Some(Duration::from_millis(4)),
        lost: false,
        unbuildable: None,
    };
    const LOST: Drew = Drew {
        cost: None,
        lost: true,
        unbuildable: None,
    };

    /// A canvas that draws a border and the shader pass, on a composer whose renderer draws every kind.
    fn shaded() -> (Composer, OutputId, NodeId) {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(2);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 77)),
        );
        composer.with_scene(|scene| {
            let mut chain = Effects::new();
            chain
                .push(Pass::Border(gnoblin_render::Border {
                    width: 1.0,
                    offset: 0.0,
                    inset: false,
                    color: Color::BLACK,
                }))
                .expect("a border");
            chain
                .push(Pass::Shader(ShaderPass::new(SOURCE, &[], false)))
                .expect("a shader");
            scene.set_effects(canvas, chain).expect("the effects");
        });
        composer.set_effect_kinds(Pass::NAMES);
        (composer, output, canvas)
    }

    /// Draws one frame of the output the way a backend does with a renderer that reports `drew`: the pass names of the frame as
    /// the renderer received it, and then the host told the draw is over.
    fn draw(composer: &mut Composer, output: OutputId, drew: Drew) -> Vec<&'static str> {
        composer.damage_output(output);
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        let names: Vec<&'static str> = frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(surface) => Some(surface.effects.passes().iter().map(Pass::name)),
                _ => None,
            })
            .flatten()
            .collect();
        for key in frame.shader_keys() {
            if let Some(cost) = drew.cost {
                frame.shader_timings().record(key, cost);
            }
            if let Some(reason) = drew.unbuildable {
                frame.shader_timings().record_refused(key, reason);
            }
        }
        if drew.lost {
            frame.shader_timings().record_loss();
        }
        composer.shader_drawn(&mut frame);
        composer.publish_skipped_effects();
        composer.advance_motion(16);
        names
    }

    fn skipped(composer: &mut Composer, node: NodeId) -> Option<Vec<String>> {
        queued_skipped(composer, node)
    }

    #[test]
    fn the_budget_and_the_counts_are_the_constants_of_the_design() {
        assert_eq!(SHADER_FRAME_BUDGET, Duration::from_millis(50));
        assert_eq!((FRAMES_OVER_BUDGET, STRIKES_TO_REFUSE), (3, 3));
    }

    #[test]
    fn three_frames_over_budget_switch_a_shader_off_and_its_node_draws_without_it() {
        let (mut composer, output, canvas) = shaded();
        assert_eq!(draw(&mut composer, output, SLOW), ["border", "shader"]);
        assert_eq!(draw(&mut composer, output, SLOW), ["border", "shader"]);
        assert_eq!(skipped(&mut composer, canvas), None, "two slow frames name nothing yet");
        assert_eq!(
            draw(&mut composer, output, SLOW),
            ["border", "shader"],
            "the third slow frame was drawn with the pass"
        );
        assert_eq!(
            skipped(&mut composer, canvas),
            Some(vec!["shader: over-budget".to_owned()]),
            "the strike names the pass with its reason"
        );
        assert_eq!(
            draw(&mut composer, output, FAST),
            ["border"],
            "the next frame leaves the pass out and keeps the node's other effects"
        );
    }

    #[test]
    fn a_strike_repaints_the_node_without_the_pass_though_nothing_else_changed() {
        let (mut composer, output, _canvas) = shaded();
        for _ in 0..FRAMES_OVER_BUDGET {
            draw(&mut composer, output, SLOW);
        }
        // No `damage_output` here, as `draw` does: only what the strike itself damaged.
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        let changed = composer.compose(output, &mut frame);
        composer.end_frame();
        assert!(
            changed,
            "the pixels the slow pass drew are replaced without anyone asking"
        );
        assert!(
            frame.shader_keys().is_empty(),
            "and the frame that replaces them has no shader pass"
        );
        assert!(!frame.layers().is_empty());
    }

    /// The next frame of the output as the host asks for it, with no damage added: `None` when nothing changed.
    fn idle_frame(composer: &mut Composer, output: OutputId) -> Option<OutputFrame> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        let changed = composer.compose(output, &mut frame);
        composer.end_frame();
        changed.then_some(frame)
    }

    #[test]
    fn a_struck_pass_comes_back_on_an_output_with_nothing_else_to_draw_when_its_time_is_up() {
        let (mut composer, output, _canvas) = shaded();
        assert_eq!(composer.shader_rearm_at(), None, "nothing struck: no time to wake for");
        for _ in 0..FRAMES_OVER_BUDGET {
            draw(&mut composer, output, SLOW);
        }
        let at = composer
            .shader_rearm_at()
            .expect("a struck pass has a time it comes back at");
        let frame = idle_frame(&mut composer, output).expect("the strike repaints the node without the pass");
        assert!(frame.shader_keys().is_empty());
        assert!(
            idle_frame(&mut composer, output).is_none(),
            "and then the output is idle"
        );
        assert_eq!(
            composer.shader_rearm_at(),
            Some(at),
            "the time does not move while the clock stands"
        );

        // The clock runs to a millisecond before the time: still idle, still off, still waiting for the same time.
        let wait = u32::try_from((at - composer.shader_clock()).as_millis()).unwrap();
        composer.advance_motion(wait - 1);
        assert!(
            idle_frame(&mut composer, output).is_none(),
            "nothing repaints before the time is up"
        );
        assert_eq!(composer.shader_rearm_at(), Some(at));

        // The last millisecond draws the pass again, with no write and no other damage.
        composer.advance_motion(1);
        let frame = idle_frame(&mut composer, output).expect("the return of the pass damages its node");
        assert_eq!(
            frame.shader_keys(),
            [gnoblin_render::ShaderKey::of(SOURCE)],
            "the frame that follows carries the pass"
        );
        assert_eq!(
            composer.shader_rearm_at(),
            None,
            "a pass that is back leaves no time to wake for"
        );
        assert!(
            idle_frame(&mut composer, output).is_none(),
            "and one repaint is all it costs"
        );
        composer.advance_motion(1_000_000);
        assert!(
            idle_frame(&mut composer, output).is_none(),
            "no repeated repaint on an idle output"
        );
    }

    #[test]
    fn a_refused_source_leaves_no_time_to_wake_for() {
        let (mut composer, output, _canvas) = shaded();
        for strike in 1..=STRIKES_TO_REFUSE {
            for _ in 0..FRAMES_OVER_BUDGET {
                draw(&mut composer, output, SLOW);
            }
            if strike < STRIKES_TO_REFUSE {
                let at = composer.shader_rearm_at().expect("a struck pass has a time");
                composer.advance_motion(u32::try_from((at - composer.shader_clock()).as_millis()).unwrap());
            }
        }
        assert!(
            composer
                .shader_refusals()
                .refuses(gnoblin_render::ShaderKey::of(SOURCE))
        );
        assert_eq!(composer.shader_rearm_at(), None, "a refusal never comes back");
    }

    #[test]
    fn a_frame_within_the_budget_ends_the_run_of_slow_ones() {
        let (mut composer, output, canvas) = shaded();
        for drew in [SLOW, SLOW, FAST, SLOW, SLOW, FAST, SLOW, SLOW] {
            assert_eq!(draw(&mut composer, output, drew), ["border", "shader"]);
        }
        assert_eq!(skipped(&mut composer, canvas), None);
    }

    #[test]
    fn a_struck_pass_comes_back_when_its_time_is_up_and_the_third_strike_refuses_the_source_for_the_session() {
        let (mut composer, output, canvas) = shaded();
        let refusals = ShaderRefusals::default();
        composer.set_shader_refusals(refusals.clone());
        let key = gnoblin_render::ShaderKey::of(SOURCE);
        for strike in 1..=STRIKES_TO_REFUSE {
            for _ in 0..FRAMES_OVER_BUDGET {
                assert_eq!(
                    draw(&mut composer, output, SLOW),
                    ["border", "shader"],
                    "strike {strike}"
                );
            }
            let named = skipped(&mut composer, canvas);
            if strike < STRIKES_TO_REFUSE {
                assert_eq!(
                    named,
                    Some(vec!["shader: over-budget".to_owned()]),
                    "strike {strike} is named"
                );
                assert_eq!(
                    draw(&mut composer, output, FAST),
                    ["border"],
                    "off after strike {strike}"
                );
                assert!(!refusals.refuses(key), "strike {strike} does not refuse");
                composer.advance_motion(u32::try_from(DISABLE_FOR.as_millis()).unwrap());
                assert_eq!(
                    draw(&mut composer, output, FAST),
                    ["border", "shader"],
                    "back on after {DISABLE_FOR:?}"
                );
                assert_eq!(
                    skipped(&mut composer, canvas),
                    Some(Vec::new()),
                    "the name is withdrawn with the pass"
                );
            } else {
                assert_eq!(
                    named,
                    Some(vec!["shader: refused-for-session".to_owned()]),
                    "the third strike refuses the source and says so"
                );
            }
        }
        assert!(
            refusals.refuses(key),
            "the list the tree's write path holds now refuses the source"
        );
        assert_eq!(draw(&mut composer, output, FAST), ["border"]);
        composer.advance_motion(1_000_000);
        assert_eq!(
            draw(&mut composer, output, FAST),
            ["border"],
            "a refusal does not wear off with time"
        );
        assert_eq!(skipped(&mut composer, canvas), None, "and the name stays");
    }

    #[test]
    fn a_device_loss_with_a_shader_in_the_frame_strikes_its_source_first() {
        let (mut composer, output, canvas) = shaded();
        assert_eq!(draw(&mut composer, output, LOST), ["border", "shader"]);
        assert_eq!(
            skipped(&mut composer, canvas),
            Some(vec!["shader: over-budget".to_owned()]),
            "one loss is one strike, with no three frames to wait for"
        );
        assert_eq!(
            draw(&mut composer, output, FAST),
            ["border"],
            "the pass is out of the frame that follows the loss"
        );
        // Two more losses, a strike each, refuse the source.
        let refusals = composer.shader_refusals();
        for _ in 1..STRIKES_TO_REFUSE {
            composer.advance_motion(u32::try_from(DISABLE_FOR.as_millis()).unwrap());
            assert_eq!(draw(&mut composer, output, LOST), ["border", "shader"]);
        }
        assert!(refusals.refuses(gnoblin_render::ShaderKey::of(SOURCE)));
    }

    #[test]
    fn a_device_loss_in_a_frame_with_no_shader_strikes_nothing() {
        let (mut composer, output, canvas) = shaded();
        composer.with_scene(|scene| scene.set_effects(canvas, Effects::new()).expect("the effects"));
        assert!(draw(&mut composer, output, LOST).is_empty());
        assert_eq!(skipped(&mut composer, canvas), None);
        assert!(
            !composer
                .shader_refusals()
                .refuses(gnoblin_render::ShaderKey::of(SOURCE))
        );
    }

    #[test]
    fn a_source_a_renderer_cannot_build_is_named_with_the_renderers_reason_and_takes_no_strike() {
        let (mut composer, output, canvas) = shaded();
        let broken = Drew {
            cost: None,
            lost: false,
            unbuildable: Some("the pipeline could not be created"),
        };
        for _ in 0..(FRAMES_OVER_BUDGET * STRIKES_TO_REFUSE + 1) {
            assert_eq!(draw(&mut composer, output, broken), ["border", "shader"]);
        }
        assert_eq!(
            skipped(&mut composer, canvas),
            Some(vec!["shader: the pipeline could not be created".to_owned()])
        );
        assert!(
            !composer
                .shader_refusals()
                .refuses(gnoblin_render::ShaderKey::of(SOURCE))
        );
    }

    #[test]
    fn a_struck_source_is_struck_on_every_node_that_carries_it() {
        let (mut composer, output, first) = shaded();
        let second = NodeId::new(3);
        add_canvas(
            &mut composer,
            second,
            None,
            Rect::new(100, 100, 40, 30),
            Some(fill_list(40.0, 30.0, 12)),
        );
        let chain = composer.with_scene(|scene| scene.effects(first).cloned().expect("the first node's chain"));
        composer.with_scene(|scene| scene.set_effects(second, chain).expect("the effects"));
        for _ in 0..FRAMES_OVER_BUDGET {
            assert_eq!(
                draw(&mut composer, output, SLOW),
                ["border", "shader", "border", "shader"]
            );
        }
        assert_eq!(draw(&mut composer, output, FAST), ["border", "border"]);
        let _ = composer.take_tree_changes();
        composer.publish_skipped_effects();
        for node in [first, second] {
            assert_eq!(
                composer.with_scene(|scene| scene.value(node, "skipped").cloned()),
                Some(TreeValue::ListText(vec!["shader: over-budget".to_owned()])),
                "node {node:?}"
            );
        }
    }
}

// ---- the shell turns the compositor's own window animations off (the seat's `builtin_motion`) ------------------------

/// Makes the main seat, lets a shell hold the tree and has it write the seat's `builtin_motion`: what a shell does to
/// stop the compositor animating windows, or to leave it doing so (`on`).
fn shell_writes_builtin_motion(composer: &mut Composer, on: bool) -> NodeId {
    let seat = composer.add_main_seat("seat0").expect("a seat node");
    composer.with_scene(|scene| {
        scene.set_holder_attached(true);
        scene
            .set_value(seat, "builtin_motion", TreeValue::Bool(on))
            .expect("the seat holds the value");
    });
    composer.follow_placement();
    seat
}

/// A mapped window with a picture its buffer keeps, so a close or a hide has something to fade.
fn window_with_picture(composer: &mut Composer) -> WindowId {
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    composer.set_window_source(window, Some(SurfaceSource::Snapshot(picture())));
    window
}

#[test]
fn a_window_maps_opaque_at_once_while_the_shell_has_turned_the_builtin_motion_off() {
    let (mut composer, _output) = screen();
    shell_writes_builtin_motion(&mut composer, false);
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        drawn_pose(&composer, window).1,
        1.0,
        "no fade in: the shell plays its own animation or none"
    );
    assert!(composer.tracks.is_empty(), "the compositor started no track");
}

#[test]
fn a_shell_that_leaves_the_builtin_motion_on_still_gets_the_fade_in() {
    let (mut composer, _output) = screen();
    shell_writes_builtin_motion(&mut composer, true);
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        drawn_pose(&composer, window).1,
        0.0,
        "a window starts transparent and fades in"
    );
    assert!(!composer.tracks.is_empty());
}

#[test]
fn the_builtin_motion_switch_is_ignored_while_no_shell_holds_the_tree() {
    let (mut composer, _output) = screen();
    let seat = shell_writes_builtin_motion(&mut composer, false);
    // The shell is gone, but a value it wrote is still on the seat: the compositor's own desktop animates as ever.
    composer.with_scene(|scene| scene.set_holder_attached(false));
    assert!(!composer.builtin_motion_off());
    composer.with_scene(|scene| {
        scene
            .set_value(seat, "builtin_motion", TreeValue::Bool(false))
            .expect("the seat holds the value");
    });
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    assert_eq!(
        drawn_pose(&composer, window).1,
        0.0,
        "the fallback desktop fades a window in"
    );
}

#[test]
fn a_closed_window_goes_at_once_with_the_builtin_motion_off_and_fades_out_with_it_on() {
    let (mut off, _output) = screen();
    shell_writes_builtin_motion(&mut off, false);
    let window = window_with_picture(&mut off);
    off.close_window(window);
    assert!(!off.windows.contains_key(&window), "the window is gone with no fade");

    let (mut on, _output) = screen();
    let window = window_with_picture(&mut on);
    on.close_window(window);
    assert!(
        on.windows[&window].closing.is_some(),
        "the fallback fades the window out"
    );
}

#[test]
fn an_unmapped_or_hidden_window_loses_its_node_at_once_with_the_builtin_motion_off() {
    let (mut off, _output) = screen();
    shell_writes_builtin_motion(&mut off, false);
    let window = window_with_picture(&mut off);
    off.set_window_visible(window, false);
    assert!(off.windows[&window].closing.is_none(), "no fade runs");
    assert!(off.window_matrix(window).is_none(), "the node went at once");
    assert!(
        off.windows[&window].content.is_some(),
        "the content is kept to show again"
    );

    let (mut unmapped, _output) = screen();
    shell_writes_builtin_motion(&mut unmapped, false);
    let window = window_with_picture(&mut unmapped);
    unmapped.unmap_window(window);
    assert!(unmapped.windows[&window].closing.is_none());
    assert!(unmapped.window_matrix(window).is_none());

    let (mut on, _output) = screen();
    let window = window_with_picture(&mut on);
    on.set_window_visible(window, false);
    assert!(
        on.windows[&window].closing.is_some(),
        "the fallback fades a hidden window out"
    );
}

#[test]
fn a_placement_change_lands_at_once_with_the_builtin_motion_off_and_tweens_with_it_on() {
    let scaled = |composer: &mut Composer| {
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 100, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        let mut base = gnoblin_types::Transform::IDENTITY;
        base.scale = Vec3::new(2.0, 2.0, 1.0);
        composer.place_window_animated(window, Rect::new(100, 100, 40, 50), base);
        drawn_pose(composer, window).0.width
    };
    let (mut off, _output) = screen();
    shell_writes_builtin_motion(&mut off, false);
    assert_eq!(scaled(&mut off), 80, "the scale is already at its target");

    let (mut on, _output) = screen();
    shell_writes_builtin_motion(&mut on, true);
    assert_eq!(scaled(&mut on), 40, "the tween has not moved yet");
}

#[test]
fn the_shells_own_timed_writes_still_play_with_the_builtin_motion_off() {
    let (mut composer, _output) = screen();
    shell_writes_builtin_motion(&mut composer, false);
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    composer.apply_shell_motion(vec![Motion::Play(Animation {
        node,
        key: "opacity".to_owned(),
        value: TreeValue::Double(0.2),
        seconds: 0.2,
        delay: 0.0,
        group: None,
        samples: vec![0.0, 0.5, 1.0],
    })]);
    assert_eq!(composer.shell_tracks_running(), 1, "the shell's animation is a track");
    composer.advance_motion(100);
    let (_, halfway) = drawn_pose(&composer, window);
    assert!(
        halfway < 1.0 && halfway > 0.2,
        "the shell's animation is part way: {halfway}"
    );
    composer.advance_motion(100);
    assert_eq!(drawn_pose(&composer, window).1, 0.2, "and lands on its target");
}

#[test]
fn a_shell_that_lets_go_of_the_tree_leaves_the_compositor_animating_and_every_window_unpaced() {
    let (mut composer, _output) = screen();
    let seat = shell_writes_builtin_motion(&mut composer, false);
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    let node = node_of(&composer, window);
    composer.with_scene(|scene| {
        for (key, value) in [
            ("frame_policy", TreeValue::Text("hold".to_owned())),
            ("frame_rate", TreeValue::Double(60.0)),
            ("report_presented", TreeValue::Bool(true)),
        ] {
            scene.set_value(node, key, value).expect("the node holds the value");
        }
    });
    composer.take_tree_changes();
    composer.with_scene(|scene| scene.set_holder_attached(false));
    composer.follow_placement();
    let changes = composer.take_tree_changes();
    let set = |node: NodeId, key: &str, value: TreeValue| TreeChange::Set {
        node,
        key: key.to_owned(),
        value,
    };
    for expected in [
        set(seat, "builtin_motion", TreeValue::Bool(true)),
        set(node, "frame_policy", TreeValue::Text("auto".to_owned())),
        set(node, "frame_rate", TreeValue::Double(30.0)),
        set(node, "report_presented", TreeValue::Bool(false)),
    ] {
        assert!(changes.contains(&expected), "{expected:?} was published: {changes:?}");
    }
    composer.with_scene(|scene| {
        assert_eq!(scene.value(seat, "builtin_motion"), Some(&TreeValue::Bool(true)));
        assert_eq!(
            scene.value(node, "frame_policy"),
            Some(&TreeValue::Text("auto".to_owned()))
        );
    });
}

// ---- the shell paces a window's frame callbacks (`frame_policy`) and reads its presentation ----------------------------

/// A window with a node, the shell holding the tree, and `values` written on the node.
fn shell_paced_window(composer: &mut Composer, values: &[(&str, TreeValue)]) -> (WindowId, NodeId) {
    let window = WindowId::new(1);
    composer.insert_window(window, Rect::new(20, 30, 40, 50));
    composer.set_window_content(window, Some(content()));
    let node = node_of(composer, window);
    composer.with_scene(|scene| {
        scene.set_holder_attached(true);
        for (key, value) in values {
            scene
                .set_value(node, key, value.clone())
                .expect("the node holds the value");
        }
    });
    (window, node)
}

#[test]
fn a_window_is_paced_as_the_shell_wrote_it_and_auto_when_it_wrote_nothing() {
    let (mut composer, _output) = screen();
    let (window, node) = shell_paced_window(&mut composer, &[]);
    assert_eq!(composer.window_frame_policy(window), FramePolicy::Auto, "the default");
    let text = |policy: &str| TreeValue::Text(policy.to_owned());
    composer.with_scene(|scene| scene.set_value(node, "frame_policy", text("hold")).expect("held"));
    assert_eq!(composer.window_frame_policy(window), FramePolicy::Hold);
    composer.with_scene(|scene| {
        scene
            .set_value(node, "frame_policy", text("throttle"))
            .expect("throttled")
    });
    assert_eq!(
        composer.window_frame_policy(window),
        FramePolicy::Throttle { interval_ms: 33 },
        "a throttled window the shell gave no rate runs at the default 30 Hz"
    );
    composer.with_scene(|scene| {
        scene
            .set_value(node, "frame_rate", TreeValue::Double(20.0))
            .expect("a rate")
    });
    assert_eq!(
        composer.window_frame_policy(window),
        FramePolicy::Throttle { interval_ms: 50 }
    );
    composer.with_scene(|scene| {
        scene
            .set_value(node, "frame_rate", TreeValue::Double(5000.0))
            .expect("a rate")
    });
    assert_eq!(
        composer.window_frame_policy(window),
        FramePolicy::Throttle { interval_ms: 1 },
        "the interval is never below a millisecond"
    );
    composer.with_scene(|scene| scene.set_value(node, "frame_policy", text("auto")).expect("auto"));
    assert_eq!(composer.window_frame_policy(window), FramePolicy::Auto);
}

#[test]
fn a_policy_the_shell_wrote_means_nothing_while_no_shell_holds_the_tree() {
    let (mut composer, _output) = screen();
    let (window, _node) = shell_paced_window(&mut composer, &[("frame_policy", TreeValue::Text("hold".to_owned()))]);
    assert_eq!(composer.window_frame_policy(window), FramePolicy::Hold);
    composer.with_scene(|scene| scene.set_holder_attached(false));
    assert_eq!(
        composer.window_frame_policy(window),
        FramePolicy::Auto,
        "with no shell every window is answered as it always was"
    );
    assert_eq!(
        composer.window_frame_policy(WindowId::new(99)),
        FramePolicy::Auto,
        "a window the composer does not hold"
    );
}

#[test]
fn presentation_is_stamped_only_on_a_window_the_shell_asked_about_and_only_when_it_changes() {
    let (mut composer, _output) = screen();
    let (window, node) = shell_paced_window(&mut composer, &[]);
    composer.take_tree_changes();
    composer.stamp_presented(window, 1_234);
    assert!(
        composer.take_tree_changes().is_empty(),
        "a window the shell did not ask about is never stamped"
    );

    composer.with_scene(|scene| {
        scene
            .set_value(node, "report_presented", TreeValue::Bool(true))
            .expect("the node holds the value");
    });
    composer.stamp_presented(window, 1_234);
    assert_eq!(
        composer.take_tree_changes(),
        vec![TreeChange::Set {
            node,
            key: "presented".to_owned(),
            value: TreeValue::Uint(1_234),
        }]
    );
    composer.stamp_presented(window, 1_234);
    assert!(
        composer.take_tree_changes().is_empty(),
        "the same millisecond is not published twice"
    );
    composer.stamp_presented(window, 1_250);
    assert_eq!(composer.take_tree_changes().len(), 1, "the next frame is");
    composer.stamp_presented(WindowId::new(99), 1_300);
    assert!(
        composer.take_tree_changes().is_empty(),
        "a window the composer does not hold"
    );
}
