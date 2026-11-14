//! The two drag bindings of the tiling layouts, `ClientSwapBinding` and `SplitResizeBinding` (`data/js/gnoblin/bindings.ts`),
//! the dividers `MasterStackLayout` and `DwindleLayout` offer them (`Layout.findSplit`) and the order swap they stand on
//! (`MutableCollection.swap`, `Layout.onClientsSwapped`), over the fake compositor of `tests/common`.
//!
//! Cases follow `design/js-research/owner-spec.md` section 11 (snippets 051, 052, 104) and a Lua shell that had no such
//! drag: every case is new. A drag is a claimed pointer chord whose `start`, `update`, `end` and `cancel` the fake plays
//! back, with a frame between them. What the real compositor does with the writes is `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::*;

const IMPORTS: &str = "import * as g from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = g;\nObject.assign(globalThis, exported);\n";

const SCREEN: u64 = 20;
const SEAT: u64 = 21;
const SWAP: &str = "<Super>Button1";
const RESIZE: &str = "<Super>Button3";

/// A shell over a screen of 1000 by 800, one seat and `n` windows (ids 3 on) in the workspace `main`, whose layout is
/// `layout` (an expression). `counted(layout)` counts the layout's computes in `computes`. The windows are at boot, so
/// none has the focus.
fn world(n: u64, layout: &str) -> Rig {
    let init = format!(
        "globalThis.computes = 0;\n\
         const counted = (layout) => {{\n\
             const compute = layout.compute.bind(layout);\n\
             layout.compute = (context) => {{ globalThis.computes += 1; return compute(context); }};\n\
             return layout;\n\
         }};\n\
         defineConfig({{ workspaces: [{{ id: \"main\", layout: {layout} }}], decorations: false }});\n"
    );
    let mut rig = rig_setup(IMPORTS, &init, move |comp| {
        comp.add_node(
            SCREEN,
            ROOT,
            "screen",
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
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

fn exec(rig: &mut Rig, source: &str) {
    rig.shell.eval(source).expect("the script runs");
}

fn rect(rig: &Rig, node: u64) -> String {
    rig.comp.rect(node)
}

/// How many times the layout has computed.
fn computes(rig: &mut Rig) -> u64 {
    rig.eval("globalThis.computes").parse().expect("a count")
}

/// The workspace's clients in the order its layout places them, as node ids.
fn order(rig: &mut Rig) -> String {
    rig.eval("gnoblin.workspaces.get('main').clients.map((client) => client.id).join(',')")
}

/// A drag of the claimed chord: the press, the pointer at each of `path` in a frame of its own, then `how` it ends.
struct Drag {
    claim: i64,
    chord: &'static str,
    from: (f64, f64),
}

impl Drag {
    fn press(rig: &mut Rig, chord: &'static str, from: (f64, f64)) -> Drag {
        let claim = rig.comp.claim_id(chord);
        rig.comp.pointer_chord(claim, chord, "start", from, from);
        let diagnostics = rig.frame();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        Drag { claim, chord, from }
    }

    /// The pointer moves to `to`, and the shell gets one frame to answer.
    fn to(&self, rig: &mut Rig, to: (f64, f64)) {
        rig.comp.pointer_chord(self.claim, self.chord, "update", self.from, to);
        let diagnostics = rig.frame();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    fn release(&self, rig: &mut Rig, at: (f64, f64)) {
        rig.comp.pointer_chord(self.claim, self.chord, "end", self.from, at);
        let diagnostics = rig.settle();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    fn cancel(&self, rig: &mut Rig, at: (f64, f64)) {
        rig.comp.pointer_chord(self.claim, self.chord, "cancel", self.from, at);
        let diagnostics = rig.settle();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}

// ---- swapping -------------------------------------------------------------------------------------------------------

#[test]
fn dropping_a_tile_on_another_trades_their_places_and_the_layout_computes_once_on_release() {
    // Snippet 051 and 104. Master stack: window 3 is the master, 4 and 5 share the stack.
    let mut rig = world(3, "counted(new MasterStackLayout())");
    assert_eq!(rect(&rig, 3), "0,0,500,800");
    assert_eq!(rect(&rig, 4), "500,0,500,400");
    assert_eq!(rect(&rig, 5), "500,400,500,400");
    let before = computes(&mut rig);
    let drag = Drag::press(&mut rig, SWAP, (100.0, 100.0));
    drag.to(&mut rig, (400.0, 300.0));
    drag.to(&mut rig, (700.0, 500.0));
    drag.to(&mut rig, (720.0, 560.0));
    assert_eq!(order(&mut rig), "3,4,5", "nothing changes while the pointer is down");
    assert_eq!(rect(&rig, 3), "0,0,500,800");
    assert_eq!(rect(&rig, 5), "500,400,500,400");
    assert_eq!(computes(&mut rig), before, "the layout did not compute during the drag");
    drag.release(&mut rig, (720.0, 560.0));
    assert_eq!(order(&mut rig), "5,4,3", "3 and 5 traded places");
    assert_eq!(rect(&rig, 5), "0,0,500,800", "5 is the master now");
    assert_eq!(rect(&rig, 4), "500,0,500,400");
    assert_eq!(rect(&rig, 3), "500,400,500,400", "and 3 sits where 5 was");
    assert_eq!(computes(&mut rig), before + 1, "one compute for the whole drag");
}

#[test]
fn a_drop_outside_every_tile_on_the_tile_itself_or_a_cancel_changes_nothing() {
    let mut rig = world(3, "counted(new MasterStackLayout())");
    let before = computes(&mut rig);
    let drag = Drag::press(&mut rig, SWAP, (100.0, 100.0));
    drag.to(&mut rig, (700.0, 500.0));
    drag.to(&mut rig, (2000.0, 2000.0));
    drag.release(&mut rig, (2000.0, 2000.0));
    assert_eq!(order(&mut rig), "3,4,5", "released outside every tile");
    // The pointer was over tile 5 and then left: only where it is released counts.
    let drag = Drag::press(&mut rig, SWAP, (100.0, 100.0));
    drag.to(&mut rig, (200.0, 200.0));
    drag.release(&mut rig, (200.0, 200.0));
    assert_eq!(order(&mut rig), "3,4,5", "released over the tile that was held");
    let drag = Drag::press(&mut rig, SWAP, (100.0, 100.0));
    drag.to(&mut rig, (700.0, 500.0));
    drag.cancel(&mut rig, (700.0, 500.0));
    assert_eq!(order(&mut rig), "3,4,5", "cancelled over another tile");
    assert_eq!(rect(&rig, 3), "0,0,500,800");
    assert_eq!(computes(&mut rig), before, "no compute for a drag that changed nothing");
}

#[test]
fn dwindle_trades_two_leaves_of_its_tree() {
    // The tree, not the order, says where a node sits, so the layout hears of the swap (`onClientsSwapped`).
    let mut rig = world(3, "new DwindleLayout()");
    assert_eq!(rect(&rig, 3), "0,0,500,800");
    assert_eq!(rect(&rig, 4), "500,0,500,400");
    assert_eq!(rect(&rig, 5), "500,400,500,400");
    let drag = Drag::press(&mut rig, SWAP, (100.0, 100.0));
    drag.to(&mut rig, (700.0, 600.0));
    drag.release(&mut rig, (700.0, 600.0));
    assert_eq!(order(&mut rig), "5,4,3");
    assert_eq!(rect(&rig, 5), "0,0,500,800");
    assert_eq!(rect(&rig, 4), "500,0,500,400");
    assert_eq!(rect(&rig, 3), "500,400,500,400");
}

#[test]
fn a_grid_trades_two_cells() {
    let mut rig = world(4, "new GridLayout()");
    assert_eq!(rect(&rig, 3), "0,0,500,400");
    assert_eq!(rect(&rig, 6), "500,400,500,400");
    let drag = Drag::press(&mut rig, SWAP, (100.0, 100.0));
    drag.to(&mut rig, (700.0, 600.0));
    drag.release(&mut rig, (700.0, 600.0));
    assert_eq!(order(&mut rig), "6,4,5,3");
    assert_eq!(rect(&rig, 6), "0,0,500,400");
    assert_eq!(rect(&rig, 3), "500,400,500,400");
    assert_eq!(rect(&rig, 4), "500,0,500,400", "the others stay");
}

#[test]
fn a_floating_window_does_not_take_part_in_a_swap() {
    let mut rig = world(3, "new MasterStackLayout()");
    let start = "(() => {
        const client = gnoblin.clients.get('3');
        return [client.floating, typeof new ClientSwapBinding().onStart({ client, position: { x: 10, y: 10 } })].join(',');
    })()";
    assert_eq!(rig.eval(start), "false,object", "a tiled window starts a swap");
    exec(&mut rig, "gnoblin.clients.get('3').floating = true");
    rig.settle();
    assert_eq!(
        rig.eval(start),
        "true,boolean",
        "a window the script floated declines, though its tiled flag is still set"
    );
}

// ---- resizing a split ----------------------------------------------------------------------------------------------

#[test]
fn dragging_the_master_stack_divider_changes_the_ratio_live_and_keeps_it_on_release() {
    let mut rig = world(2, "counted(new MasterStackLayout())");
    assert_eq!(rect(&rig, 3), "0,0,500,800");
    let before = computes(&mut rig);
    let drag = Drag::press(&mut rig, RESIZE, (500.0, 400.0));
    drag.to(&mut rig, (600.0, 400.0));
    assert_eq!(
        rect(&rig, 3),
        "0,0,600,800",
        "the master follows the pointer while it is down"
    );
    assert_eq!(rect(&rig, 4), "600,0,400,800");
    assert_eq!(computes(&mut rig), before + 1, "one compute for the frame");
    drag.to(&mut rig, (700.0, 400.0));
    assert_eq!(rect(&rig, 3), "0,0,700,800");
    assert_eq!(computes(&mut rig), before + 2, "and one for the next");
    drag.release(&mut rig, (700.0, 400.0));
    assert_eq!(rect(&rig, 3), "0,0,700,800", "committed on release");
    assert_eq!(
        rig.eval("Math.round(gnoblin.workspaces.get('main').layout.ratio * 100)"),
        "70"
    );
    assert_eq!(computes(&mut rig), before + 2, "a release adds none");
}

#[test]
fn the_ratio_stays_within_the_layouts_limits_and_a_cancel_restores_it() {
    let mut rig = world(2, "new MasterStackLayout()");
    let drag = Drag::press(&mut rig, RESIZE, (500.0, 400.0));
    drag.to(&mut rig, (990.0, 400.0));
    assert_eq!(rect(&rig, 3), "0,0,900,800", "held to 0.9");
    drag.to(&mut rig, (5.0, 400.0));
    assert_eq!(rect(&rig, 3), "0,0,100,800", "and to 0.1");
    drag.release(&mut rig, (5.0, 400.0));
    assert_eq!(rig.eval("gnoblin.workspaces.get('main').layout.ratio"), "0.1");
    exec(&mut rig, "gnoblin.workspaces.get('main').layout.ratio = 0.5");
    rig.settle();
    let drag = Drag::press(&mut rig, RESIZE, (500.0, 400.0));
    drag.to(&mut rig, (600.0, 400.0));
    assert_eq!(rect(&rig, 3), "0,0,600,800");
    drag.cancel(&mut rig, (600.0, 400.0));
    assert_eq!(
        rect(&rig, 3),
        "0,0,500,800",
        "cancelled: the ratio the drag started from"
    );
    assert_eq!(rig.eval("gnoblin.workspaces.get('main').layout.ratio"), "0.5");
}

#[test]
fn the_divider_follows_the_pointer_on_the_right_and_across_a_gap() {
    // Master on the right: moving the pointer right shrinks it.
    let mut rig = world(2, "new MasterStackLayout({ orientation: \"right\" })");
    assert_eq!(rect(&rig, 3), "500,0,500,800");
    let drag = Drag::press(&mut rig, RESIZE, (500.0, 400.0));
    drag.to(&mut rig, (600.0, 400.0));
    assert_eq!(rect(&rig, 3), "600,0,400,800");
    assert_eq!(rect(&rig, 4), "0,0,600,800");
    drag.release(&mut rig, (600.0, 400.0));
    // A gap of 10 leaves 990 pixels to divide: the divider moves by exactly the pointer's travel.
    let mut rig = world(2, "new MasterStackLayout({ gap: 10 })");
    assert_eq!(rect(&rig, 3), "0,0,495,800");
    assert_eq!(rect(&rig, 4), "505,0,495,800");
    let drag = Drag::press(&mut rig, RESIZE, (600.0, 400.0));
    drag.to(&mut rig, (699.0, 400.0));
    assert_eq!(rect(&rig, 3), "0,0,594,800");
    assert_eq!(rect(&rig, 4), "604,0,396,800");
    drag.release(&mut rig, (699.0, 400.0));
    // Master on top: the pointer's y moves it.
    let mut rig = world(2, "new MasterStackLayout({ orientation: \"top\" })");
    assert_eq!(rect(&rig, 3), "0,0,1000,400");
    let drag = Drag::press(&mut rig, RESIZE, (500.0, 100.0));
    drag.to(&mut rig, (500.0, 180.0));
    assert_eq!(rect(&rig, 3), "0,0,1000,480");
    assert_eq!(rect(&rig, 4), "0,480,1000,320");
    drag.release(&mut rig, (500.0, 180.0));
}

#[test]
fn a_layout_with_no_divider_declines_a_resize() {
    // One window has no stack to divide from; a grid has no divider at all.
    let mut rig = world(1, "new MasterStackLayout()");
    let got = rig.eval(
        "(() => {
            const client = gnoblin.clients.get('3');
            const layout = gnoblin.workspaces.get('main').layout;
            return String(new SplitResizeBinding().onStart({ client, layout, position: { x: 10, y: 10 } }));
        })()",
    );
    assert_eq!(got, "false");
    let mut rig = world(3, "new GridLayout()");
    let got = rig.eval(
        "(() => {
            const client = gnoblin.clients.get('3');
            const layout = gnoblin.workspaces.get('main').layout;
            return [client.tiled, new SplitResizeBinding().onStart({ client, layout, position: { x: 10, y: 10 } })].join(',');
        })()",
    );
    assert_eq!(got, "true,false");
    assert!(
        !rig.comp.claimed().iter().any(|claim| claim.contains("Button3")),
        "a grid claims no resize chord: {:?}",
        rig.comp.claimed()
    );
    // A floating layout's windows are not tiled, so neither tiling binding takes them.
    let mut rig = world(2, "new FloatingLayout()");
    let got = rig.eval(
        "(() => {
            const client = gnoblin.clients.get('3');
            const layout = gnoblin.workspaces.get('main').layout;
            const at = { x: 10, y: 10 };
            return [
                client.tiled,
                new SplitResizeBinding().onStart({ client, layout, position: at }),
                new ClientSwapBinding().onStart({ client, layout, position: at }),
                layout.bindings.has('swap'),
                layout.bindings.has('resize'),
            ].join(',');
        })()",
    );
    assert_eq!(got, "false,false,false,false,true");
}

