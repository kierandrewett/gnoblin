//! `client`: a toplevel window the compositor manages, as a handle over its tree node.
//!
//! Reads go through the tree as this tick sees it, so a write the shell made is read back at once, and through the values the window had
//! when it went once it is gone. A write is a write of the node's property, checked here so the failure names the member.

use gnoblin_core_abi as abi;
use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{ErrorCode, Fail, Res, Val};
use gnoblin_tree_client::tree_mirror::{MirrorValue, TxAnimation};

use super::values::{self, enum_name, fields_of, finite, nodes_of, rect, size};
use super::world::{ClientState, detached, gone, write_fail};
use super::{handout, listener, properties, with_world};
use crate::session;
use crate::stage;
use crate::tree::{LINEAR, timed_animation};

type Class<'a> = ClassBuilder<'a, ClientState>;

impl ClientState {
    /// The failure for a member of a window that is gone.
    fn gone(&self, member: &str) -> Fail {
        gone("Client", self.node, member, "client")
    }

    /// Writes a property of the node. A plain write is offered to the write stage hook first (a `requestTransition` callback's writes
    /// are held by it and sent together when the callback completes), and goes to the tree when no hook keeps it. A timed write
    /// (`cx.timing()`) is sent with the animation its timing says and is never offered.
    fn write(&self, cx: &Cx<'_>, key: &str, value: MirrorValue) -> Res<()> {
        if !self.alive {
            return Err(self.gone(key));
        }
        let timing = cx.timing();
        if timing.is_none() && stage::offer(self.node, key, &value) {
            return Ok(());
        }
        session::with(|session| {
            let tree = session.tree().map_err(detached)?;
            let target = format!("Client#{}.{key}", self.node);
            let animation = match timing {
                None => TxAnimation::default(),
                Some(timing) => {
                    let samples = if timing.samples.is_empty() {
                        &LINEAR[..]
                    } else {
                        &timing.samples[..]
                    };
                    timed_animation(tree, &target, timing.seconds, timing.delay, timing.tween, samples)
                        .map_err(Fail::type_error)?
                }
            };
            tree.set(self.node, key, value, animation)
                .map_err(|error| write_fail(&target, &error))
        })
    }

    /// Calls a method of the node.
    fn call(&self, method: &str) -> Res<()> {
        let target = format!("Client#{}.{method}()", self.node);
        if !self.alive {
            return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("{target}: the client is gone")));
        }
        session::with(|session| {
            session
                .tree()
                .map_err(detached)?
                .call(self.node, method, Vec::new(), String::new())
                .map(|_| ())
                .map_err(|error| write_fail(&target, &error))
        })
    }

    /// A number that must be finite, as the property's value.
    fn number(&self, key: &str, value: &Val) -> Res<MirrorValue> {
        finite(value, &format!("Client#{}.{key}", self.node)).map(MirrorValue::Double)
    }

    /// Three finite numbers.
    fn triple(&self, key: &str, value: &Val) -> Res<Vec<f64>> {
        let target = format!("Client#{}.{key}", self.node);
        let parts = value.list()?;
        let numbers: Vec<f64> = parts.iter().filter_map(|part| part.double().ok()).collect();
        if parts.len() != 3 || numbers.len() != 3 || numbers.iter().any(|number| !number.is_finite()) {
            return Err(Fail::type_error(format!("{target}: must be three finite numbers")));
        }
        Ok(numbers)
    }
}

fn text<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    class.get(name, move |state, _| Ok(Val::Text(state.read(key).string())))
}

fn flag<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    class.get(name, move |state, _| Ok(Val::Bool(state.read(key).flag())))
}

fn flag_rw<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    flag(class, name, key).set(name, move |state, cx, value| {
        state.write(cx, key, MirrorValue::Bool(value.bool()?))
    })
}

fn number<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    class.get(name, move |state, _| Ok(Val::Double(state.read(key).number())))
}

fn number_rw<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    number(class, name, key).set(name, move |state, cx, value| {
        state.write(cx, key, state.number(key, &value)?)
    })
}

fn triple_rw<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    class
        .get(name, move |state, _| {
            Ok(Val::List(
                state.read(key).numbers().into_iter().map(Val::Double).collect(),
            ))
        })
        .set(name, move |state, cx, value| {
            let parts = state.triple(key, &value)?;
            state.write(cx, key, MirrorValue::List(parts))
        })
}

/// Puts the window on top of every other: under the root, last. A fullscreen sibling keeps the top.
fn raise(state: &ClientState) -> Res<()> {
    let target = format!("Client#{}.raise()", state.node);
    if !state.alive {
        return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("{target}: the client is gone")));
    }
    let fullscreen = state.read("fullscreen").flag();
    let others: Vec<u64> = nodes_of("window")
        .into_iter()
        .filter(|node| *node != state.node)
        .collect();
    session::with(|session| {
        let tree = session.tree().map_err(detached)?;
        let root = tree.root();
        if tree.children_of(root).and_then(|children| children.last().copied()) != Some(state.node) {
            tree.insert(state.node, root, -1)
                .map_err(|error| write_fail(&target, &error))?;
        }
        if !fullscreen {
            for node in others {
                let over = tree
                    .get(node, "fullscreen")
                    .is_some_and(|value| matches!(value, MirrorValue::Bool(true)));
                if over {
                    tree.insert(node, root, -1)
                        .map_err(|error| write_fail(&target, &error))?;
                }
            }
        }
        Ok(())
    })
}

