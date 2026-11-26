//! Style and transitions (`gnoblin-std/style/*`, `gnoblin-std/transition/*`): the cascade, the refusal messages, what a resolved style
//! becomes on a window node, shader filters, and what `requestTransition` writes.

use crate::scn::{At, Home, Scn, V, World};

/// A screen of 1000 by 800 and two windows (ids 3 and 4, titled `win1` and `win2`, app id `foot`).
fn world() -> World {
    World::desktop(1000.0, 800.0).windows(2)
}

/// `caught(fn)` is the message of the error `fn` throws, or `none`.
const CAUGHT: &str = "globalThis.caught = (fn: () => void) => { try { fn(); return 'none'; } catch (e) { return (e as Error).message; } };\n";

#[test]
fn cascade() {
    let mut s = Scn::new("style_cascade", Home::Reference, &world(), CAUGHT);
    s.boot();
    s.eval(
        "gnoblin.clients.style.cornerRadius = 12; gnoblin.clients.get('3').style.cornerRadius = 20;\n\
         rec('radii', gnoblin.clients.map((c) => c.computedStyle.cornerRadius));",
    )
    .step("global_and_explicit");
    s.open(9, "late", "foot")
        .eval(
            "gnoblin.clients.style.cornerRadius = 8;\n\
             rec('radii', gnoblin.clients.map((c) => [c.id, c.computedStyle.cornerRadius, c.computedStyle.origin.cornerRadius]));",
        )
        .step("future_clients_inherit");
    s.eval(
        "const ws = gnoblin.workspaces.at(0);\n\
         const selHigh = gnoblin.clients.where({ appId: 'foot' }, { priority: 5 });\n\
         const selLow1 = gnoblin.clients.where({ title: 'win1' });\n\
         const selLow2 = ws.clients.where({ appId: 'foot' });\n\
         selHigh.style; selLow1.style; selLow2.style;\n\
         const client = gnoblin.clients.get('3');\n\
         const scopes = { global: gnoblin.clients, workspace: ws.clients, selLow1, selLow2, selHigh };\n\
         const names = ['global', 'workspace', 'selLow1', 'selLow2', 'selHigh', 'client'];\n\
         const styleOf = (name) => (name === 'client' ? client.style : scopes[name].style);\n\
         const props = { opacity: [0.3, 0.6], cornerRadius: [4, 8], background: ['#111111', '#222222'], shadows: [[{ blur: 3 }], [{ blur: 6 }]] };\n\
         const read = (name) => { const v = client.computedStyle[name]; return name === 'shadows' ? v[0]?.blur : v; };\n\
         const clear = () => { for (const n of names) styleOf(n).reset(); };\n\
         const table = [];\n\
         for (const prop of Object.keys(props)) {\n\
           clear();\n\
           const none = client.computedStyle.origin[prop];\n\
           for (let lo = 0; lo < names.length; lo += 1) {\n\
             for (let hi = lo + 1; hi < names.length; hi += 1) {\n\
               clear(); styleOf(names[lo])[prop] = props[prop][0]; const alone = read(prop);\n\
               styleOf(names[hi])[prop] = props[prop][1]; const later = read(prop);\n\
               clear(); styleOf(names[hi])[prop] = props[prop][1]; styleOf(names[lo])[prop] = props[prop][0]; const earlier = read(prop);\n\
               table.push([prop, names[lo], names[hi], none, alone, later, earlier]);\n\
             }\n\
           }\n\
         }\n\
         clear();\n\
         ws.clients.style.cornerRadius = 5; selLow1.style.opacity = 0.5;\n\
         const c = client.computedStyle;\n\
         rec('matrix', table);\n\
         rec('per_property', [c.cornerRadius, c.opacity, c.origin.cornerRadius, String(c.origin.opacity).replace(/where\\(.*/, 'where(...)')]);\n\
         globalThis.workspaceId = ws.id;",
    )
    .step("priority_and_registration_order");
    s.eval(
        "gnoblin.clients.style.reset(); for (const c of gnoblin.clients) c.style.reset();\n\
         const client = gnoblin.clients.get('3'); const log = [];\n\
         gnoblin.clients.style.background = '#112233'; gnoblin.clients.style.shadows = [{ blur: 10 }];\n\
         log.push(client.computedStyle.background, client.style.background);\n\
         client.style.background = null;\n\
         log.push(client.computedStyle.background, client.style.background, client.computedStyle.origin.background);\n\
         client.style.unset('background');\n\
         log.push(client.computedStyle.background, client.style.background);\n\
         client.style.shadows = [];\n\
         log.push(client.computedStyle.shadows.length, client.computedStyle.origin.shadows);\n\
         client.style.unset('shadows');\n\
         log.push(client.computedStyle.shadows.length, client.computedStyle.origin.shadows);\n\
         client.style.cornerRadius = 3; client.style.opacity = 0.5; client.style.reset();\n\
         log.push(client.computedStyle.cornerRadius, client.computedStyle.opacity);\n\
         log.push(caught(() => client.style.unset('sparkle')));\n\
         rec('log', log);",
    )
    .step("unset_null_and_empty");
    s.eval(
        "gnoblin.clients.style.reset(); for (const c of gnoblin.clients) c.style.reset();\n\
         gnoblin.clients.style.shadows = [{ blur: 10 }, { blur: 20, y: 4 }];\n\
         const [a, b] = [gnoblin.clients.get('3'), gnoblin.clients.get('4')];\n\
         const blurs = (c) => c.computedStyle.shadows.map((s) => s.blur); const log = [];\n\
         a.style.shadows[1].blur = 40;\n\
         log.push(blurs(a), blurs(b), gnoblin.clients.style.shadows.map((s) => s.blur), a.computedStyle.origin.shadows, b.computedStyle.origin.shadows);\n\
         gnoblin.clients.style.shadows[0].blur = 30; log.push(blurs(a), blurs(b));\n\
         b.style.shadows.push({ blur: 1 });\n\
         rec('log', log);",
    )
    .step("lists_copy_on_write");
    s.eval("rec('after', [gnoblin.clients.get('4'), gnoblin.clients.get('3')].map((c) => [c.computedStyle.shadows.map((s) => s.blur), c.computedStyle.origin.shadows]));")
        .step("lists_after_a_frame");
    s.eval(
        "gnoblin.clients.style.reset(); for (const c of gnoblin.clients) c.style.reset();\n\
         gnoblin.clients.style.shadows = [{ blur: 10 }];\n\
         const [a, b] = [gnoblin.clients.get('3'), gnoblin.clients.get('4')];\n\
         a.style.shadows = [{ blur: 2 }];\n\
         const snap = b.computedStyle;\n\
         const log = [Object.isFrozen(snap), Object.isFrozen(snap.shadows), Object.isFrozen(snap.shadows[0]), Object.isFrozen(snap.origin)];\n\
         log.push(caught(() => { snap.opacity = 0.1; }) !== 'none');\n\
         for (const blur of [12, 14, 16]) { gnoblin.clients.style.shadows[0].blur = blur; log.push(a.computedStyle.shadows[0].blur, b.computedStyle.shadows[0].blur); }\n\
         log.push(snap.shadows[0].blur);\n\
         rec('log', log);",
    )
    .step("computed_style_is_frozen");
    s.finish();
}