#[test]
fn dragging_a_dwindle_split_moves_that_split_alone() {
    // Window 3 on the left, 4 above 5 on the right: the outer split is vertical (a divider at x = 500) and the inner
    // is horizontal (a divider at y = 400).
    let mut rig = world(3, "counted(new DwindleLayout())");
    let before = computes(&mut rig);
    // Pressed in 5 at (700, 450): the inner divider is 50 pixels away and the outer one 200.
    let drag = Drag::press(&mut rig, RESIZE, (700.0, 450.0));
    drag.to(&mut rig, (700.0, 530.0));
    assert_eq!(
        rect(&rig, 4),
        "500,0,500,480",
        "the inner divider moved by the pointer's travel"
    );
    assert_eq!(rect(&rig, 5), "500,480,500,320");
    assert_eq!(rect(&rig, 3), "0,0,500,800", "the outer split stayed");
    assert_eq!(computes(&mut rig), before + 1);
    drag.release(&mut rig, (700.0, 530.0));
    assert_eq!(
        rig.eval("gnoblin.workspaces.get('main').layout.ratio"),
        "0.5",
        "the layout's own ratio is the same"
    );
    // Pressed in 3: the only split above it is the outer one.
    let drag = Drag::press(&mut rig, RESIZE, (100.0, 300.0));
    drag.to(&mut rig, (200.0, 300.0));
    assert_eq!(rect(&rig, 3), "0,0,600,800");
    assert_eq!(
        rect(&rig, 4),
        "600,0,400,480",
        "the inner split keeps what the first drag gave it"
    );
    assert_eq!(rect(&rig, 5), "600,480,400,320");
    drag.release(&mut rig, (200.0, 300.0));
}