/// Puts the window under every other: under the root, first.
fn lower(state: &ClientState) -> Res<()> {
    let target = format!("Client#{}.lower()", state.node);
    if !state.alive {
        return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("{target}: the client is gone")));
    }
    session::with(|session| {
        let tree = session.tree().map_err(detached)?;
        let root = tree.root();
        if tree.children_of(root).and_then(|children| children.first().copied()) != Some(state.node) {
            tree.insert(state.node, root, 0)
                .map_err(|error| write_fail(&target, &error))?;
        }
        Ok(())
    })
}

/// Gives the window the keyboard focus: a write of the first seat's `focus`.
fn focus(state: &ClientState) -> Res<()> {
    let target = format!("Client#{}.focus()", state.node);
    if !state.alive {
        return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("{target}: the client is gone")));
    }
    let Some(seat) = nodes_of("seat").first().copied() else {
        return Err(Fail::refused(
            ErrorCode::Unavailable,
            format!("{target}: there is no seat to hold the focus"),
        ));
    };
    session::with(|session| {
        session
            .tree()
            .map_err(detached)?
            .set(seat, "focus", MirrorValue::Node(state.node), TxAnimation::default())
            .map_err(|error| write_fail(&target, &error))
    })
}

pub(super) fn register(builder: &mut Builder) {
    let mut class = builder
        .class::<ClientState>("client")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "a client is made by the compositor, not by a shell",
            ))
        })
        .get("id", |state, _| Ok(Val::Text(state.node.to_string())))
        .get("alive", |state, _| Ok(Val::Bool(state.alive)));
    for (name, key) in [
        ("app_id", "app_id"),
        ("title", "title"),
        ("decoration", "decoration"),
        ("decoration_wanted", "decoration_wanted"),
    ] {
        class = text(class, name, key);
    }
    for (name, key) in [
        ("focused", "focused"),
        ("unresponsive", "unresponsive"),
        ("from_launcher", "from_launcher"),
    ] {
        class = flag(class, name, key);
    }
    for (name, key) in [
        ("fullscreen", "fullscreen"),
        ("secure", "secure"),
        ("report_presented", "report_presented"),
        ("maximised", "maximised"),
        ("minimised", "minimised"),
        ("tiled", "tiled"),
        ("visible", "visible"),
    ] {
        class = flag_rw(class, name, key);
    }
    for (name, key) in [
        ("frame_rate", "frame_rate"),
        ("x", "x"),
        ("y", "y"),
        ("width", "width"),
        ("height", "height"),
        ("opacity", "opacity"),
        ("z", "z"),
    ] {
        class = number_rw(class, name, key);
    }
    class = triple_rw(class, "rotation", "rotation");
    class = triple_rw(class, "scale", "scale");
    let _ = class
        .get("presented", |state, _| {
            Ok(Val::Uint(state.read("presented").number() as u64))
        })
        .get("frame_policy", |state, cx| {
            let word = state.read("frame_policy").string();
            cx.enum_val("frame_policy", if word.is_empty() { "auto" } else { &word })
        })
        .set("frame_policy", |state, cx, value| {
            let word = enum_name(cx, &value)?;
            state.write(cx, "frame_policy", MirrorValue::Text(word))
        })
        .get("bounds", |state, cx| {
            rect(
                cx,
                state.read("x").number(),
                state.read("y").number(),
                state.read("width").number(),
                state.read("height").number(),
            )
        })
        .set("bounds", |state, cx, value| {
            let mut parts = Vec::new();
            for (name, field) in fields_of(cx, "rect", &value)? {
                parts.push((name, finite(field, &format!("Client#{}.bounds.{name}", state.node))?));
            }
            if !state.alive {
                return Err(state.gone("bounds"));
            }
            for (name, number) in parts {
                state.write(cx, name, MirrorValue::Double(number))?;
            }
            Ok(())
        })
        .get("committed_size", |state, cx| {
            if state.frozen.is_some() {
                return size(cx, state.read("width").number(), state.read("height").number());
            }
            size(
                cx,
                values::published(state.node, state.class, "width").number(),
                values::published(state.node, state.class, "height").number(),
            )
        })
        .get("position", |state, _| {
            Ok(Val::List(
                ["x", "y", "z"]
                    .iter()
                    .map(|key| Val::Double(state.read(key).number()))
                    .collect(),
            ))
        })
        .set("position", |state, cx, value| {
            let parts = state.triple("position", &value)?;
            for (key, number) in ["x", "y", "z"].into_iter().zip(parts) {
                state.write(cx, key, MirrorValue::Double(number))?;
            }
            Ok(())
        })
        .get("parent_window", |state, cx| {
            let parent = state.read("parent_window").node();
            if parent == 0 {
                return Ok(Val::None);
            }
            match with_world(cx, |world| world.clients.get(&parent).copied())? {
                Some(client) => handout(cx, client),
                None => Ok(Val::None),
            }
        })
        .get("properties", |state, cx| {
            properties::bindings_of(cx, cx.me(), &state.bindings)
        })
        .method("close", |state, _, _| state.call("close").map(|()| Val::None))
        .method("kill", |state, _, _| state.call("kill").map(|()| Val::None))
        .method("focus", |state, _, _| focus(state).map(|()| Val::None))
        .method("raise", |state, _, _| raise(state).map(|()| Val::None))
        .method("lower", |state, _, _| lower(state).map(|()| Val::None))
        .method("add_listener", |state, cx, args| {
            listener::add(cx, cx.me(), &mut state.listeners, &args[0])?;
            Ok(Val::None)
        })
        .method("remove_listener", |state, cx, args| {
            listener::remove(cx, &mut state.listeners, &args[0])?;
            Ok(Val::None)
        });
}
