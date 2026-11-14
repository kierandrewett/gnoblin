//! The style model and its cascade (`data/js/gnoblin/style.ts` and the host half in `js/gnoblin.ts`), over the fake
//! compositor of `tests/common`: the cases ST1 to ST7 of design/js-research/spec-review.md section 5, what a resolved
//! style becomes on a window node, `defineConfig({ style })` and a window rule's `style`.
//!
//! Nothing here proves what the real compositor does with the node's `effects`: that is
//! `crates/gnoblin/tests/js_shell.rs`.

use gnoblin_proto::tree::effect;
use gnoblin_tree_client::tree_mirror::MirrorValue;
use serde_json::json;

mod common;
use common::*;

const IMPORTS: &str = "import * as g from \"gnoblin\";\nconst { gnoblin: _runtime, ...exported } = g;\nObject.assign(globalThis, exported);\n";

/// A shell over a screen of 1000 by 800 and two windows (ids 3 and 4, titled `win1` and `win2`, app id `foot`),
/// whose `init.ts` is `body`, settled. The only diagnostics it may have said are the `not applied` notices.
fn world(body: &str) -> Rig {
    let mut rig = rig_setup(IMPORTS, body, |comp| {
        comp.add_node(
            20,
            ROOT,
            "screen",
            vec![
                ("width", MirrorValue::Double(1000.0)),
                ("height", MirrorValue::Double(800.0)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        comp.add_window(3, "win1", "foot");
        comp.add_window(4, "win2", "foot");
    });
    let diagnostics = rig.settle();
    assert!(
        diagnostics.iter().all(|d| d.starts_with("not applied")),
        "{diagnostics:?} {:?}",
        rig.load_diagnostics
    );
    rig
}

/// Runs `code`, a strict function body that returns a string, with `caught(fn)` at hand: the message of the error
/// `fn` throws, or `none`.
fn js(rig: &mut Rig, code: &str) -> String {
    rig.eval(&format!(
        "(() => {{ 'use strict'; \
         const caught = (fn) => {{ try {{ fn(); return 'none'; }} catch (e) {{ return e.message; }} }};\n{code}\n}})()"
    ))
}

fn effects(rig: &Rig, node: u64) -> Vec<effect::Properties> {
    match rig.comp.value(node, "effects") {
        Some(MirrorValue::Effects(list)) => list,
        _ => Vec::new(),
    }
}

#[test]
fn st1_a_global_radius_reaches_current_and_future_clients_and_spares_an_explicit_one() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        "gnoblin.clients.style.cornerRadius = 12; gnoblin.clients.get('3').style.cornerRadius = 20; \
         return JSON.stringify(gnoblin.clients.map(c => c.computedStyle.cornerRadius));",
    );
    assert_eq!(out, "[20,12]");
    rig.comp.create_window(9, "late", "foot");
    rig.settle();
    let out = js(
        &mut rig,
        "gnoblin.clients.style.cornerRadius = 8; \
         return JSON.stringify(gnoblin.clients.map(c => [c.id, c.computedStyle.cornerRadius, c.computedStyle.origin.cornerRadius]));",
    );
    assert_eq!(
        out, r#"[["3",20,"client"],["4",8,"global"],["9",8,"global"]]"#,
        "the explicit override survives the global edit, a future client inherits"
    );
}

#[test]
fn st2_the_cascade_is_default_global_workspace_selections_by_priority_then_registration_then_client() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        r##"
        const ws = gnoblin.workspaces.at(0);
        // Registered in this order: the high priority first, so registration alone would rank it lowest.
        const selHigh = gnoblin.clients.where({ appId: "foot" }, { priority: 5 });
        const selLow1 = gnoblin.clients.where({ title: "win1" });
        const selLow2 = ws.clients.where({ appId: "foot" });
        selHigh.style; selLow1.style; selLow2.style;
        const client = gnoblin.clients.get("3");
        const scopes = { global: gnoblin.clients, workspace: ws.clients, selLow1, selLow2, selHigh };
        const names = ["global", "workspace", "selLow1", "selLow2", "selHigh", "client"];
        const styleOf = (name) => (name === "client" ? client.style : scopes[name].style);
        const props = {
            opacity: [0.3, 0.6],
            cornerRadius: [4, 8],
            background: ["#111111", "#222222"],
            shadows: [[{ blur: 3 }], [{ blur: 6 }]],
        };
        const read = (name) => { const v = client.computedStyle[name]; return name === "shadows" ? v[0]?.blur : v; };
        const want = (name, i) => (name === "shadows" ? props[name][i][0].blur : props[name][i]);
        const fails = [];
        const clear = () => { for (const n of names) styleOf(n).reset(); };
        for (const prop of Object.keys(props)) {
            clear();
            if (client.computedStyle.origin[prop] !== "default") fails.push(prop + " is not default when nothing sets it");
            for (let lo = 0; lo < names.length; lo += 1) {
                for (let hi = lo + 1; hi < names.length; hi += 1) {
                    clear();
                    styleOf(names[lo])[prop] = props[prop][0];
                    if (read(prop) !== want(prop, 0)) fails.push(`${prop}: ${names[lo]} alone`);
                    styleOf(names[hi])[prop] = props[prop][1];
                    if (read(prop) !== want(prop, 1)) fails.push(`${prop}: ${names[hi]} does not beat ${names[lo]}`);
                    clear();
                    styleOf(names[hi])[prop] = props[prop][1];
                    styleOf(names[lo])[prop] = props[prop][0];
                    if (read(prop) !== want(prop, 1)) fails.push(`${prop}: ${names[hi]} set first does not beat ${names[lo]}`);
                }
            }
        }
        clear();
        // Inheritance is per property: one scope sets the radius and another the opacity, and each is kept.
        ws.clients.style.cornerRadius = 5;
        selLow1.style.opacity = 0.5;
        const c = client.computedStyle;
        if (c.cornerRadius !== 5 || c.opacity !== 0.5) fails.push("per-property inheritance");
        if (c.origin.cornerRadius !== "workspace:" + ws.id) fails.push("origin " + c.origin.cornerRadius);
        if (!c.origin.opacity.startsWith("where(")) fails.push("origin " + c.origin.opacity);
        return JSON.stringify(fails);
        "##,
    );
    assert_eq!(out, "[]");
}

