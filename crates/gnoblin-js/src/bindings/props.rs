//! The tree properties the `gnoblin` module exposes, with their kinds and defaults.
//!
//! The mirror keeps only the properties that differ from their schema default, so a read of an absent
//! property has to supply the default itself. This table is the one place that knows which properties
//! of a class a script may see, what kind each is, and whether a script may write it.

use gnoblin_tree_client::tree_mirror::{MirrorValue, lua_class};

/// The kind of value a property holds, and so what a script's value must convert to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsPropKind {
    Flag,
    Double,
    Text,
    Node,
}

/// One property of a class.
#[derive(Clone, Copy, Debug)]
pub(crate) struct JsPropDef {
    pub name: &'static str,
    pub kind: JsPropKind,
    /// Whether a script may write it. The compositor owns a client's title, focus and fullscreen state.
    pub writable: bool,
}

const fn def(name: &'static str, kind: JsPropKind, writable: bool) -> JsPropDef {
    JsPropDef { name, kind, writable }
}

const WINDOW: &[JsPropDef] = &[
    def("title", JsPropKind::Text, false),
    def("app_id", JsPropKind::Text, false),
    def("focused", JsPropKind::Flag, false),
    def("fullscreen", JsPropKind::Flag, false),
    def("maximised", JsPropKind::Flag, true),
    def("minimised", JsPropKind::Flag, true),
    def("x", JsPropKind::Double, true),
    def("y", JsPropKind::Double, true),
    def("width", JsPropKind::Double, true),
    def("height", JsPropKind::Double, true),
    def("workspace", JsPropKind::Node, false),
];

/// An output. Its size is the work area the layout driver tiles in, and `primary` picks the screen the
/// primary display shows.
const SCREEN: &[JsPropDef] = &[
    def("width", JsPropKind::Double, false),
    def("height", JsPropKind::Double, false),
    def("primary", JsPropKind::Flag, false),
];

/// The methods a script may call on a window.
const WINDOW_METHODS: &[&str] = &["focus", "close"];

/// The properties a script sees on `class`, which the mirror names by its wire name (`window`) or the
/// shell's (`Window`).
pub(crate) fn defs_for(class: &str) -> Option<&'static [JsPropDef]> {
    match lua_class(class).as_str() {
        "Window" => Some(WINDOW),
        "Screen" => Some(SCREEN),
        _ => None,
    }
}

/// Whether a script may call `method` on `class`.
pub(crate) fn may_call(class: &str, method: &str) -> bool {
    lua_class(class) == "Window" && WINDOW_METHODS.contains(&method)
}

/// The value of a property no write has changed from its schema default.
pub(crate) fn default_value(kind: JsPropKind) -> MirrorValue {
    match kind {
        JsPropKind::Flag => MirrorValue::Bool(false),
        JsPropKind::Double => MirrorValue::Double(0.0),
        JsPropKind::Text => MirrorValue::Text(String::new()),
        JsPropKind::Node => MirrorValue::Node(0),
    }
}

#[cfg(test)]
mod tests {
    use super::{JsPropKind, defs_for, may_call};

    #[test]
    fn a_class_is_found_by_either_spelling() {
        assert!(defs_for("window").is_some());
        assert!(defs_for("Window").is_some());
        assert!(defs_for("screen").is_some());
        assert!(defs_for("Screen").is_some());
        assert!(
            defs_for("workspace").is_none(),
            "workspaces are the script's, not the tree's"
        );
        assert!(defs_for("seat").is_none());
    }

    #[test]
    fn only_the_geometry_and_the_maximised_and_minimised_state_of_a_window_are_writable() {
        let writable: Vec<&str> = defs_for("window")
            .into_iter()
            .flatten()
            .filter(|def| def.writable)
            .map(|def| def.name)
            .collect();
        assert_eq!(writable, ["maximised", "minimised", "x", "y", "width", "height"]);
        assert!(
            defs_for("window")
                .into_iter()
                .flatten()
                .filter(|def| def.writable)
                .all(|def| match def.name {
                    "maximised" | "minimised" => def.kind == JsPropKind::Flag,
                    _ => def.kind == JsPropKind::Double,
                })
        );
    }

    #[test]
    fn a_script_may_close_and_focus_a_window_and_nothing_else() {
        assert!(may_call("window", "close"));
        assert!(may_call("Window", "focus"));
        assert!(!may_call("window", "kill"));
        assert!(!may_call("screen", "close"));
    }
}
