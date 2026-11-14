//! The tree properties the `gnoblin` module exposes, with their kinds and defaults.
//!
//! The mirror keeps only the properties that differ from their schema default, so a read of an absent
//! property has to supply the default itself. This table is the one place that knows which properties
//! of a class a script may see, what kind each is, and whether a script may write it.

use gnoblin_tree_client::tree_mirror::{MirrorValue, class_name};

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
    /// For a list of texts: the only names an item may be. Empty means any text. The schema says a name outside the set
    /// "claims nothing", so a script that writes one has made a mistake the shell tells it about.
    pub choices: &'static [&'static str],
}

/// The inputs of a controller a shell binding may claim: the schema's five, lowercase.
pub(crate) const CLAIMABLE_INPUTS: &[&str] = &["trigger", "grip", "thumbstick", "primary", "secondary"];

const fn def(name: &'static str, kind: JsPropKind, writable: bool) -> JsPropDef {
    JsPropDef {
        name,
        kind,
        writable,
        animated: false,
        choices: &[],
    }
}

const fn animated(name: &'static str, kind: JsPropKind) -> JsPropDef {
    JsPropDef {
        name,
        kind,
        writable: true,
        animated: true,
        choices: &[],
    }
}

/// A writable list of texts whose items come from `choices`.
const fn texts_of(name: &'static str, choices: &'static [&'static str]) -> JsPropDef {
    JsPropDef {
        name,
        kind: JsPropKind::Texts,
        writable: true,
        animated: false,
        choices,
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
    // The window this one is a dialog of, and whether a launcher's activation token opened it. The compositor writes
    // both from the client's protocol requests.
    def("parent_window", JsPropKind::Node, false),
    def("from_launcher", JsPropKind::Flag, false),
    // Who draws the window's frame (`client`, `server` or `none`) and what its client asked of `xdg-decoration`
    // (`client`, `server` or `unset`). The compositor writes both.
    def("decoration", JsPropKind::Text, false),
    def("decoration_wanted", JsPropKind::Text, false),
    // Written by the shell: the window sits in a tiled layout, so its client drops its border and rounding.
    def("tiled", JsPropKind::Flag, true),
    // A hidden node is neither drawn nor hit.
    def("visible", JsPropKind::Flag, true),
    animated("opacity", JsPropKind::Double),
    // The radius of the rounded corners the window's frame (content and titlebar together) is clipped to.
    animated("corner_radius", JsPropKind::Double),
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

/// An output. Its rectangle is the work area the layout driver tiles in, `primary` picks the screen the primary display
/// shows, and `name` is what a workspace set's entry calls it. The compositor owns every property.
const SCREEN: &[JsPropDef] = &[
    def("name", JsPropKind::Text, false),
    def("connector", JsPropKind::Text, false),
    def("identity", JsPropKind::Text, false),
    def("mirror_of", JsPropKind::Text, false),
    def("make", JsPropKind::Text, false),
    def("model", JsPropKind::Text, false),
    def("x", JsPropKind::Double, false),
    def("y", JsPropKind::Double, false),
    def("width", JsPropKind::Double, false),
    def("height", JsPropKind::Double, false),
    def("output_scale", JsPropKind::Double, false),
    def("transform", JsPropKind::Text, false),
    def("mode", JsPropKind::Text, false),
    def("primary", JsPropKind::Flag, false),
    def("enabled", JsPropKind::Flag, false),
];

/// A plain node: what the XR room is. A script sets its name, placement and visibility, and nothing else.
const NODE: &[JsPropDef] = &[
    def("name", JsPropKind::Text, true),
    animated("x", JsPropKind::Double),
    animated("y", JsPropKind::Double),
    animated("z", JsPropKind::Double),
    animated("rotation", JsPropKind::Doubles),
    animated("scale", JsPropKind::Doubles),
    animated("width", JsPropKind::Double),
    animated("height", JsPropKind::Double),
    animated("opacity", JsPropKind::Double),
    def("visible", JsPropKind::Flag, true),
    def("input", JsPropKind::Flag, true),
    // What a hit does when `input` is set: `forward` or `capture`. `input` unset is the third state, `none`.
    def("input_mode", JsPropKind::Text, true),
    def("secure", JsPropKind::Flag, true),
];

/// A `view`: it draws another node (its `source`) through an optional `camera`. The XR program reads the Views under
/// the node named `room`. `fps` is not here: nothing in the shell needs it, and it is a `uint`, which no kind holds.
const VIEW: &[JsPropDef] = &[
    def("name", JsPropKind::Text, true),
    animated("x", JsPropKind::Double),
    animated("y", JsPropKind::Double),
    animated("z", JsPropKind::Double),
    animated("rotation", JsPropKind::Doubles),
    animated("scale", JsPropKind::Doubles),
    animated("width", JsPropKind::Double),
    animated("height", JsPropKind::Double),
    animated("opacity", JsPropKind::Double),
    def("visible", JsPropKind::Flag, true),
    def("input", JsPropKind::Flag, true),
    // What a hit does when `input` is set: `forward` or `capture`. `input` unset is the third state, `none`.
    def("input_mode", JsPropKind::Text, true),
    def("secure", JsPropKind::Flag, true),
    def("source", JsPropKind::Node, true),
    def("camera", JsPropKind::Node, true),
];

/// A `camera`. `layer_budget` is the XR program's to write: the shell reads it.
const CAMERA: &[JsPropDef] = &[
    def("name", JsPropKind::Text, true),
    animated("x", JsPropKind::Double),
    animated("y", JsPropKind::Double),
    animated("z", JsPropKind::Double),
    animated("rotation", JsPropKind::Doubles),
    animated("scale", JsPropKind::Doubles),
    animated("width", JsPropKind::Double),
    animated("height", JsPropKind::Double),
    animated("opacity", JsPropKind::Double),
    def("visible", JsPropKind::Flag, true),
    def("input", JsPropKind::Flag, true),
    // What a hit does when `input` is set: `forward` or `capture`. `input` unset is the third state, `none`.
    def("input_mode", JsPropKind::Text, true),
    def("secure", JsPropKind::Flag, true),
    def("projection", JsPropKind::Text, true),
    def("metres", JsPropKind::Double, true),
    def("near", JsPropKind::Double, true),
    def("far", JsPropKind::Double, true),
    animated("fov", JsPropKind::Double),
    def("ipd", JsPropKind::Double, true),
    def("fov_left", JsPropKind::Double, true),
    def("fov_right", JsPropKind::Double, true),
    def("fov_up", JsPropKind::Double, true),
    def("fov_down", JsPropKind::Double, true),
    def("layer_budget", JsPropKind::Double, false),
];

/// A `controller`: one tracked hand of the XR program (`design/js-xr.md`, "New native work"). The program writes
/// every property but `claimed`, so a script reads them all and writes only the claims of its own bindings.
const CONTROLLER: &[JsPropDef] = &[
    def("name", JsPropKind::Text, false),
    def("hand", JsPropKind::Text, false),
    def("tracked", JsPropKind::Flag, false),
    def("x", JsPropKind::Double, false),
    def("y", JsPropKind::Double, false),
    def("z", JsPropKind::Double, false),
    def("rotation", JsPropKind::Doubles, false),
    def("trigger", JsPropKind::Double, false),
    def("grip", JsPropKind::Double, false),
    def("thumbstick_x", JsPropKind::Double, false),
    def("thumbstick_y", JsPropKind::Double, false),
    def("thumbstick_click", JsPropKind::Flag, false),
    def("primary", JsPropKind::Flag, false),
    def("secondary", JsPropKind::Flag, false),
    texts_of("claimed", CLAIMABLE_INPUTS),
];

/// A `tween`: the play state of the tracks a transition's timed writes started (`tween.toml`). The shell makes one for
/// each transition with tracks, writes `progress` to scrub them, `paused` to hold them and `reverse` to run them back.
const TWEEN: &[JsPropDef] = &[
    def("progress", JsPropKind::Double, true),
    def("paused", JsPropKind::Flag, true),
    def("reverse", JsPropKind::Flag, true),
];

/// The methods a script may call on a window. `kill` is the not-responding alert's Force quit: the compositor
/// refuses its own process, init and a process that did not open the window.
const WINDOW_METHODS: &[&str] = &["focus", "close", "kill"];

/// The properties a script sees on `class`, which the mirror names by its wire name (`window`) or the
/// shell's (`Window`).
pub(crate) fn defs_for(class: &str) -> Option<&'static [JsPropDef]> {
    match class_name(class).as_str() {
        "Window" => Some(WINDOW),
        "Screen" => Some(SCREEN),
        "Node" => Some(NODE),
        "View" => Some(VIEW),
        "Camera" => Some(CAMERA),
        "Controller" => Some(CONTROLLER),
        "Tween" => Some(TWEEN),
        _ => None,
    }
}

/// Whether a script may call `method` on `class`.
pub(crate) fn may_call(class: &str, method: &str) -> bool {
    class_name(class) == "Window" && WINDOW_METHODS.contains(&method)
}

/// The value of a property no write has changed from its schema default.
pub(crate) fn default_value(def: &JsPropDef) -> MirrorValue {
    match (def.name, def.kind) {
        ("visible", _) | ("enabled", _) => MirrorValue::Bool(true),
        ("output_scale", _) => MirrorValue::Double(1.0),
        ("transform", _) => MirrorValue::Text("normal".to_owned()),
        ("decoration", _) => MirrorValue::Text("client".to_owned()),
        ("decoration_wanted", _) => MirrorValue::Text("unset".to_owned()),
        ("opacity", _) => MirrorValue::Double(1.0),
        ("input", _) => MirrorValue::Bool(true),
        ("input_mode", _) => MirrorValue::Text("forward".to_owned()),
        ("metres", _) => MirrorValue::Double(0.001),
        ("near", _) => MirrorValue::Double(0.1),
        ("far", _) => MirrorValue::Double(1000.0),
        ("fov", _) => MirrorValue::Double(60.0),
        ("ipd", _) => MirrorValue::Double(0.063),
        ("projection", _) => MirrorValue::Text("orthographic".to_owned()),
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
    use gnoblin_tree_client::tree_mirror::MirrorValue;

    use super::{JsPropKind, default_value, defs_for, may_call};

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
                "corner_radius",
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
            [
                "x",
                "y",
                "width",
                "height",
                "opacity",
                "corner_radius",
                "z",
                "rotation",
                "scale"
            ]
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

    #[test]
    fn a_script_may_make_and_place_a_room_a_camera_and_a_view_and_may_only_read_a_controller_but_claim_its_inputs() {
        for class in ["node", "Node", "view", "camera"] {
            assert!(defs_for(class).is_some(), "{class}");
        }
        let names =
            |class: &str| -> Vec<&'static str> { defs_for(class).into_iter().flatten().map(|def| def.name).collect() };
        assert!(names("view").contains(&"source") && names("view").contains(&"camera"));
        assert!(!names("node").contains(&"source"), "a plain node shows nothing");
        let writable = |class: &str, key: &str| {
            defs_for(class)
                .into_iter()
                .flatten()
                .find(|def| def.name == key)
                .map(|def| def.writable)
        };
        assert_eq!(writable("camera", "projection"), Some(true));
        assert_eq!(
            writable("camera", "layer_budget"),
            Some(false),
            "the XR program writes the budget"
        );
        assert_eq!(writable("view", "source"), Some(true));
        let controller_writes: Vec<&str> = defs_for("controller")
            .into_iter()
            .flatten()
            .filter(|def| def.writable)
            .map(|def| def.name)
            .collect();
        assert_eq!(
            controller_writes,
            ["claimed"],
            "the program writes every property of a controller but the claims"
        );
        // A property at its schema default is absent from the mirror, so the table supplies it.
        let metres = defs_for("camera")
            .into_iter()
            .flatten()
            .find(|def| def.name == "metres")
            .expect("metres");
        assert_eq!(default_value(metres), MirrorValue::Double(0.001));
        let input = defs_for("view")
            .into_iter()
            .flatten()
            .find(|def| def.name == "input")
            .expect("input");
        assert_eq!(default_value(input), MirrorValue::Bool(true));
        let mode = defs_for("view")
            .into_iter()
            .flatten()
            .find(|def| def.name == "input_mode")
            .expect("input_mode");
        assert_eq!(default_value(mode), MirrorValue::Text("forward".to_owned()));
    }

    /// The class the XR program publishes is the schema's, not a shape this table made up: every property the schema's
    /// `controller.toml` declares is in the table with the kind of its type, and a script may write exactly the ones
    /// the schema does not mark read-only (`claimed`). The table also carries the node's own pose and name, which a
    /// script reads.
    #[test]
    fn the_controller_table_is_the_schemas_controller() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../gnoblin-schema/schema/tree/controller.toml"
        );
        let text = std::fs::read_to_string(path).expect("the schema file is in the repository");
        let document: toml::Table = text.parse().expect("the schema file is TOML");
        let object = document["object"]
            .as_array()
            .and_then(|objects| objects.first())
            .expect("an object");
        assert_eq!(object["name"].as_str(), Some("controller"));
        assert_eq!(object["base"].as_str(), Some("node"));
        let properties = object["property"].as_array().expect("properties");
        let table = defs_for("controller").expect("the controller table");
        for property in properties {
            let name = property["name"].as_str().expect("a name");
            let ty = property["type"].as_str().expect("a type");
            let readonly = property.get("readonly").and_then(toml::Value::as_bool).unwrap_or(false);
            let def = table
                .iter()
                .find(|def| def.name == name)
                .unwrap_or_else(|| panic!("the controller table has no `{name}`"));
            let kind = match ty {
                "string" => JsPropKind::Text,
                "bool" => JsPropKind::Flag,
                "double" => JsPropKind::Double,
                "list<string>" => JsPropKind::Texts,
                other => panic!("`{name}` has a type the table does not know: {other}"),
            };
            assert_eq!(def.kind, kind, "the kind of `{name}`");
            assert_eq!(
                def.writable, !readonly,
                "a script writes `{name}` exactly when the schema does"
            );
        }
        for inherited in ["name", "x", "y", "z", "rotation"] {
            assert!(
                table.iter().any(|def| def.name == inherited && !def.writable),
                "`{inherited}` of the node is read by scripts"
            );
        }
    }
}