#[test]
fn st3_unset_reveals_the_inherited_value_null_stays_null_and_an_empty_list_is_explicit() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        r##"
        const client = gnoblin.clients.get("3");
        gnoblin.clients.style.background = "#112233";
        gnoblin.clients.style.shadows = [{ blur: 10 }];
        const log = [];
        log.push(client.computedStyle.background, client.style.background);
        client.style.background = null;
        log.push(client.computedStyle.background, client.style.background, client.computedStyle.origin.background);
        client.style.unset("background");
        log.push(client.computedStyle.background, client.style.background);
        client.style.shadows = [];
        log.push(client.computedStyle.shadows.length, client.computedStyle.origin.shadows);
        client.style.unset("shadows");
        log.push(client.computedStyle.shadows.length, client.computedStyle.origin.shadows);
        client.style.cornerRadius = 3;
        client.style.opacity = 0.5;
        client.style.reset();
        log.push(client.computedStyle.cornerRadius, client.computedStyle.opacity);
        log.push(caught(() => client.style.unset("sparkle")));
        return JSON.stringify(log);
        "##,
    );
    let expected = json!([
        "#112233",
        null,
        null,
        null,
        "client",
        "#112233",
        null,
        0,
        "client",
        1,
        "global",
        0,
        1,
        "Client#3.style.unset: no property \"sparkle\" (known: opacity, filter, backdropFilter, cornerRadius, borders, shadows, background)"
    ]);
    assert_eq!(out, expected.to_string());
}

