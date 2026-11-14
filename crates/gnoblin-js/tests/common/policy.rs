//! A world the window-policy tests share (`reposition.rs`, `window_policy.rs`): one screen of 1000 by 800 (two with
//! `second`), a seat, a lock, and windows of 300 by 200 at (10 n, 20 n), on workspaces `1` to `3`. Cases name the
//! Lua test they port.

use super::*;

pub const IMPORTS: &str = "import * as lib from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = lib;\nObject.assign(globalThis, exported);\n(globalThis as any).g = globalThis;\n";

pub const SEAT: u64 = 21;
pub const LOCK: u64 = 23;

/// A layout that gives every window the whole area it is offered, and marks it tiled.
pub const FILL: &str = "class Fill extends Layout {\n\
    \x20   compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { ...c.area } })); }\n\
}\n";

/// A shell over the screens, a seat, a lock and `n` windows (ids 3 on). `layout` is the TypeScript expression of every
/// workspace's layout; `config` is more keys of the `defineConfig` object.
pub fn world(n: u64, second: bool, layout: &str, config: &str) -> Rig {
    let init = format!(
        "{FILL}defineConfig({{ workspaces: [{{ id: \"1\", layout: {layout} }}, {{ id: \"2\", layout: {layout} }}, {{ id: \"3\", layout: {layout} }}], decorations: false, {config} }});\n"
    );
    let mut rig = rig_setup(IMPORTS, &init, move |comp| {
        comp.add_screen(20, "output-0", (0.0, 0.0, 1000.0, 800.0), true);
        if second {
            comp.add_screen(22, "output-1", (1000.0, 0.0, 600.0, 400.0), false);
        }
        comp.add_node(
            SEAT,
            ROOT,
            "seat",
            vec![
                ("name", text("seat0")),
                ("pointer_x", MirrorValue::Double(0.0)),
                ("pointer_y", MirrorValue::Double(0.0)),
            ],
        );
        comp.add_node(LOCK, ROOT, "lock", Vec::new());
        for i in 1..=n {
            comp.add_node(
                2 + i,
                ROOT,
                "window",
                vec![
                    ("title", text(&format!("win{i}"))),
                    ("app_id", text("foot")),
                    ("x", MirrorValue::Double(10.0 * i as f64)),
                    ("y", MirrorValue::Double(20.0 * i as f64)),
                    ("width", MirrorValue::Double(300.0)),
                    ("height", MirrorValue::Double(200.0)),
                ],
            );
        }
    });
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?} {:?}", rig.load_diagnostics);
    rig
}

/// Runs a statement in the script. `Rig::eval` wants a value back; a statement has none.
pub fn exec(rig: &mut Rig, source: &str) {
    rig.shell.eval(source).expect("the script runs");
}

/// Runs the frames and fails on any diagnostic.
pub fn settle(rig: &mut Rig) {
    let diagnostics = rig.settle();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// The seat's focus: a node, or 0 for none.
pub fn focus(rig: &Rig) -> u64 {
    match rig.comp.value(SEAT, "focus") {
        Some(MirrorValue::Node(id)) => id,
        _ => 0,
    }
}

pub fn give_focus(rig: &mut Rig, node: u64) {
    rig.comp.external_set(SEAT, "focus", MirrorValue::Node(node));
    settle(rig);
}

/// `x,y,width,height` of window `node` as the compositor holds it.
pub fn rect(rig: &Rig, node: u64) -> String {
    rig.comp.rect(node)
}

/// A script's string, parsed as a number.
pub fn number(rig: &mut Rig, expression: &str) -> f64 {
    rig.eval(&format!("String({expression})")).parse().expect("a number")
}

/// The workspace id of window `node`, by the script.
pub fn workspace_of(rig: &mut Rig, node: u64) -> String {
    rig.eval(&format!("gnoblin.clients.get('{node}')?.workspace?.id ?? 'none'"))
}

/// Which window holds the keyboard, by the script: the node's id, or `none`.
pub fn focused_title(rig: &mut Rig) -> String {
    let focus = focus(rig);
    rig.eval(&format!("gnoblin.clients.get('{focus}')?.title ?? 'none'"))
}

/// A dialog of window `parent`, mapped as the compositor does: created with its `parent_window` and size.
pub fn create_dialog(rig: &mut Rig, id: u64, parent: u64) {
    let props = [
        ("title", text("dialog")),
        ("app_id", text("dialog")),
        ("width", MirrorValue::Double(100.0)),
        ("height", MirrorValue::Double(60.0)),
        ("parent_window", MirrorValue::Node(parent)),
    ];
    rig.comp.add_node(id, ROOT, "window", props.to_vec());
    let mut changes = vec![
        MirrorChange {
            class: "window".to_owned(),
            ..change(MirrorChangeKind::Created, id)
        },
        MirrorChange {
            parent: ROOT,
            index: 99,
            ..change(MirrorChangeKind::Parented, id)
        },
    ];
    changes.extend(props.into_iter().map(|(key, value)| set_change(id, key, value)));
    rig.comp.publish(changes);
    settle(rig);
}

/// The compositor closes window `node`: the seat's focus drops, and the node goes.
pub fn close_window(rig: &mut Rig, node: u64) {
    if focus(rig) == node {
        rig.comp.external_set(SEAT, "focus", MirrorValue::Node(0));
    }
    rig.comp.destroy(node);
    settle(rig);
}

/// A second shell on the compositor of `rig`, as a reload starts one, with `config` (the keys of a `defineConfig` object)
/// over the same three workspaces. The caller `reconcile`s it against the first one's snapshot.
pub fn next_generation(rig: &Rig, config: &str) -> Rig {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = std::fs::canonicalize(dir.path()).expect("the temp dir exists");
    let init = format!(
        "{IMPORTS}defineConfig({{ workspaces: [{{ id: \"1\" }}, {{ id: \"2\" }}, {{ id: \"3\" }}], decorations: false, {config} }});\n"
    );
    std::fs::write(root.join("init.ts"), init).expect("init.ts is written");
    let mut shell = gnoblin_js::JsShell::new(Box::new(rig.comp.clone()), gnoblin_js::JsShellConfig::new(&root))
        .expect("the shell boots");
    let load_diagnostics = shell
        .load_entry(std::path::Path::new("init.ts"))
        .expect("init.ts loads");
    Rig {
        shell,
        comp: rig.comp.clone(),
        load_diagnostics,
        _dir: dir,
    }
}