#[test]
fn a_cancelled_dwindle_drag_gives_the_split_back_to_the_layouts_ratio() {
    let mut rig = world(3, "new DwindleLayout()");
    let drag = Drag::press(&mut rig, RESIZE, (700.0, 450.0));
    drag.to(&mut rig, (700.0, 530.0));
    assert_eq!(rect(&rig, 4), "500,0,500,480");
    drag.cancel(&mut rig, (700.0, 530.0));
    assert_eq!(rect(&rig, 4), "500,0,500,400");
    // The split follows `ratio` again, as one nobody dragged does.
    exec(&mut rig, "gnoblin.workspaces.get('main').layout.ratio = 0.3");
    rig.settle();
    assert_eq!(rect(&rig, 3), "0,0,300,800");
    assert_eq!(rect(&rig, 4), "300,0,700,240");
    assert_eq!(rect(&rig, 5), "300,240,700,560");
}

// ---- configuration --------------------------------------------------------------------------------------------------

#[test]
fn the_tiling_layouts_register_their_bindings_and_triggers_override_them() {
    let mut rig = world(1, "new Layout()");
    let got = rig.eval(
        "(() => {
            const list = (layout) => [...layout.bindings].map((b) => `${b.id}=${b.trigger}`).join(' ');
            const dwindle = new DwindleLayout();
            dwindle.bindings.remove('swap');
            return [
                list(new MasterStackLayout()),
                list(new MasterStackLayout({ triggers: { swap: 'Alt+MouseLeft+Drag' } })),
                list(new MasterStackLayout({ triggers: { swap: null } })),
                list(new MasterStackLayout({ triggers: { resize: null } })),
                list(new DwindleLayout({ triggers: { resize: 'Alt+MouseRight+Drag' } })),
                list(dwindle),
                list(new GridLayout()),
                list(new GridLayout({ triggers: { swap: null } })),
                list(new MonocleLayout()),
                list(new FloatingLayout()),
            ].join('\\n');
        })()",
    );
    assert_eq!(
        got,
        [
            "swap=Super+MouseLeft+Drag resize=Super+MouseRight+Drag",
            "swap=Alt+MouseLeft+Drag resize=Super+MouseRight+Drag",
            "resize=Super+MouseRight+Drag",
            "swap=Super+MouseLeft+Drag",
            "swap=Super+MouseLeft+Drag resize=Alt+MouseRight+Drag",
            "resize=Super+MouseRight+Drag",
            "swap=Super+MouseLeft+Drag",
            "",
            "",
            "move=Super+MouseLeft+Drag resize=Super+MouseRight+Drag",
        ]
        .join("\n")
    );
    let got = rig.eval(
        "(() => {
            const swap = new ClientSwapBinding();
            const split = new SplitResizeBinding({ trigger: 'Alt+MouseRight+Drag', threshold: 10 });
            return [swap.id, swap.trigger, swap.threshold, split.id, split.trigger, split.threshold].join(',');
        })()",
    );
    assert_eq!(
        got, "swap,Super+MouseLeft+Drag,4,resize,Alt+MouseRight+Drag,10",
        "the ids and triggers the owner spec gives"
    );
}