#[test]
fn st4_writing_an_element_of_an_inherited_list_copies_the_list_into_the_target_and_leaks_nowhere() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        r#"
        gnoblin.clients.style.shadows = [{ blur: 10 }, { blur: 20, y: 4 }];
        const [a, b] = [gnoblin.clients.get("3"), gnoblin.clients.get("4")];
        const blurs = (c) => JSON.stringify(c.computedStyle.shadows.map(s => s.blur));
        const log = [];
        a.style.shadows[1].blur = 40;
        log.push(blurs(a), blurs(b), JSON.stringify(gnoblin.clients.style.shadows.map(s => s.blur)));
        log.push(a.computedStyle.origin.shadows, b.computedStyle.origin.shadows);
        // The copy detached a from later global edits; b still follows them.
        gnoblin.clients.style.shadows[0].blur = 30;
        log.push(blurs(a), blurs(b));
        // An in-place change of an inherited list is a write to a copy too, seen at the next frame.
        b.style.shadows.push({ blur: 1 });
        return JSON.stringify(log);
        "#,
    );
    assert_eq!(
        out,
        json!([
            "[10,40]", "[10,20]", "[10,20]", "client", "global", "[10,40]", "[30,20]"
        ])
        .to_string()
    );
    rig.settle();
    let out = js(
        &mut rig,
        "return JSON.stringify([gnoblin.clients.get('4'), gnoblin.clients.get('3')].map(c => [c.computedStyle.shadows.map(s => s.blur), c.computedStyle.origin.shadows]));",
    );
    assert_eq!(
        out, r#"[[[30,20,1],"client"],[[10,40],"client"]]"#,
        "the push on the copy made it b's own list, and only b's"
    );
}

#[test]
fn st5_computed_style_is_a_frozen_snapshot_and_a_global_element_edit_moves_only_the_clients_that_inherit() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        r#"
        gnoblin.clients.style.shadows = [{ blur: 10 }];
        const [a, b] = [gnoblin.clients.get("3"), gnoblin.clients.get("4")];
        a.style.shadows = [{ blur: 2 }];
        const snap = b.computedStyle;
        const log = [Object.isFrozen(snap), Object.isFrozen(snap.shadows), Object.isFrozen(snap.shadows[0]), Object.isFrozen(snap.origin)];
        log.push(caught(() => { snap.opacity = 0.1; }) !== "none");
        for (const blur of [12, 14, 16]) {
            gnoblin.clients.style.shadows[0].blur = blur;
            log.push(a.computedStyle.shadows[0].blur, b.computedStyle.shadows[0].blur);
        }
        log.push(snap.shadows[0].blur);
        return JSON.stringify(log);
        "#,
    );
    assert_eq!(out, "[true,true,true,true,true,2,12,2,14,2,16,10]");
}

#[test]
fn st7_an_invalid_value_is_refused_with_its_property_path_and_leaves_the_previous_state() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        r##"
        const client = gnoblin.clients.get("3");
        client.style.borders = [{ width: 1 }, { width: 2 }];
        client.style.shadows = [{ blur: 5 }];
        client.style.opacity = 0.5;
        const log = [];
        log.push(caught(() => { client.style.borders = [{}, {}, { width: -1 }]; }));
        log.push(caught(() => { client.style.opacity = NaN; }));
        log.push(caught(() => { client.style.opacity = 2; }));
        log.push(caught(() => { client.style.shadows[0].blur = -1; }));
        log.push(caught(() => { client.style.shadows = [{ blur: 1 }, { color: "red" }]; }));
        log.push(caught(() => { client.style.shadows = new Array(9).fill({ blur: 1 }); }));
        log.push(caught(() => { client.style.shadows = [{ sparkle: 1 }]; }));
        log.push(caught(() => { client.style.backdropFilter = [{ blur: 1, saturate: 2 }]; }));
        log.push(caught(() => { client.style.cornerRadius = { topLeft: 1 }; }));
        log.push(caught(() => { gnoblin.clients.style.shadows = [{ spread: 1000 }]; }));
        log.push(JSON.stringify(client.style.borders.map(b => b.width)), client.style.opacity, client.style.shadows[0].blur);
        return JSON.stringify(log);
        "##,
    );
    let expected = json!([
        "Client#3.style.borders[2].width: must be finite and >= 0",
        "Client#3.style.opacity: must be finite and between 0 and 1",
        "Client#3.style.opacity: must be finite and between 0 and 1",
        "Client#3.style.shadows[0].blur: must be finite and between 0 and 100",
        "Client#3.style.shadows[1].color: must be a #rgb, #rgba, #rrggbb or #rrggbbaa colour",
        "Client#3.style.shadows: holds at most 8 entries, got 9",
        "Client#3.style.shadows[0]: no field \"sparkle\" (known: x, y, blur, spread, color, inset)",
        "Client#3.style.backdropFilter[0]: must be a filter: { blur: 18 }, { saturate: 1.2 }, { brightness: 1.1 }, { contrast: 1.1 } or a BlurFilter, SaturateFilter, BrightnessFilter or ContrastFilter",
        "Client#3.style.cornerRadius.topRight: must be finite and >= 0",
        "gnoblin.clients.style.shadows[0].spread: must be finite and between -100 and 100",
        "[1,2]",
        0.5,
        5
    ]);
    assert_eq!(
        out,
        expected.to_string(),
        "every refusal names its path, and nothing changed"
    );
}