#[test]
fn refusals_and_classes() {
    let mut s = Scn::new("style_refusals", Home::Reference, &world(), CAUGHT);
    s.boot();
    s.eval(
        "const client = gnoblin.clients.get('3');\n\
         client.style.borders = [{ width: 1 }, { width: 2 }]; client.style.shadows = [{ blur: 5 }]; client.style.opacity = 0.5;\n\
         const log = [];\n\
         log.push(caught(() => { client.style.borders = [{}, {}, { width: -1 }]; }));\n\
         log.push(caught(() => { client.style.opacity = NaN; }));\n\
         log.push(caught(() => { client.style.opacity = 2; }));\n\
         log.push(caught(() => { client.style.shadows[0].blur = -1; }));\n\
         log.push(caught(() => { client.style.shadows = [{ blur: 1 }, { color: 'red' }]; }));\n\
         log.push(caught(() => { client.style.shadows = new Array(9).fill({ blur: 1 }); }));\n\
         log.push(caught(() => { client.style.shadows = [{ sparkle: 1 }]; }));\n\
         log.push(caught(() => { client.style.backdropFilter = [{ blur: 1, saturate: 2 }]; }));\n\
         log.push(caught(() => { client.style.cornerRadius = { topLeft: 1 }; }));\n\
         log.push(caught(() => { gnoblin.clients.style.shadows = [{ spread: 1000 }]; }));\n\
         log.push(client.style.borders.map((b) => b.width), client.style.opacity, client.style.shadows[0].blur);\n\
         rec('log', log);",
    )
    .step("refusals_name_the_path_and_change_nothing");
    s.eval("globalThis.goneStyle = gnoblin.clients.get('4').style;")
        .destroy(4)
        .eval("rec('gone', caught(() => { goneStyle.opacity = 0.5; }));")
        .step("style_of_a_destroyed_client");
    s.finish();

    let mut s = Scn::new("style_selection_membership", Home::Reference, &world(), CAUGHT);
    s.boot();
    s.eval(
        "gnoblin.clients.where({ appId: 'foot' }).style.cornerRadius = 9; gnoblin.clients.get('4').style.opacity = 0.4;\n\
         rec('styles', gnoblin.clients.map((c) => [c.computedStyle.cornerRadius, c.computedStyle.opacity]));",
    )
    .step("selection_style");
    s.set(4, "app_id", V::Text("firefox"))
        .eval("rec('styles', gnoblin.clients.map((c) => [c.computedStyle.cornerRadius, c.computedStyle.opacity]));")
        .step("left_the_selection");
    s.set(4, "app_id", V::Text("foot"))
        .eval("rec('radius', gnoblin.clients.get('4').computedStyle.cornerRadius);")
        .step("joined_again");
    s.finish();

    let mut s = Scn::new("style_classes", Home::Reference, &world(), CAUGHT);
    s.boot();
    s.eval(
        "const s = gnoblin.clients.style;\n\
         s.backdropFilter = [{ blur: 18 }, { saturate: 1.25 }, { brightness: 1.1 }, { contrast: 1.2 }];\n\
         s.shadows = [{ blur: 4 }, new Shadow({ y: 2 })];\n\
         s.borders = [{ width: 2, color: '#7c3aed', offset: 3 }];\n\
         const log = [\n\
           s.backdropFilter.map((f) => f.constructor.name), s.shadows.every((x) => x instanceof Shadow), s.borders[0] instanceof Border,\n\
           s.backdropFilter[0].radius, s.backdropFilter[1].amount, s.shadows[0].color, s.shadows[0].spread, s.shadows[0].inset,\n\
           s.borders[0].offset, s.borders[0].inset,\n\
         ];\n\
         gnoblin.clients.style = new ClientStyle({ cornerRadius: 3, shadows: [{ blur: 7 }], background: null });\n\
         log.push(gnoblin.clients.style === s, s.cornerRadius, s.shadows[0].blur, s.background, s.borders.length, s.filter.length);\n\
         gnoblin.clients.style = { opacity: 0.8 };\n\
         log.push(s.opacity, s.cornerRadius, JSON.stringify(s));\n\
         log.push(new BlurFilter().radius, new BrightnessFilter({ amount: 1.5 }).amount, new SaturateFilter().amount);\n\
         log.push(new Style({ opacity: 0.5 }).opacity, caught(() => new Style({ borders: [] })));\n\
         rec('log', log);",
    )
    .step("classes_and_literals");
    s.finish();
}

