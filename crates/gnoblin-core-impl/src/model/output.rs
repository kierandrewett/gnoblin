//! `output`: a connected display, the compositor's `screen` node, as a handle.
//!
//! Its `apply` sends one request to the compositor's output engine, and the answer a frame later settles the operation it returned and
//! tells `applied` to the output's listeners.

use gnoblin_core_abi as abi;
use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{ErrorCode, Fail, Res, Val};

use super::operation::OperationState;
use super::values::{fields_of, record, rect, size};
use super::world::{OutputNode, detached, write_fail};
use super::{handout, listener, properties, with_world};
use crate::session;

type Class<'a> = ClassBuilder<'a, OutputNode>;

/// The scale range the compositor accepts for an output.
const MIN_SCALE: f64 = 0.5;
const MAX_SCALE: f64 = 4.0;
/// The supersample range the compositor accepts for an output.
const MIN_SUPERSAMPLE: f64 = 1.0;
const MAX_SUPERSAMPLE: f64 = 16.0;
/// The factors the compositor recommends: whole powers of two give an exact box average.
const RECOMMENDED_SUPERSAMPLE: [f64; 3] = [1.0, 2.0, 4.0];

/// The compositor's names of the transforms, by the catalog's.
fn transform_word(name: &str) -> &str {
    match name {
        "rotate-90" => "90",
        "rotate-180" => "180",
        "rotate-270" => "270",
        other => other,
    }
}

/// The request text for `config` (one `key=value` a line), checking each value and naming `path` and the key in the failure.
fn request(cx: &Cx<'_>, config: &Val, path: &str) -> Res<(String, Option<String>)> {
    let mut lines = Vec::new();
    let mut notice = None;
    let fields = fields_of(cx, "output_config", config)?;
    let given = |name: &str| fields.iter().find(|(held, _)| *held == name).map(|(_, value)| *value);
    let present = |name: &str| given(name).filter(|value| !matches!(value, Val::None));
    if let Some(mode) = present("mode") {
        let mode = mode.text()?;
        if !(mode == "preferred" || is_mode(mode)) {
            return Err(Fail::type_error(format!(
                "{path}.mode must be \"preferred\" or \"WIDTHxHEIGHT@HERTZ\", such as \"3440x1440@144\""
            )));
        }
        lines.push(format!("mode={mode}"));
    }
    if let Some(scale) = present("scale") {
        let scale = scale.double()?;
        if !scale.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(&scale) {
            return Err(Fail::type_error(format!(
                "{path}.scale must be a number from {MIN_SCALE} to {MAX_SCALE}"
            )));
        }
        lines.push(format!("scale={scale}"));
    }
    if let Some(factor) = present("supersample") {
        let factor = factor.double()?;
        if !factor.is_finite() || !(MIN_SUPERSAMPLE..=MAX_SUPERSAMPLE).contains(&factor) {
            return Err(Fail::type_error(format!(
                "{path}.supersample must be a number from {MIN_SUPERSAMPLE} to {MAX_SUPERSAMPLE}, such as 1, 2 or 4"
            )));
        }
        if !RECOMMENDED_SUPERSAMPLE.contains(&factor) {
            notice = Some(format!("supersample {factor} is not recommended: use 1, 2 or 4"));
        }
        lines.push(format!("supersample={factor}"));
    }
    if let Some(transform) = present("transform") {
        let word = super::values::enum_name(cx, transform)?;
        lines.push(format!("transform={}", transform_word(&word)));
    }
    match (present("position_x"), present("position_y")) {
        (Some(x), Some(y)) => lines.push(format!("position={},{}", x.int()?, y.int()?)),
        (None, None) => {}
        _ => {
            return Err(Fail::type_error(format!(
                "{path}.position must be [x, y] or {{ x, y }} with whole numbers"
            )));
        }
    }
    if let Some(enabled) = present("enabled") {
        lines.push(format!("enabled={}", enabled.bool()?));
    }
    if let Some(mirror) = present("mirror_of") {
        lines.push(format!("mirror_of={}", mirror.text()?));
    }
    Ok((lines.join("\n"), notice))
}

/// Whether `text` is `WIDTHxHEIGHT@HERTZ`: digits, `x`, digits, `@`, digits with an optional fraction.
fn is_mode(text: &str) -> bool {
    let Some((size, rate)) = text.split_once('@') else {
        return false;
    };
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let rate_ok = match rate.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(rate),
    };
    digits(width) && digits(height) && rate_ok
}

/// Sends a request to the output's engine and returns the operation its answer settles.
fn send(cx: &Cx<'_>, state: &OutputNode, text: String, notice: Option<String>, target: &str) -> Res<Val> {
    if !state.alive {
        return Err(Fail::new(abi::E_NO_SUCH_NODE, format!("{target}: the output is gone")));
    }
    if let Some(notice) = notice {
        let fresh = with_world(cx, |world| world.noticed.insert(notice.clone()))?;
        if fresh {
            cx.log(abi::LOG_WARN, &notice);
        }
    }
    let token = session::with(|session| {
        session
            .tree()
            .map_err(detached)?
            .call(state.node, "apply", Vec::new(), text)
            .map_err(|error| write_fail(target, &error))
    })?;
    let me = cx.me();
    let operation = cx.spawn("output_operation", OperationState::for_output(me))?;
    // The library holds the operation until the answer settles it, whatever the shell does with the value it got.
    cx.hold(operation)?;
    with_world(cx, |world| world.applications.insert(token, (me, operation)))?;
    handout(cx, operation)
}