#[test]
fn a_write_to_the_style_of_a_destroyed_client_names_the_member() {
    let mut rig = world("");
    js(
        &mut rig,
        "globalThis.goneStyle = gnoblin.clients.get('4').style; return 'ready';",
    );
    rig.comp.destroy(4);
    rig.settle();
    let out = js(&mut rig, "return caught(() => { goneStyle.opacity = 0.5; });");
    assert_eq!(out, "Client#4.style: the client is gone");
}

#[test]
fn a_client_that_leaves_a_selection_loses_its_defaults_and_keeps_its_own() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        r#"
        gnoblin.clients.where({ appId: "foot" }).style.cornerRadius = 9;
        gnoblin.clients.get("4").style.opacity = 0.4;
        return JSON.stringify(gnoblin.clients.map(c => [c.computedStyle.cornerRadius, c.computedStyle.opacity]));
        "#,
    );
    assert_eq!(out, "[[9,1],[9,0.4]]");
    rig.comp.external_set(4, "app_id", text("firefox"));
    rig.settle();
    let out = js(
        &mut rig,
        "return JSON.stringify(gnoblin.clients.map(c => [c.computedStyle.cornerRadius, c.computedStyle.opacity]));",
    );
    assert_eq!(
        out, "[[9,1],[0,0.4]]",
        "4 left the selection: its radius is the default, its override stays"
    );
    rig.comp.external_set(4, "app_id", text("foot"));
    rig.settle();
    let out = js(&mut rig, "return gnoblin.clients.get('4').computedStyle.cornerRadius;");
    assert_eq!(out, "9", "and a client that joins it gets them");
}

#[test]
fn style_classes_are_ordinary_objects_and_literals_become_them() {
    let mut rig = world("");
    let out = js(
        &mut rig,
        r##"
        const s = gnoblin.clients.style;
        s.backdropFilter = [{ blur: 18 }, { saturate: 1.25 }, { brightness: 1.1 }, { contrast: 1.2 }];
        s.shadows = [{ blur: 4 }, new Shadow({ y: 2 })];
        s.borders = [{ width: 2, color: "#7c3aed", offset: 3 }];
        const log = [
            s.backdropFilter.map(f => f.constructor.name),
            s.shadows.every(x => x instanceof Shadow),
            s.borders[0] instanceof Border,
            s.backdropFilter[0].radius, s.backdropFilter[1].amount,
            s.shadows[0].color, s.shadows[0].spread, s.shadows[0].inset,
            s.borders[0].offset, s.borders[0].inset,
        ];
        gnoblin.clients.style = new ClientStyle({ cornerRadius: 3, shadows: [{ blur: 7 }], background: null });
        log.push(gnoblin.clients.style === s, s.cornerRadius, s.shadows[0].blur, s.background, s.borders.length, s.filter.length);
        gnoblin.clients.style = { opacity: 0.8 };
        log.push(s.opacity, s.cornerRadius, JSON.stringify(s));
        log.push(new BlurFilter().radius, new BrightnessFilter({ amount: 1.5 }).amount, new SaturateFilter().amount);
        log.push(new Style({ opacity: 0.5 }).opacity, caught(() => new Style({ borders: [] })));
        return JSON.stringify(log);
        "##,
    );
    let expected = json!([
        ["BlurFilter", "SaturateFilter", "BrightnessFilter", "ContrastFilter"],
        true,
        true,
        18,
        1.25,
        "#00000080",
        0,
        false,
        3,
        false,
        true,
        3,
        7,
        null,
        0,
        0,
        0.8,
        null,
        "{\"opacity\":0.8}",
        12,
        1.5,
        1,
        0.5,
        "Style: no field \"borders\" (known: opacity, filter, backdropFilter)"
    ]);
    assert_eq!(out, expected.to_string());
}

