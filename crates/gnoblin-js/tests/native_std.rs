//! The harness of `common::native`: a shell over the real core and the real std library, in the test's own process, over the fake
//! compositor.
//!
//! What it proves is the shape every std-touching scenario of this crate then runs in: the engine's mirror is a view of the core's tree
//! (one holder of the compositor's link), a window the compositor makes is a window of `gnoblin.clients` and of the library's
//! collection, the library's layout pass writes the placements and the engine reads them back, and what the engine writes and what the
//! library writes reach the compositor as the one connection's transactions, none of them refused.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::{FakeCompositor, ROOT, native, rig_native, text};

/// A 1000 by 800 screen, a seat and the window 40.
fn world(comp: &FakeCompositor) {
    comp.add_node(
        2,
        ROOT,
        "screen",
        vec![
            ("width", MirrorValue::Double(1000.0)),
            ("height", MirrorValue::Double(800.0)),
            ("primary", MirrorValue::Bool(true)),
        ],
    );
    comp.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
    comp.add_window(40, "Terminal", "foot");
}

const IMPORTS: &str = r#"
import { gnoblin } from "gnoblin-core";
import { EventKind, MasterStackLayout, clients, createWorkspace, insetArea, tile } from "gnoblin-std/native";
import type { Layout, LayoutContext } from "gnoblin-std/native";
(globalThis as any).clients = clients;
"#;

/// Equal-width columns across the workspace, the newest on the right: a layout written in the shell.
const COLUMNS: &str = r#"
class Columns implements Layout {
    compute(cx: LayoutContext) {
        const count = cx.children.length;
        if (count === 0) {
            return [];
        }
        const area = insetArea(cx);
        const width = Math.max((area.width - cx.gapInner * (count - 1)) / count, 0);
        return cx.children.map((client, index) =>
            tile(client, area.x + index * (width + cx.gapInner), area.y, width, area.height),
        );
    }
}
const main = createWorkspace("main", { name: "Main", layout: new Columns(), gaps: [10, 20] });
(globalThis as any).opened = [] as number[];
clients().listen(EventKind.ADDED, 0, (event) => {
    (globalThis as any).opened.push(event.member);
});
"#;

#[test]
fn a_shell_writes_a_layout_of_its_own_and_the_library_arranges_the_windows_of_the_compositor() {
    let mut rig = rig_native(IMPORTS, COLUMNS, world);
    rig.settle();
    assert_eq!(rig.comp.rect(40), "20,20,960,760");
    rig.comp.create_window(41, "Files", "nautilus");
    rig.settle();
    assert_eq!(rig.comp.rect(40), "20,20,475,760");
    assert_eq!(rig.comp.rect(41), "505,20,475,760");
    rig.comp.create_window(42, "Editor", "code");
    rig.settle();
    assert_eq!(rig.comp.rect(40), "20,20,313,760");
    assert_eq!(rig.comp.rect(41), "343,20,313,760");
    assert_eq!(rig.comp.rect(42), "666,20,314,760");
    // The shell's own listener heard the windows that opened after it listened (the library's collection, through the shell).
    assert_eq!(rig.eval("globalThis.opened.length"), "2");
    // One connection: nothing the engine or the library wrote was refused.
    assert_eq!(rig.comp.refused(), 0);
}

#[test]
fn the_engine_and_the_library_see_one_tree() {
    let mut rig = rig_native(IMPORTS, COLUMNS, world);
    rig.settle();
    rig.comp.create_window(41, "Files", "nautilus");
    rig.settle();
    // The engine's clients are the library's clients: the same windows, in the same order, under the same names.
    assert_eq!(rig.eval("gnoblin.clients.length"), "2");
    assert_eq!(rig.eval("clients().len()"), "2");
    // A write of the engine (a title it cannot make, a position it can) reaches the compositor beside the library's own placements.
    rig.eval("[...gnoblin.clients][0].opacity = 0.5");
    rig.settle();
    assert_eq!(rig.comp.number(40, "opacity"), 0.5);
    assert_eq!(rig.comp.rect(40), "20,20,475,760");
    assert_eq!(rig.comp.refused(), 0);
}

#[test]
fn a_built_in_layout_of_the_library_is_named_by_a_class_and_runs_in_the_library() {
    let mut rig = rig_native(
        IMPORTS,
        r#"
const ws = createWorkspace("main", { gaps: [6, 12] });
new MasterStackLayout().attach(ws);
"#,
        world,
    );
    rig.comp.create_window(41, "Files", "nautilus");
    rig.settle();
    // A master on the left and one in the stack, in whole pixels, a 12 pixel margin and a 6 pixel gap.
    assert_eq!(rig.comp.rect(40), "12,12,485,776");
    assert_eq!(rig.comp.rect(41), "503,12,485,776");
    assert_eq!(rig.comp.refused(), 0);
    let _ = native::NativeCore::ctx;
}