#[test]
fn effects_on_the_node() {
    let mut s = Scn::new("style_effects", Home::Reference, &world(), CAUGHT);
    s.boot();
    s.eval(
        "gnoblin.clients.style.shadows = [{ x: 1, y: 2, blur: 8, spread: 3, color: '#123' }, { blur: 4, color: '#ff000080' }, { inset: true, blur: 2 }];\n\
         gnoblin.clients.style.backdropFilter = [{ blur: 12 }, { saturate: 1.25 }, { brightness: 1.5 }];\n\
         gnoblin.clients.get('3').style.opacity = 0.7;",
    )
    .step("shadows_and_backdrop");
    s.tree("windows", At::Id(3));
    s.tree("windows", At::Id(4));
    s.eval("rec('warnings', gnoblin.clients.get('3').styleWarnings);")
        .step("warnings");
    s.eval("gnoblin.clients.get('4').style.shadows[0].blur = 20;")
        .step("element_edit_on_a_copy");
    s.tree("windows", At::Id(4));
    s.eval("gnoblin.clients.style.reset(); gnoblin.clients.get('4').style.reset(); gnoblin.clients.get('3').style.reset();").step("reset");
    s.tree("windows", At::Id(3));
    s.eval(
        "gnoblin.clients.style.cornerRadius = 12;\n\
         gnoblin.clients.style.borders = [{ width: 2, color: '#7c3aed', offset: 3 }, { width: 1, inset: true }];\n\
         gnoblin.clients.style.shadows = [{ blur: 8 }];",
    )
    .step("radius_borders_and_a_frame_shadow");
    s.tree("windows", At::Id(3));
    s.eval("gnoblin.clients.style.cornerRadius = { topLeft: 4, topRight: 8, bottomRight: 8, bottomLeft: 4 };")
        .step("corners_that_differ");
    s.tree("windows", At::Id(3));
    s.eval("gnoblin.clients.style.reset();").step("reset_again");
    s.eval(
        "gnoblin.clients.style.cornerRadius = 12; gnoblin.clients.style.borders = [{ width: 2 }]; gnoblin.clients.style.background = '#000';\n\
         gnoblin.clients.style.filter = [{ contrast: 1.1 }]; gnoblin.clients.style.backdropFilter = [{ blur: 20 }];",
    )
    .step("a_background_a_filter_and_a_blur_over_the_limit");
    s.eval("const c = gnoblin.clients.get('3'); rec('kept', [c.computedStyle.cornerRadius, c.computedStyle.borders.length, c.styleWarnings]);").step("kept_in_computed_style");
    s.open(9, "late", "foot").step("a_second_window_says_nothing_more");
    s.eval("gnoblin.clients.style.reset();").step("reset_third");
    s.set(3, "skipped", V::Texts(&["border: unsupported-by-renderer"]))
        .step("compositor_skipped_border");
    s.eval("gnoblin.clients.style.borders = [{ width: 2 }]; rec('warnings', gnoblin.clients.get('3').styleWarnings);")
        .step("border_warning");
    s.eval("gnoblin.clients.style.reset();").step("reset_fourth");
    s.finish();

    let mut s = Scn::new("style_effect_limit", Home::Reference, &world(), CAUGHT);
    s.boot();
    s.eval("gnoblin.clients.style.shadows = [{ blur: 1 }];")
        .step("one_shadow");
    s.eval(
        "gnoblin.clients.style.shadows = new Array(8).fill({ blur: 2 }); gnoblin.clients.style.borders = new Array(8).fill({ width: 1 });\n\
         gnoblin.clients.style.backdropFilter = [{ blur: 12 }];",
    )
    .step("seventeen_effects");
    s.tree("window", At::Id(3));
    s.finish();

    let mut s = Scn::new(
        "style_global_and_selection",
        Home::Reference,
        &world(),
        "gnoblin.clients.style.cornerRadius = 12; gnoblin.clients.style.opacity = 0.9;\n\
         gnoblin.clients.style.shadows = [{ y: 8, blur: 24, color: '#00000066' }];\n\
         const dim = gnoblin.clients.where({ appId: 'dim' }); dim.style.opacity = 0.5; dim.style.shadows = [{ blur: 3 }];\n\
         globalThis.caught = (fn: () => void) => { try { fn(); return 'none'; } catch (e) { return (e as Error).message; } };\n",
    );
    s.boot();
    s.tree("window", At::Id(3));
    s.eval("const c = gnoblin.clients.get('3').computedStyle; rec('origin', [c.cornerRadius, c.origin.cornerRadius, c.origin.opacity]);").step("origins");
    s.eval(
        "rec('refused', [caught(() => { gnoblin.clients.style.opacity = 3; }), caught(() => { gnoblin.clients.style.shadows = [{ blur: -1 }]; })]);",
    )
    .step("refusals");
    s.open(10, "dimmed", "dim")
        .open(11, "plain", "other")
        .step("two_more_windows");
    s.tree("dimmed", At::Id(10));
    s.tree("plain", At::Id(11));
    s.eval("rec('origin', [String(gnoblin.clients.get('10').computedStyle.origin.opacity).startsWith('where('), gnoblin.clients.get('11').computedStyle.origin.opacity]);")
        .step("selection_beats_global");
    s.finish();
}