#[test]
fn a_resolved_style_becomes_the_node_effects_and_opacity_and_is_rewritten_when_an_element_changes() {
    let mut rig = world("");
    js(
        &mut rig,
        r##"
        gnoblin.clients.style.shadows = [
            { x: 1, y: 2, blur: 8, spread: 3, color: "#123" },
            { blur: 4, color: "#ff000080" },
            { inset: true, blur: 2 },
        ];
        gnoblin.clients.style.backdropFilter = [{ blur: 12 }, { saturate: 1.25 }, { brightness: 1.5 }];
        gnoblin.clients.get("3").style.opacity = 0.7;
        return "ok";
        "##,
    );
    let diagnostics = rig.settle();
    for node in [3, 4] {
        let list = effects(&rig, node);
        let kinds: Vec<&str> = list.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["blur", "shadow", "shadow"],
            "backdrop blur, then shadows last to first"
        );
        assert_eq!(
            (list[0].saturation, list[0].brightness, list[0].contrast),
            (1.25, 1.5, 1.0)
        );
        assert_eq!(
            (list[0].downscale, list[0].passes, list[0].algorithm.as_str()),
            (2, 3, "kawase")
        );
        assert_eq!((list[1].colour.as_str(), list[1].blur_radius), ("#ff000080", 4.0));
        assert_eq!(
            (
                list[2].offset_x,
                list[2].offset_y,
                list[2].blur_radius,
                list[2].spread,
                list[2].colour.as_str()
            ),
            (1.0, 2.0, 8.0, 3.0, "#112233ff"),
            "x, y, blur, spread and the colour, written out in full"
        );
    }
    assert_eq!(rig.comp.number(3, "opacity"), 0.7);
    assert_eq!(
        rig.comp.number(4, "opacity"),
        0.0,
        "a window whose style sets no opacity is not written"
    );
    assert_eq!(
        diagnostics,
        ["not applied: style.shadows: an inset shadow has no effect kind"],
        "said once for both windows"
    );
    assert!(rig.settle().is_empty(), "and not again");
    let out = js(
        &mut rig,
        "return JSON.stringify(gnoblin.clients.get('3').styleWarnings);",
    );
    assert_eq!(
        out,
        r#"["not applied: style.shadows: an inset shadow has no effect kind"]"#
    );

    // A live element edit: client 4's list is a copy, and writing to its element rewrites the node.
    js(
        &mut rig,
        "gnoblin.clients.get('4').style.shadows[0].blur = 20; return 'ok';",
    );
    rig.settle();
    assert_eq!(effects(&rig, 4)[2].blur_radius, 20.0);
    assert_eq!(effects(&rig, 3)[2].blur_radius, 8.0, "and only client 4's");

    // Taking the style away takes the effects away.
    js(
        &mut rig,
        "gnoblin.clients.style.reset(); gnoblin.clients.get('4').style.reset(); return 'ok';",
    );
    rig.settle();
    assert!(effects(&rig, 3).is_empty() && effects(&rig, 4).is_empty());
}

#[test]
fn what_no_effect_kind_draws_stays_in_computed_style_with_a_warning_and_a_notice() {
    let mut rig = world("");
    js(
        &mut rig,
        r##"
        gnoblin.clients.style.cornerRadius = 12;
        gnoblin.clients.style.borders = [{ width: 2 }];
        gnoblin.clients.style.background = "#000";
        gnoblin.clients.style.filter = [{ contrast: 1.1 }];
        gnoblin.clients.style.backdropFilter = [{ blur: 20 }];
        return "ok";
        "##,
    );
    let notices = rig.settle();
    assert_eq!(
        notices,
        [
            "not applied: style.backdropFilter: the compositor blurs at a fixed radius of 12, so a BlurFilter's radius only switches the blur on",
            "not applied: style.cornerRadius: the tree has no corner radius for a window",
            "not applied: style.borders: the tree has no border for a window",
            "not applied: style.background: the tree has no background for a window",
            "not applied: style.filter: no effect kind filters a window's own content",
        ]
    );
    let out = js(
        &mut rig,
        "const c = gnoblin.clients.get('3'); return JSON.stringify([c.computedStyle.cornerRadius, c.computedStyle.borders.length, c.styleWarnings.length]);",
    );
    assert_eq!(out, "[12,1,5]", "the values are still there");
    // A second window with the same style says nothing more.
    rig.comp.create_window(9, "late", "foot");
    assert!(rig.settle().is_empty());
}

