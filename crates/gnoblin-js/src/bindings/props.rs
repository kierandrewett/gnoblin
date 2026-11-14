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
    /// Three numbers: a window's `rotation` and `scale`.
    Doubles,
    Text,
    Node,
    /// A list of effect objects: a node's `effects`.
    Effects,
    /// A list of texts: a node's `skipped`.
    Texts,
}

/// One property of a class.
#[derive(Clone, Copy, Debug)]
pub(crate) struct JsPropDef {
    pub name: &'static str,
    pub kind: JsPropKind,
    /// Whether a script may write it. The compositor owns a client's title, focus and fullscreen state.
    pub writable: bool,
    /// Whether the tree accepts a write of it that carries `seconds` (`animated` in the schema).
    pub animated: bool,
}

const fn def(name: &'static str, kind: JsPropKind, writable: bool) -> JsPropDef {
    JsPropDef {
        name,
        kind,
        writable,
        animated: false,
    }
}

const fn animated(name: &'static str, kind: JsPropKind) -> JsPropDef {
    JsPropDef {
        name,
        kind,
        writable: true,
        animated: true,
    }
}

const WINDOW: &[JsPropDef] = &[
    def("title", JsPropKind::Text, false),
    def("app_id", JsPropKind::Text, false),
    def("focused", JsPropKind::Flag, false),
    def("fullscreen", JsPropKind::Flag, true),
    def("maximised", JsPropKind::Flag, true),
    def("minimised", JsPropKind::Flag, true),
    animated("x", JsPropKind::Double),
    animated("y", JsPropKind::Double),
    animated("width", JsPropKind::Double),
    animated("height", JsPropKind::Double),
    def("workspace", JsPropKind::Node, false),
    // Written by the shell: the window sits in a tiled layout, so its client drops its border and rounding.
    def("tiled", JsPropKind::Flag, true),
    // A hidden node is neither drawn nor hit.
    def("visible", JsPropKind::Flag, true),
    animated("opacity", JsPropKind::Double),
    // The placement in depth: `z` and the rotation and scale about each axis. A layout that wants a 3D look
    // writes these; the others leave them at their defaults.
    animated("z", JsPropKind::Double),
    animated("rotation", JsPropKind::Doubles),
    animated("scale", JsPropKind::Doubles),
    // Whether the window is kept out of screen captures.
    def("secure", JsPropKind::Flag, true),
    // Whether the client has left a liveness ping unanswered too long. The compositor writes it.
    def("unresponsive", JsPropKind::Flag, false),
    // The node's effects in paint order (blur and shadow), and the ones the compositor cannot draw. The style
    // cascade writes the first; the second is read-only.
    def("effects", JsPropKind::Effects, true),
    def("skipped", JsPropKind::Texts, false),
];

/// An output. Its size is the work area the layout driver tiles in, `primary` picks the screen the primary display
/// shows, and its `name` is what a workspace set's entry calls it.
const SCREEN: &[JsPropDef] = &[
    def("name", JsPropKind::Text, false),
    def("width", JsPropKind::Double, false),
    def("height", JsPropKind::Double, false),
    def("primary", JsPropKind::Flag, false),
];

/// The methods a script may call on a window. `kill` is the not-responding alert's Force quit: the compositor
/// refuses its own process, init and a process that did not open the window.
const WINDOW_METHODS: &[&str] = &["focus", "close", "kill"];

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
pub(crate) fn default_value(def: &JsPropDef) -> MirrorValue {
    match (def.name, def.kind) {
        ("visible", _) => MirrorValue::Bool(true),
        ("opacity", _) => MirrorValue::Double(1.0),
        ("scale", _) => MirrorValue::List(vec![1.0, 1.0, 1.0]),
        (_, JsPropKind::Flag) => MirrorValue::Bool(false),
        (_, JsPropKind::Double) => MirrorValue::Double(0.0),
        (_, JsPropKind::Doubles) => MirrorValue::List(vec![0.0, 0.0, 0.0]),
        (_, JsPropKind::Text) => MirrorValue::Text(String::new()),
        (_, JsPropKind::Node) => MirrorValue::Node(0),
        (_, JsPropKind::Effects) => MirrorValue::Effects(Vec::new()),
        (_, JsPropKind::Texts) => MirrorValue::Texts(Vec::new()),
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
    fn the_geometry_the_state_flags_and_the_depth_placement_of_a_window_are_writable() {
        let writable: Vec<&str> = defs_for("window")
            .into_iter()
            .flatten()
            .filter(|def| def.writable)
            .map(|def| def.name)
            .collect();
        assert_eq!(
            writable,
            [
                "fullscreen",
                "maximised",
                "minimised",
                "x",
                "y",
                "width",
                "height",
                "tiled",
                "visible",
                "opacity",
                "z",
                "rotation",
                "scale",
                "secure",
                "effects"
            ]
        );
        for def in defs_for("window").into_iter().flatten().filter(|def| def.writable) {
            let want = match def.name {
                "fullscreen" | "maximised" | "minimised" | "tiled" | "visible" | "secure" => JsPropKind::Flag,
                "effects" => JsPropKind::Effects,
                "rotation" | "scale" => JsPropKind::Doubles,
                _ => JsPropKind::Double,
            };
            assert_eq!(def.kind, want, "{}", def.name);
        }
    }

    #[test]
    fn only_the_properties_the_schema_animates_take_a_curve() {
        let animated: Vec<&str> = defs_for("window")
            .into_iter()
            .flatten()
            .filter(|def| def.animated)
            .map(|def| def.name)
            .collect();
        assert_eq!(
            animated,
            ["x", "y", "width", "height", "opacity", "z", "rotation", "scale"]
        );
    }

    #[test]
    fn a_script_may_close_focus_and_kill_a_window_and_nothing_else() {
        assert!(may_call("window", "close"));
        assert!(may_call("Window", "focus"));
        assert!(may_call("window", "kill"));
        assert!(!may_call("window", "destroy"));
        assert!(!may_call("screen", "close"));
    }
}