#[test]
fn a_swap_trigger_the_config_remaps_is_the_chord_the_shell_claims() {
    let mut rig = world(
        3,
        "new MasterStackLayout({ triggers: { swap: \"Alt+MouseLeft+Drag\" } })",
    );
    let claimed = rig.comp.claimed();
    assert!(
        claimed.iter().any(|claim| claim.contains("<Alt>Button1")),
        "{claimed:?}"
    );
    assert!(!claimed.iter().any(|claim| claim == SWAP), "{claimed:?}");
    let drag = Drag::press(&mut rig, "<Alt>Button1", (100.0, 100.0));
    drag.to(&mut rig, (700.0, 600.0));
    drag.release(&mut rig, (700.0, 600.0));
    assert_eq!(order(&mut rig), "5,4,3");
    // Removing the binding takes the claim with it.
    exec(
        &mut rig,
        "gnoblin.workspaces.get('main').layout.bindings.remove('swap')",
    );
    rig.settle();
    assert!(
        !rig.comp.claimed().iter().any(|claim| claim.contains("Button1")),
        "{:?}",
        rig.comp.claimed()
    );
}

// ---- the order ------------------------------------------------------------------------------------------------------

#[test]
fn a_collection_swap_trades_two_members_and_tells_the_layout() {
    let mut rig = world(
        3,
        "(() => { const layout = new Layout(); layout.heard = []; layout.onClientsSwapped = (a, b) => layout.heard.push(a.id + b.id); return layout; })()",
    );
    let got = rig.eval(
        "(() => {
            const workspace = gnoblin.workspaces.get('main');
            const [a, b, c] = workspace.clients.map((client) => client);
            workspace.clients.swap(a, c);
            workspace.clients.swap(b, b);
            const first = workspace.clients.map((client) => client.id).join(',');
            let refused = 'no error';
            try {
                workspace.clients.swap(a, {});
            } catch (error) {
                refused = error.code;
            }
            return [first, refused, workspace.layout.heard.join(';')].join('|');
        })()",
    );
    assert_eq!(
        got, "5,4,3|not-a-member|35",
        "the hook heard the one swap, and not a swap with itself"
    );
}