const TINT: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> { return c * p[0]; }";

#[test]
fn shader_filters() {
    let body = format!("{CAUGHT}globalThis.TINT = '{TINT}';\n");
    let mut s = Scn::new(
        "style_shader",
        Home::Reference,
        &world().node(21, "user_shaders", V::Bool(true)),
        &body,
    );
    s.boot();
    s.eval(
        "const f = new ShaderFilter({ source: TINT, params: [0.5, 2] });\n\
         gnoblin.clients.style.filter = [f];\n\
         gnoblin.clients.get('4').style.backdropFilter = [{ blur: 12 }, { shader: { source: TINT } }];\n\
         rec('filter', [f instanceof StyleElement, f.params, gnoblin.clients.style.filter[0] === f, JSON.stringify(f)]);",
    )
    .step("accepted");
    s.tree("content_filter", At::Id(3));
    s.tree("backdrop_and_content", At::Id(4));
    s.eval(
        "const f = new ShaderFilter({ source: TINT });\n\
         rec('refused', [caught(() => { f.source = '  '; }), caught(() => { f.source = 'x'.repeat(16385); }), caught(() => { f.params = new Array(9).fill(0); }),\n\
           caught(() => { f.params = [NaN]; }), caught(() => { gnoblin.clients.style.filter = [{ shader: 'fn effect() {}' }]; }),\n\
           caught(() => { new ShaderFilter({ sparkle: 1 }); }), f.source === TINT]);",
    )
    .step("refusals");
    s.finish();

    // The compositor accepts shaders unless the seat says `user_shaders = false`: with no property the shader is drawn, with `false` it is refused.
    for (label, seat) in [("default_on", None), ("switched_off", Some(false))] {
        let mut world = world();
        if let Some(on) = seat {
            world = world.node(21, "user_shaders", V::Bool(on));
        }
        let mut s = Scn::new(&format!("style_shader_{label}"), Home::Reference, &world, &body);
        s.boot();
        s.eval("gnoblin.clients.style.shadows = [{ y: 4 }]; gnoblin.clients.style.filter = [new ShaderFilter({ source: TINT })];").step("written");
        s.tree("window", At::Id(3));
        s.eval("rec('warnings', gnoblin.clients.get('3').styleWarnings);")
            .step("warnings");
        s.finish();
    }
}