#[test]
fn a_style_that_makes_more_than_eight_effects_is_reported_and_the_node_keeps_what_it_had() {
    let mut rig = world("");
    js(&mut rig, "gnoblin.clients.style.shadows = [{ blur: 1 }]; return 'ok';");
    rig.settle();
    js(
        &mut rig,
        "gnoblin.clients.style.shadows = new Array(8).fill({ blur: 2 }); gnoblin.clients.style.backdropFilter = [{ blur: 12 }]; return 'ok';",
    );
    let diagnostics = rig.settle();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.contains("a window carries at most 8 effects, the style makes 9")),
        "{diagnostics:?}"
    );
    assert_eq!(effects(&rig, 3).len(), 1, "the earlier list stays");
}

#[test]
fn the_compositors_skipped_effects_are_the_clients_style_warnings() {
    let mut rig = world("");
    rig.comp.external_set(
        3,
        "skipped",
        MirrorValue::Texts(vec!["shadow: unsupported-by-renderer".to_owned()]),
    );
    rig.settle();
    let out = js(
        &mut rig,
        "return JSON.stringify([gnoblin.clients.get('3').styleWarnings, gnoblin.clients.get('4').styleWarnings]);",
    );
    assert_eq!(
        out,
        r#"[["the compositor does not draw the effect shadow: unsupported-by-renderer"],[]]"#
    );
}

#[test]
fn define_config_style_is_the_global_default_and_a_bad_value_names_config_style() {
    let mut rig = world(
        r##"defineConfig({ style: { cornerRadius: 12, opacity: 0.9, shadows: [{ y: 8, blur: 24, color: "#00000066" }] } });"##,
    );
    for node in [3, 4] {
        assert_eq!(rig.comp.number(node, "opacity"), 0.9);
        let list = effects(&rig, node);
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].offset_y, list[0].blur_radius, list[0].colour.as_str()),
            (8.0, 24.0, "#00000066")
        );
    }
    let out = js(
        &mut rig,
        "const c = gnoblin.clients.get('3').computedStyle; return JSON.stringify([c.cornerRadius, c.origin.cornerRadius, c.origin.opacity]);",
    );
    assert_eq!(out, r#"[12,"global","global"]"#);
    let out = js(
        &mut rig,
        "return caught(() => desugarConfig({ style: { opacity: 3 } })) + '|' + caught(() => desugarConfig({ style: { shadows: [{ blur: -1 }] } })) + '|' + caught(() => desugarConfig({ style: { glow: 1 } }));",
    );
    assert_eq!(
        out,
        "config.style.opacity: must be finite and between 0 and 1|config.style.shadows[0].blur: must be finite and between 0 and 100|config.style: no field \"glow\" (known: opacity, filter, backdropFilter, cornerRadius, borders, shadows, background)"
    );
}

#[test]
fn a_rules_style_is_the_clients_own_style_and_a_bad_shadow_is_refused_when_the_rule_compiles() {
    let mut rig = world(
        r##"defineConfig({ style: { opacity: 0.9 }, rules: [{ appId: "dim", style: { opacity: 0.5, shadows: [{ blur: 3 }] } }] });"##,
    );
    rig.comp.create_window(9, "dimmed", "dim");
    rig.comp.create_window(10, "plain", "other");
    rig.settle();
    assert_eq!(rig.comp.number(9, "opacity"), 0.5, "the rule beats the global default");
    assert_eq!(rig.comp.number(10, "opacity"), 0.9);
    assert_eq!(effects(&rig, 9).len(), 1);
    assert!(effects(&rig, 10).is_empty());
    let out = js(
        &mut rig,
        "return JSON.stringify(gnoblin.clients.get('9').computedStyle.origin.opacity) + caught(() => compileRules([{ appId: 'x', style: { shadows: [{ blur: -1 }] } }]));",
    );
    assert_eq!(
        out,
        "\"client\"rules[0].style.shadows[0].blur: must be finite and between 0 and 100"
    );
}
