//! Shorthands for building a configuration tree in a test.
//!
//! A document is written the way the Lua host produces it: nested tables, lists
//! and scalars. The helpers exist so a test reads as the config file a user would
//! write, and so a change to [`gnoblin_config::Value`] is a change in one place.

#![allow(dead_code)]

use gnoblin_config::Value;

/// A table.
pub fn table<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::table(entries)
}

/// A list.
pub fn list(items: Vec<Value>) -> Value {
    Value::List(items)
}

/// A string.
pub fn text(value: &str) -> Value {
    Value::String(value.to_owned())
}

/// A boolean.
pub fn flag(value: bool) -> Value {
    Value::Bool(value)
}

/// An integer.
pub fn int(value: i64) -> Value {
    Value::Integer(value)
}

/// A number with a fractional part.
pub fn number(value: f64) -> Value {
    Value::Float(value)
}

/// A document with one valid setting in each of several sections.
pub fn kitchen_sink() -> Value {
    table([
        ("auth", table([("polkit-agent", flag(true))])),
        (
            "compositor",
            table([
                ("blur", text("on")),
                ("debug", flag(false)),
                ("locate-pointer", flag(true)),
            ]),
        ),
        ("cursor", table([("size", int(24)), ("theme", text("default"))])),
        (
            "frame-renderers",
            table([("cairo", list(vec![text("gnoblin-frame-cairo")]))]),
        ),
        (
            "input",
            table([
                ("mouse", table([("speed", number(0.5)), ("left-handed", flag(true))])),
                (
                    "touchpad",
                    table([("tap-to-click", flag(true)), ("left-handed", text("right"))]),
                ),
            ]),
        ),
        (
            "input-sources",
            table([
                (
                    "sources",
                    list(vec![table([("type", text("xkb")), ("id", text("gb"))])]),
                ),
                ("per-window", flag(true)),
            ]),
        ),
        ("layer-shell", table([("preserve-active-window", flag(false))])),
        (
            "location",
            table([("enabled", flag(true)), ("max-accuracy", text("city"))]),
        ),
        ("monitors", table([("privacy-screen", text("inherit"))])),
        ("prompts", table([("enabled", flag(true))])),
        ("protocols", table([("wlr-screencopy", flag(false))])),
        (
            "touchpad-gestures",
            list(vec![table([
                ("name", text("next-workspace")),
                ("gesture", text("swipe")),
                ("fingers", int(3)),
                (
                    "path",
                    list(vec![
                        table([("x", number(0.0)), ("y", number(0.0))]),
                        table([("x", number(1.0)), ("y", number(0.0))]),
                    ]),
                ),
                ("action", text("workspace.next")),
            ])]),
        ),
        (
            "window-management",
            table([("focus-mode", text("hover")), ("edge-tiling", flag(true))]),
        ),
        (
            "workspaces",
            list(vec![
                table([("id", text("main")), ("name", text("Main"))]),
                table([("id", text("chat")), ("name", text("Chat"))]),
            ]),
        ),
        (
            "xwayland",
            table([("allow-grabs", flag(false)), ("scaling-factor", int(0))]),
        ),
    ])
}
