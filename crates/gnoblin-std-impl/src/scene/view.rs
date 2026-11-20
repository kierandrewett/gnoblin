//! The class of a view: a node that draws a client or a workspace in the scene without moving it.
//!
//! A client is drawn by pointing the node's `source` at it. A workspace is drawn by a mirror (`mirror.rs`): a holder node under the
//! view with one child view for each window on the workspace. Pointing the view at one ends the other.

use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Res, Val};

use super::Kind;
use super::camera::finite;
use super::node::{enumeration, node_class, node_of};
use super::registry::NodeHandle;
use crate::animation::staging::Prop;

/// The places of `content_fit`, counted from one.
const FITS: u32 = 3;

impl NodeHandle {
    /// The client the view draws, by node, or zero.
    fn source(&self) -> Res<u64> {
        Ok(match self.read("source")? {
            Some(Prop::Node(node)) => node,
            _ => 0,
        })
    }

    /// A write of a size that the mirror, if there is one, follows.
    fn put_size(&self, key: &str, value: f64) -> Res<()> {
        self.put(key, Prop::Number(value))?;
        self.world.mirror_sync(self.node);
        Ok(())
    }
}

pub(super) fn register(builder: &mut Builder) {
    let _ = node_class(builder, "view", Kind::View)
        .get("camera", |h, cx| match h.read("camera")? {
            Some(Prop::Node(node)) if node != 0 => match h.world.entry(node) {
                Some(entry) if entry.kind == Kind::Camera3D => cx.handout(entry.obj),
                _ => Ok(Val::None),
            },
            _ => Ok(Val::None),
        })
        .set("camera", |h, cx, value| {
            let node = node_of(cx, value.object()?)?.0;
            h.put("camera", Prop::Node(node))
        })
        .get("size", |h, _| {
            Ok(Val::Record {
                class: 0,
                fields: vec![Val::Double(h.number("width")?), Val::Double(h.number("height")?)],
            })
        })
        .set("size", |h, _, value| {
            let fields = value.record()?;
            let at = |index: usize| fields.get(index).map_or(Ok(0.0), Val::double);
            let label = h.label();
            // Both are checked before either is written.
            let width = finite(&label, "size.width", at(0)?).map_err(Fail::from)?;
            let height = finite(&label, "size.height", at(1)?).map_err(Fail::from)?;
            h.put("width", Prop::Number(width))?;
            h.put_size("height", height)
        })
        .get("width", |h, _| Ok(Val::Double(h.number("width")?)))
        .set("width", |h, _, value| h.put_size("width", value.double()?))
        .get("height", |h, _| Ok(Val::Double(h.number("height")?)))
        .set("height", |h, _, value| h.put_size("height", value.double()?))
        .get("fit", |h, _| {
            Ok(enumeration(h.world.entry(h.node).map_or(FITS, |entry| entry.fit)))
        })
        .set("fit", |h, _, value| {
            let fit = value.enum_value()?;
            if fit == 0 || fit > FITS {
                return Err(Fail::type_error("not a value of `content_fit`"));
            }
            h.world.tweak(h.node, |entry| entry.fit = fit);
            h.world.mirror_sync(h.node);
            Ok(())
        })
        .get("source", |h, _| Ok(Val::Uint(h.source()?)))
        .set("source", |h, _, value| {
            let node = value.uint()?;
            h.world.mirror_stop(h.node);
            h.put("source", Prop::Node(node))
        })
        .get("workspace", |h, _| {
            Ok(Val::Text(h.world.mirrors_workspace(h.node).unwrap_or_default()))
        })
        .set("workspace", |h, _, value| {
            let id = value.text()?;
            h.world.mirror_stop(h.node);
            if id.is_empty() {
                return Ok(());
            }
            if !h.world.has_workspace(id) {
                return Err(Fail::no_such(format!(
                    "{}.workspace: there is no workspace `{id}`",
                    h.label()
                )));
            }
            h.put("source", Prop::Node(0))?;
            h.world.mirror_start(h.node, id)?;
            h.touched();
            Ok(())
        })
        .get("fps", |h, _| Ok(Val::Uint(h.number("fps")? as u64)))
        .set("fps", |h, _, value| h.put("fps", Prop::Number(value.double()?)))
        .get("orphaned", |h, _| {
            Ok(Val::Bool(
                h.world.mirrors_workspace(h.node).is_none() && h.source()? == 0,
            ))
        })
        .method("clear_camera", |h, _, _| {
            h.put("camera", Prop::Node(0))?;
            Ok(Val::None)
        });
}