/// Two windows (3 and 4) tiled by a layout, as `crates/gnoblin-js/tests/transition.rs` has them.
const TWO: &str = "class Fill extends Layout {\n\
        compute(c: any) { return c.children.map((n: any) => ({ node: n, tiled: true, bounds: { ...c.area } })); }\n\
    }\n\
    for (const id of ['1', '2', '3']) gnoblin.workspaces.add(new Workspace({ id, layout: LAYOUT }));\n\
    globalThis.fail = (f: () => void) => { try { f(); return 'accepted'; } catch (e) { return (e as Error).name + ': ' + (e as Error).message; } };\n";

fn two(layout: &str) -> (World, String) {
    (World::policy(2, false), TWO.replace("LAYOUT", layout))
}

const HALF: &str = "g.a = gnoblin.clients.get('3'); g.b = gnoblin.clients.get('4'); g.ws = gnoblin.workspaces.get('1'); g.computes = 0; g.calls = 0;\n\
    g.Half = class extends Layout { compute(c) { g.computes += 1; const h = c.area.height / 2;\n\
      return c.children.map((n, i) => ({ node: n, tiled: true, bounds: { x: c.area.x, y: c.area.y + i * h, width: c.area.width, height: h } })); } };\n";

#[test]
fn transition_writes() {
    let (world, body) = two("new Layout()");
    let mut s = Scn::new("transition_writes", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval("gnoblin.requestTransition(() => { g.a.opacity = 0.5; }, { duration: 100 });")
        .advance(1000)
        .step("only_the_callbacks_write");
    s.eval("g.t = gnoblin.requestTransition(() => { g.calls += 1; g.a.opacity = 0.5; g.b.opacity = 0.25; g.a.x = 70; }, { duration: 200, easing: 'ease-out' }); rec('calls', g.calls);")
        .advance(1000)
        .eval("rec('calls', g.calls);")
        .step("three_writes_one_transaction");
    for easing in ["linear", "ease", "ease-in", "ease-out", "ease-in-out"] {
        s.eval(&format!(
            "gnoblin.requestTransition(() => {{ g.a.opacity = 0.5; }}, {{ duration: 10, easing: '{easing}' }});"
        ))
        .step(&format!("easing_{easing}"));
    }
    s.eval(
        "rec('refused', [\n\
           fail(() => gnoblin.requestTransition(() => { g.calls += 1; }, { easing: 'bouncy' })),\n\
           fail(() => gnoblin.requestTransition(() => { g.calls += 1; }, { duration: -1 })),\n\
           fail(() => gnoblin.requestTransition(() => { g.calls += 1; }, { autoplay: 'no' })),\n\
           fail(() => gnoblin.requestTransition(() => { g.calls += 1; }, { delay: -10 })),\n\
           fail(() => gnoblin.requestTransition(() => { g.calls += 1; }, { speed: 2 })),\n\
         ]); rec('calls', g.calls);",
    )
    .step("options_refused_before_the_callback_runs");
    s.eval("rec('seen', (() => { let seen = ''; gnoblin.requestTransition(() => { g.a.opacity = 0.3; g.a.rotation = [0, 1, 0]; seen = g.a.opacity + '|' + g.a.rotation.join(','); }, { duration: 100 }); return seen; })());")
        .step("reads_see_the_staged_value");
    s.eval("g.a.opacity = 0.9;").step("a_plain_write_is_immediate");
    s.eval("gnoblin.requestTransition(() => { g.a.rotation = [0, 0.5, 0]; g.a.scale = [2, 2, 1]; gnoblin.clients.style.opacity = 0.6; g.b.z = 40; }, { duration: 250, easing: 'ease-out' });")
        .step("opacity_rotation_scale_z");
    s.finish();

    let (world, body) = two("new DwindleLayout()");
    let mut s = Scn::new("transition_layouts", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval("g.t = gnoblin.requestTransition(() => { g.ws.layout = new g.Half(); }, { duration: 300, easing: 'ease-out' }); rec('computes', g.computes);")
        .eval("rec('computes', g.computes);")
        .step("layout_in_the_callback");
    s.eval("g.ws.layout = new DwindleLayout(); g.computes = 0;")
        .step("back_to_dwindle");
    s.eval("g.ws.layout = new MasterStackLayout({ ratio: 0.5 });")
        .step("master_stack");
    s.eval("gnoblin.requestTransition(() => { g.ws.layout.ratio = 0.7; }, { duration: 150 });")
        .step("layout_field_in_the_callback");
    s.set(4, "visible", V::Bool(false))
        .eval("gnoblin.requestTransition(() => { g.ws.layout = new g.Half(); }, { duration: 300 });")
        .step("a_window_with_no_place");
    s.finish();

    let (world, body) = two("new DwindleLayout()");
    let mut s = Scn::new("transition_reduced_motion", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.seat("reduced_motion", V::Bool(true));
    s.eval(
        "g.done = []; g.t = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.ws.layout = new g.Half(); }, { duration: 300 });\n\
         g.t.onFinished((r) => g.done.push(r.cancelled));",
    )
    .eval("rec('done', [g.done, g.t.running, g.t.duration, g.computes]);")
    .step("nothing_animates");
    s.finish();
}

#[test]
fn transition_failures() {
    let (world, body) = two("new DwindleLayout()");
    let mut s = Scn::new("transition_failures", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval(
        "rec('before', [g.ws.layout.constructor.name, g.b.opacity, gnoblin.clients.style.opacity]);\n\
         rec('failed', fail(() => gnoblin.requestTransition(() => { g.b.opacity = 0.1; g.ws.layout = new g.Half(); gnoblin.clients.style.opacity = 0.2; g.a.opacity = 'dim'; }, { duration: 100 })));\n\
         rec('after', [g.ws.layout.constructor.name, g.b.opacity, gnoblin.clients.style.opacity, g.computes]);",
    )
    .step("failing_callback_leaves_the_previous_state");
    s.eval(
        "rec('failed', fail(() => gnoblin.requestTransition(() => { g.ws.layout = new (class extends Layout { compute() { throw new Error('boom'); } })(); }, { duration: 100 })));\n\
         rec('layout', g.ws.layout.constructor.name);",
    )
    .step("failing_layout");
    s.destroy(4)
        .eval("rec('failed', fail(() => gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.b.opacity = 0.5; }, { duration: 100 }))); rec('opacity', g.a.opacity);")
        .step("stale_target");
    s.finish();

    let (world, body) = two("new Layout()");
    let mut s = Scn::new("transition_misuse", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval(
        "rec('misuse', [\n\
           fail(() => gnoblin.requestTransition(() => { g.a.opacity = 0.5; gnoblin.requestTransition(() => {}, { duration: 1 }); }, { duration: 100 })),\n\
           fail(() => gnoblin.requestTransition(async () => { g.a.opacity = 0.5; }, { duration: 100 })),\n\
           fail(() => gnoblin.requestTransition('nope')),\n\
           fail(() => gnoblin.requestTransition(() => { g.a.raise(); }, { duration: 1 })),\n\
           fail(() => gnoblin.requestTransition(() => { gnoblin.spawn(['true']); }, { duration: 1 })),\n\
         ]); rec('opacity', g.a.opacity);",
    )
    .step("refused");
    s.eval("g.a.opacity = 0.7; g.a.raise();").step("plain_again");
    s.finish();
}

#[test]
fn transition_lifecycle() {
    let (world, body) = two("new Layout()");
    let mut s = Scn::new("transition_ownership", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval(
        "g.log = [];\n\
         g.first = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.a.x = 90; }, { duration: 400 }); g.first.onFinished((r) => g.log.push('first:' + r.cancelled));",
    )
    .step("first");
    s.eval("g.second = gnoblin.requestTransition(() => { g.a.opacity = 0.2; }, { duration: 100 }); g.second.onFinished((r) => g.log.push('second:' + r.cancelled)); rec('first_running', g.first.running);")
        .step("second_owns_opacity");
    s.end_track(3, "opacity", true)
        .eval("rec('log', g.log);")
        .step("second_ended");
    s.end_track(3, "x", true).eval("rec('log', g.log);").step("first_ended");
    s.eval(
        "g.log = [];\n\
         g.third = gnoblin.requestTransition(() => { g.b.opacity = 0.5; }, { duration: 400 }); g.third.onFinished((r) => g.log.push('third:' + r.cancelled));\n\
         g.fourth = gnoblin.requestTransition(() => { g.b.opacity = 0.9; }, { duration: 400 });",
    )
    .eval("rec('log', g.log);")
    .step("third_loses_every_property");
    s.eval("g.b.opacity = 0.3; rec('fourth_cancelled', g.fourth.cancelled);")
        .step("a_plain_write_ends_the_owner");
    s.finish();

    let (world, body) = two("new Layout()");
    let mut s = Scn::new("transition_finished", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval(
        "g.log = [];\n\
         g.t = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.a.x = 90; }, { duration: 500 });\n\
         g.t.finished.then((r) => g.log.push('promise:' + r.cancelled)); g.t.onFinished((r) => g.log.push('callback:' + r.cancelled));\n\
         g.u = gnoblin.requestTransition(() => { g.b.opacity = 0.5; }, { duration: 200 }); g.u.finished.then((r) => g.log.push('u:' + r.cancelled));\n\
         rec('running', g.t.running);",
    )
    .step("two_transitions");
    s.end_track(4, "opacity", true)
        .eval("rec('log', g.log);")
        .step("the_shorter_ends_on_its_own");
    s.eval("g.t.cancel(); g.t.cancel();")
        .eval("rec('log', g.log);")
        .step("cancel_lands_the_end_values");
    s.end_track(3, "opacity", false)
        .end_track(3, "x", false)
        .eval("rec('log', g.log);")
        .step("late_word_changes_nothing");
    s.eval("g.t.onFinished((r) => g.log.push('late:' + r.cancelled));")
        .settle_frames()
        .eval("rec('log', g.log);")
        .step("callback_added_after_the_end");
    s.finish();

    let (world, body) = two("new DwindleLayout()");
    let mut s = Scn::new("transition_set", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval("g.t = g.a.set({ opacity: 0.92, rotation: [0, 0.12, 0] }, { duration: 250, easing: 'ease-out' });")
        .step("set_with_options");
    s.eval("rec('refused', fail(() => g.a.set({ title: 'x' })));")
        .step("set_refuses_a_title");
    s.eval("g.a.set({ opacity: 0.4, z: 3 });")
        .step("set_without_options_is_atomic");
    s.eval("g.ws.set({ layout: new g.Half() }, { duration: 300 }); rec('layout', g.ws.layout instanceof g.Half);")
        .step("set_a_layout");
    s.finish();

    let (world, body) = two("new Layout()");
    let mut s = Scn::new("transition_tween", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval(
        "g.log = []; g.t = gnoblin.requestTransition(() => { g.a.opacity = 0.5; g.a.x = 70; }, { duration: 200, delay: 100 });\n\
         g.t.finished.then((r) => g.log.push('done:' + r.cancelled));",
    )
    .step("tracks_join_a_tween_node");
    s.advance(5000)
        .eval("rec('log', g.log);")
        .step("the_shells_clock_does_not_end_it");
    s.end_track(3, "opacity", true)
        .eval("rec('log', g.log);")
        .step("one_track_ended");
    s.end_track(3, "x", true)
        .eval("rec('log', g.log);")
        .step("both_tracks_ended");
    s.finish();

    let (world, body) = two("new Layout()");
    let mut s = Scn::new("transition_held", Home::Reference, &world, &body);
    s.boot();
    s.eval(HALF).step("ready");
    s.eval(
        "g.x0 = g.a.x; g.log = [];\n\
         g.t = gnoblin.requestTransition(() => { g.a.x = g.x0 + 50; }, { duration: 800, easing: 'ease-in-out', autoplay: false });\n\
         g.t.finished.then((r) => g.log.push('done:' + r.cancelled)); rec('paused', g.t.paused);",
    )
    .step("held");
    for value in ["0.4", "0.1", "1.0"] {
        s.eval(&format!("g.t.progress = {value};"))
            .step(&format!("progress_{value}"));
    }
    s.eval("rec('refused', fail(() => { g.t.progress = 1.5; }));")
        .step("progress_out_of_range");
    s.eval("g.t.reverse();").step("reverse");
    s.end_track(3, "x", false)
        .eval("rec('log', g.log); rec('x', [g.a.x, g.x0]);")
        .step("ran_back");
    s.eval("g.h = gnoblin.requestTransition(() => { g.a.opacity = 0.5; }, { duration: 100, delay: 50, easing: 'linear', autoplay: false }); g.h.onFinished((r) => g.log.push('staged finished cancelled=' + r.cancelled));")
        .step("held_again");
    s.eval("g.h.play();").step("play");
    s.eval("g.h.pause();").step("pause");
    s.eval("g.h.cancel();").eval("rec('log', g.log);").step("cancel");
    s.finish();
}