fn text<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    class.get(name, move |state, _| Ok(Val::Text(state.read(key).string())))
}

fn flag<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    class.get(name, move |state, _| Ok(Val::Bool(state.read(key).flag())))
}

fn number<'a>(class: Class<'a>, name: &'static str, key: &'static str) -> Class<'a> {
    class.get(name, move |state, _| Ok(Val::Double(state.read(key).number())))
}

/// The width and height of a mode `WxH@Hz`, zero for a mode that is not one.
fn mode_size(mode: &str) -> (f64, f64) {
    let Some((size, _)) = mode.split_once('@') else {
        return (0.0, 0.0);
    };
    let Some((width, height)) = size.split_once('x') else {
        return (0.0, 0.0);
    };
    (width.parse().unwrap_or(0.0), height.parse().unwrap_or(0.0))
}

pub(super) fn register(builder: &mut Builder) {
    let mut class = builder
        .class::<OutputNode>("output")
        .ctor(|_, _| {
            Err(Fail::refused(
                ErrorCode::Unsupported,
                "an output is made by the compositor, not by a shell",
            ))
        })
        .get("id", |state, _| Ok(Val::Text(state.node.to_string())))
        .get("alive", |state, _| Ok(Val::Bool(state.alive)));
    for (name, key) in [
        ("name", "name"),
        ("make", "make"),
        ("model", "model"),
        ("connector", "connector"),
        ("identity", "identity"),
        ("mode", "mode"),
        ("transform", "transform"),
    ] {
        class = text(class, name, key);
    }
    for (name, key) in [("primary", "primary"), ("enabled", "enabled")] {
        class = flag(class, name, key);
    }
    for (name, key) in [
        ("scale", "output_scale"),
        ("supersample", "supersample"),
        ("x", "x"),
        ("y", "y"),
        ("width", "width"),
        ("height", "height"),
    ] {
        class = number(class, name, key);
    }
    let _ = class
        .get("effective_scale", |state, _| {
            Ok(Val::Double(
                state.read("output_scale").number() * state.read("supersample").number(),
            ))
        })
        .get("virtual_size", |state, cx| {
            let (width, height) = mode_size(&state.read("mode").string());
            let factor = state.read("supersample").number();
            size(cx, (width * factor).round(), (height * factor).round())
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
        .get("gpu_memory", |state, cx| {
            let field = |name: &'static str, key: &str| (name, Val::Double(state.read(key).number()));
            record(
                cx,
                "gpu_memory",
                vec![
                    field("target", "gpu_memory_target_bytes"),
                    field("scratch", "gpu_memory_scratch_bytes"),
                    field("staging", "gpu_memory_staging_bytes"),
                    field("views", "gpu_memory_views_bytes"),
                    field("capture", "gpu_memory_capture_bytes"),
                    field("total", "gpu_memory_bytes"),
                    field("clients", "gpu_memory_clients_bytes"),
                    field("budget", "gpu_memory_budget_bytes"),
                    field("refused", "gpu_memory_refused_total"),
                    field("evicted", "gpu_memory_evicted_total"),
                ],
            )
        })
        .get("mirror_of", |state, cx| {
            let source = state.read("mirror_of").string();
            if source.is_empty() {
                return Ok(Val::None);
            }
            let outputs = with_world(cx, |world| world.outputs.values().copied().collect::<Vec<_>>())?;
            for output in outputs {
                let connector = cx
                    .with_state::<OutputNode, _>(output, |other| other.read("connector").string())
                    .unwrap_or_default();
                if connector == source {
                    return handout(cx, output);
                }
            }
            Ok(Val::None)
        })
        .get("properties", |state, cx| {
            properties::bindings_of(cx, cx.me(), &state.bindings)
        })
        .method("apply", |state, cx, args| {
            let target = format!("Output#{}.apply", state.node);
            let (text, notice) = request(cx, &args[0], &target)?;
            send(cx, state, text, notice, &target)
        })
        .method("mirror", |state, cx, args| {
            let target = format!("Output#{}.mirror", state.node);
            let connector = match args.first() {
                None | Some(Val::None) => String::new(),
                Some(source) => {
                    let source = source.object()?;
                    cx.with_state::<OutputNode, _>(source, |other| other.read("connector").string())
                        .map_err(|_| Fail::type_error(format!("{target}: the source is not another output")))?
                }
            };
            send(cx, state, format!("mirror_of={connector}"), None, &target)
        })
        .method("add_listener", |state, cx, args| {
            listener::add(cx, cx.me(), &mut state.listeners, &args[0])?;
            Ok(Val::None)
        })
        .method("remove_listener", |state, cx, args| {
            listener::remove(cx, &mut state.listeners, &args[0])?;
            Ok(Val::None)
        });
}
