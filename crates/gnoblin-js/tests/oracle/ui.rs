//! The UI toolkit (`gnoblin-std/ui/*`, `widgets.ts`): box layout, grid, text metrics, paint encoding, event bubbling, key focus, the entry,
//! the clock, the bar and the toast.

use crate::scn::{At, Home, LEFT, MIDDLE, RIGHT, Scn, World};

fn world() -> World {
    World::desktop(1000.0, 800.0)
}

#[test]
fn layout() {
    let mut s = Scn::new(
        "ui_layout",
        Home::Reference,
        &world(),
        "const box = (o: object) => new Box(o as any);\n\
         const row = box({ width: 300, height: 50, direction: 'row', gap: 10, padding: 5 });\n\
         row.add(box({ width: 40, height: 10 })).add(box({ width: 60, height: 10 }));\n\
         const growRow = box({ width: 300, height: 50, direction: 'row', gap: 10, padding: 5 });\n\
         growRow.add(box({ width: 40 })).add(box({ grow: 1 })).add(box({ grow: 2 }));\n\
         const col = box({ width: 100, height: 200, direction: 'column', gap: 6, padding: 4 });\n\
         col.add(box({ height: 30 })).add(box({ grow: 1 })).add(box({ height: 20 }));\n\
         const none = box({ width: 100, height: 100 });\n\
         const inner = box({ x: 7, y: 9, width: 30, height: 20, direction: 'column', padding: 2 });\n\
         none.add(inner); inner.add(box({ height: 5 }));\n\
         const grid = new Grid({ width: 100, height: 100, padding: 4, gap: 2 });\n\
         [0, 1, 2].forEach(() => grid.add(box({})));\n\
         const tight = box({ width: 50, height: 50, direction: 'row', padding: 5 });\n\
         tight.add(box({ width: 100 })).add(box({ grow: 1 }));\n\
         const stray = box({ width: 80, height: 40, direction: 'row', gap: 3 });\n\
         stray.add(box({ width: 10 })).add(new Canvas({ x: 9, y: 9, width: 4, height: 4 })).add(box({ width: 20 }));\n\
         Object.assign(globalThis, { box, row, growRow, col, none, grid, tight, stray });",
    );
    let roots = [
        ("row", "row.node"),
        ("grow_row", "growRow.node"),
        ("column", "col.node"),
        ("none", "none.node"),
        ("grid", "grid.node"),
        ("tight", "tight.node"),
        ("stray", "stray.node"),
    ];
    s.boot();
    for (label, node) in roots {
        s.tree(label, At::Js(node));
    }
    s.eval("growRow.width = 500; col.height = 120; grid.add(box({})); grid.padding = 0; grid.gap = 6;")
        .step("resized");
    s.eval("row.children[0].visible = false; row.children[1].opacity = 0.5; row.gap = 0; col.children[1].grow = 3;")
        .step("changed");
    s.eval("col.remove(col.children[0]); row.add(growRow.children[0]);")
        .step("moved_and_removed");
    s.eval("row.padding = 12; row.direction = 'column';").step("turned");
    s.eval("growRow.children[1].width = 30; growRow.children[1].grow = 0;")
        .step("grow_off");
    for (label, node) in roots {
        s.tree(&format!("final_{label}"), At::Js(node));
    }
    s.finish();
}

#[test]
fn text_metrics_and_painting() {
    let mut s = Scn::new(
        "ui_text",
        Home::Reference,
        &world(),
        "const holder = new Box({ width: 400, height: 400 });\n\
         const put = (y: number, w: Canvas) => { w.y = y; holder.add(w); return w; };\n\
         const t1 = put(0, new Text({ text: 'Hello', size: 10 }));\n\
         const multi = put(20, new Text({ text: 'a\\nbbbb\\ncc', size: 10 }));\n\
         const roomy = put(80, new Text({ text: 'A long window title', size: 10, ellipsize: true }));\n\
         const cut = put(100, new Text({ text: 'A long window title', size: 10, ellipsize: true, width: 50 }));\n\
         const plain = put(120, new Text({ text: 'A long window title', size: 10, width: 50 }));\n\
         const mono = put(140, new Text({ text: 'ab', size: 13, color: '#c6d0f5', font: 'mono' }));\n\
         const squeezed = put(160, new Text({ text: 'ab', size: 13, height: 13 }));\n\
         const wide = put(180, new Text({ text: 'h\\u00e9llo w\\u00f6rld \\u2713', size: 16 }));\n\
         const empty = put(210, new Text({ text: '', size: 12 }));\n\
         const icon = put(230, new Icon({ icon: 'window-close-symbolic', size: 16, color: '#fff' }));\n\
         const iconDefault = put(260, new Icon({ icon: 'folder' }));\n\
         const texts = [t1, multi, roomy, cut, plain, mono, squeezed, wide, empty];\n\
         const metrics = () => texts.map((t) => [t.width, t.height, t.ascent, t.shown()]);\n\
         Object.assign(globalThis, { holder, t1, metrics, icon, iconDefault });",
    );
    s.boot();
    s.eval("rec('estimate', metrics());").step("estimate");
    s.paint("estimate", At::Js("holder.node"));
    s.answer_measures().eval("rec('measured', metrics());").step("measured");
    s.paint("measured", At::Js("holder.node"));
    s.eval("t1.text = 'Hello, world'; rec('after_change', metrics());")
        .step("text_changed");
    s.answer_measures()
        .eval("rec('answered', metrics());")
        .step("text_answered");
    s.eval(
        "t1.size = 20; t1.font = 'mono'; icon.size = 32; iconDefault.icon = 'user-trash'; rec('restyled', metrics());",
    )
    .step("restyled");
    s.answer_measures()
        .eval("rec('restyled_answered', metrics());")
        .step("restyled_answered");
    s.paint("restyled", At::Js("holder.node"));
    s.finish();
}

#[test]
fn paint_ops_and_colours() {
    let mut s = Scn::new(
        "ui_paint",
        Home::Reference,
        &world(),
        "class Chart extends Canvas {\n\
           draw() {\n\
             return [\n\
               { op: 'rect', x: 1, y: 2, width: 30, height: 20, radius: 4, fill: '#ff000080', stroke: '#00ff00', strokeWidth: 2 },\n\
               { op: 'rect', x: 0, y: 0, width: 5, height: 5 },\n\
               { op: 'line', x1: 0, y1: 0, x2: 30, y2: 20, width: 2, color: '#0000ff' },\n\
               { op: 'line', x1: 1, y1: 1, x2: 2, y2: 2 },\n\
               { op: 'text', text: 'hi \\u2713', x: 3, y: 4, size: 12, color: '#abc', font: 'mono' },\n\
               { op: 'text', text: 'plain' },\n\
               { op: 'icon', name: 'folder', x: 5, y: 6, size: 24, color: '#fff8' },\n\
               { op: 'icon', name: 'user-home' },\n\
               { op: 'pushClip', x: 0, y: 0, width: 10, height: 10, radius: 3 },\n\
               { op: 'pushOpacity', alpha: 0.5 },\n\
               { op: 'rect', x: 0, y: 0, width: 5, height: 5, fill: '#123' },\n\
               { op: 'popOpacity' },\n\
               { op: 'popClip' },\n\
             ];\n\
           }\n\
         }\n\
         class Bad extends Canvas { draw() { return [{ op: 'rect', x: 0, y: 0, width: 4, height: 4, fill: 'red' }] as any; } }\n\
         class Nan extends Canvas { draw() { return [{ op: 'rect', x: NaN, y: 0, width: 4, height: 4 }] as any; } }\n\
         const holder = new Box({ width: 300, height: 200 });\n\
         holder.add(new Chart({ width: 60, height: 40 }));\n\
         const swatch = (x: number, fill: string) => holder.add(new Box({ x, y: 50, width: 8, height: 8, fill }));\n\
         swatch(0, '#f00'); swatch(10, '#f008'); swatch(20, '#ff8800'); swatch(30, '#ff880044');\n\
         holder.add(new Box({ x: 40, y: 50, width: 20, height: 10, radius: 5, fill: '#112233' }));\n\
         holder.add(new Bad({ width: 5, height: 5 }));\n\
         holder.add(new Nan({ width: 5, height: 5 }));\n\
         globalThis.holder = holder;",
    );
    s.boot();
    s.paint("ops", At::Js("holder.node"));
    s.eval("holder.children[0].width = 80; holder.children[5].fill = '#00ff0080';")
        .step("resized");
    s.paint("resized", At::Js("holder.node"));
    s.finish();
}

#[test]
fn events_bubble_and_stop() {
    let mut s = Scn::new(
        "ui_events",
        Home::Reference,
        &world(),
        "const outer = new Box({ x: 10, y: 20, width: 200, height: 100 });\n\
         const mid = new Box({ x: 5, y: 5, width: 100, height: 60 });\n\
         const leaf = new Box({ x: 7, y: 3, width: 20, height: 20 });\n\
         outer.add(mid); mid.add(leaf);\n\
         globalThis.stopAt = '';\n\
         for (const [name, c] of [['outer', outer], ['mid', mid], ['leaf', leaf]] as [string, any][]) {\n\
           for (const h of ['onPress', 'onRelease', 'onMotion', 'onScroll', 'onEnter', 'onLeave']) {\n\
             c[h] = (e: any) => { rec(name + '.' + h, { x: e.x, y: e.y, button: e.button, kind: e.kind, target: e.target === leaf }); if (name === 'mid' && (globalThis as any).stopAt === h) e.stop(); };\n\
           }\n\
         }\n\
         Object.assign(globalThis, { outer, mid, leaf });",
    );
    s.boot();
    for (kind, button) in [
        ("press", LEFT),
        ("release", LEFT),
        ("motion", 0),
        ("scroll", 0),
        ("enter", 0),
        ("leave", 0),
        ("press", RIGHT),
        ("press", MIDDLE),
    ] {
        s.pointer(At::Js("leaf.node"), kind, 4.0, 5.0, button)
            .step(&format!("leaf_{kind}_{button}"));
    }
    s.eval("globalThis.stopAt = 'onPress';")
        .pointer(At::Js("leaf.node"), "press", 4.0, 5.0, LEFT)
        .step("stopped_at_mid");
    s.eval("globalThis.stopAt = ''; leaf.onRelease = () => { throw new Error('a handler that throws'); };")
        .pointer(At::Js("leaf.node"), "release", 1.0, 1.0, LEFT)
        .step("handler_throws");
    s.pointer(At::Js("mid.node"), "press", 50.0, 30.0, LEFT)
        .step("mid_press");
    s.eval("mid.destroy();")
        .pointer(At::Js("leaf.node"), "press", 1.0, 1.0, LEFT)
        .step("after_destroy");
    s.finish();
}

#[test]
fn keys_and_focus() {
    let mut s = Scn::new(
        "ui_keys",
        Home::Reference,
        &world(),
        "const a = new Box({ width: 100, height: 50 });\n\
         const b = new Box({ width: 50, height: 20 });\n\
         const c = new Box({ width: 50, height: 20, x: 50 });\n\
         a.add(b); a.add(c);\n\
         a.onKey = (e: any) => rec('a.key', { key: e.key, text: e.text, pressed: e.pressed });\n\
         b.onKey = (e: any) => { rec('b.key', { key: e.key, text: e.text, pressed: e.pressed }); if (e.key === 'x') e.stop(); };\n\
         a.mountTop(); b.focus();\n\
         Object.assign(globalThis, { a, b, c });",
    );
    s.boot();
    s.eval("rec('focus', { a: a.focused, b: b.focused, c: c.focused });")
        .step("focus_on_b");
    s.tap(At::Js("b.node"), "y", "y").step("key_bubbles");
    s.tap(At::Js("b.node"), "x", "x").step("key_stopped");
    s.key(At::Js("b.node"), "Return", "", true).step("return_pressed");
    s.eval("c.focus(); rec('focus', { a: a.focused, b: b.focused, c: c.focused });")
        .step("focus_on_c");
    s.tap(At::Js("c.node"), "z", "z").step("key_to_c");
    s.eval("c.destroy(); rec('focus', { a: a.focused, b: b.focused });")
        .step("c_destroyed");
    s.eval("b.focus(); b.destroy(); rec('focus', { a: a.focused, b: b.focused });")
        .step("b_destroyed_while_focused");
    s.finish();
}

#[test]
fn entry_states() {
    let mut s = Scn::new(
        "ui_entry",
        Home::Reference,
        &world(),
        "const e = new Entry({ width: 300, height: 40 });\n\
         e.mountTop(); e.focus();\n\
         e.onSubmit = (text: string) => rec('submit', text);\n\
         const idle = new Entry({ width: 200, height: 30, text: 'caf\\u00e9 \\u2713', size: 12, color: '#ffffff' });\n\
         idle.mountTop();\n\
         Object.assign(globalThis, { e, idle });",
    );
    let e = At::Js("e.node");
    s.boot();
    s.paint("empty", e);
    s.answer_measures().step("measured_empty");
    for (key, text) in [("f", "f"), ("o", "o"), ("o", "o"), ("BackSpace", ""), ("x", "x")] {
        s.tap(e, key, text);
    }
    s.answer_measures()
        .eval("rec('text', [e.text, e.model.caret, e.model.selectedText]);")
        .step("typed");
    s.paint("typed", e);
    s.key(e, "Left", "", true)
        .key(e, "Left", "", true)
        .answer_measures()
        .step("moved_left");
    s.paint("caret_moved", e);
    s.key(e, "Shift_L", "", true)
        .key(e, "Left", "", true)
        .key(e, "Shift_L", "", false);
    s.answer_measures()
        .eval("rec('selection', [e.model.caret, e.model.anchor, e.model.selectedText]);")
        .step("selected");
    s.paint("selected", e);
    s.key(e, "Control_L", "", true)
        .key(e, "a", "a", true)
        .key(e, "Control_L", "", false);
    s.answer_measures()
        .eval("rec('selection', [e.model.caret, e.model.anchor, e.model.selectedText]);")
        .step("select_all");
    s.paint("select_all", e);
    s.key(e, "End", "", true)
        .key(e, "Home", "", true)
        .key(e, "Delete", "", true)
        .answer_measures()
        .step("home_delete");
    s.paint("home_delete", e);
    s.key(e, "Return", "", true).step("submit");
    s.eval("e.secret = true;").answer_measures().step("secret");
    s.paint("secret", e);
    s.eval("e.selectionFill = '#ff0000'; e.color = '#00ff00'; e.size = 20; e.font = 'mono'; e.model.selectAll();")
        .answer_measures()
        .step("restyled");
    s.paint("restyled", e);
    s.eval("e.text = 'h\\u00e9llo';").answer_measures().step("unicode");
    s.paint("unicode", e);
    s.eval("e.focus();").step("refocus");
    s.paint("idle_unfocused", At::Js("idle.node"));
    s.eval("idle.focus();").answer_measures().step("idle_focused");
    s.paint("idle_focused", At::Js("idle.node"));
    s.finish();
}

#[test]
fn entry_model() {
    let mut s = Scn::new(
        "ui_entry_model",
        Home::Reference,
        &world(),
        "const m = new EntryModel();\n\
         const at = () => [m.text, m.caret, m.anchor, m.selectedText, m.length];\n\
         Object.assign(globalThis, { m, at });",
    );
    s.boot();
    let script = [
        "m.insert('h\\u00e9llo')",
        "m.backspace()",
        "m.left(); m.left(); m.insert('X\\n')",
        "m.home(true)",
        "m.insert('Z')",
        "m.selectAll()",
        "m.delete()",
        "m.insert('ab'); m.left(); m.delete()",
        "m.end(); m.right(); m.left(true)",
        "m.text = 'new'",
        "m.home(); m.right(true); m.right(true); m.backspace()",
        "m.insert('\\ud83d\\ude00x'); m.left(); m.left(); m.delete()",
        "m.end(true); m.insert('')",
    ];
    for (n, code) in script.iter().enumerate() {
        s.eval(&format!("{code}; rec('state', at());"))
            .step(&format!("edit_{}", n + 1));
    }
    s.finish();
}

#[test]
fn clock_bar_toast() {
    let mut s = Scn::new(
        "ui_widgets",
        Home::Reference,
        &world(),
        "globalThis.t = new Date(2024, 0, 5, 9, 30, 7);\n\
         const clock = new Clock({ now: () => (globalThis as any).t, size: 18, color: '#ff0000' });\n\
         const seconds = new Clock({ now: () => (globalThis as any).t, format: '%a %d %b %Y %I:%M:%S %p' });\n\
         const holder = new Box({ width: 400, height: 60, direction: 'row', gap: 8, padding: 4 });\n\
         holder.add(clock).add(seconds);\n\
         holder.mountTop();\n\
         Object.assign(globalThis, { clock, seconds, holder });",
    );
    s.boot();
    s.answer_measures()
        .eval("rec('clocks', [clock.text, seconds.text]);")
        .step("clocks");
    s.paint("clocks", At::Js("holder.node"));
    s.eval("globalThis.t = new Date(2024, 11, 31, 23, 59, 59);")
        .answer_measures()
        .eval("rec('clocks', [clock.text, seconds.text]);")
        .step("clocks_later");
    s.paint("clocks_later", At::Js("holder.node"));
    s.eval(
        "const d = new Date(2024, 0, 5, 14, 7, 9);\n\
         rec('formats', ['%H:%M', '%I:%M %p', '%a %d %b %Y', '%T %%', '%Q', '%F %j', '%A %B %e %y %R', '%S %m'].map((f) => formatTime(f, d)));",
    )
    .step("formats");
    s.eval(
        "globalThis.top = new Bar({ height: 32 }); globalThis.low = new Bar({ edge: 'bottom', height: 20 });\n\
         globalThis.second = new Bar({ height: 10, fill: '#ff000080' });\n\
         top.add(new Text({ text: 'left', grow: 1 })); top.add(new Text({ text: 'right' }));",
    )
    .answer_measures()
    .eval("rec('insets', barInsets()); rec('area', gnoblin.workspaces.area); rec('bars', [top, second, low].map((b) => [b.x, b.y, b.width, b.height, b.visible]));")
    .step("bars");
    s.paint("bars", At::Js("top.node"));
    s.eval("top.hideWhen = () => true;").eval("rec('insets', barInsets()); rec('bars', [top, second, low].map((b) => [b.x, b.y, b.width, b.height, b.visible]));")
        .step("top_hidden");
    s.eval("second.destroy(); top.destroy(); low.destroy(); rec('insets', barInsets());")
        .step("bars_gone");
    s.eval("globalThis.now = 0; globalThis.card = toast('saved', { fadeIn: 100, hold: 200, fadeOut: 100, now: () => globalThis.now });")
        .answer_measures()
        .eval("rec('toast', [card.x, card.y, card.width, card.height, card.opacity]);")
        .step("toast_start");
    s.paint("toast_start", At::Js("card.node"));
    for (at, label) in [(50, "fading_in"), (150, "full"), (350, "fading_out"), (500, "gone")] {
        s.eval(&format!("globalThis.now = {at};"))
            .settle_frames()
            .eval("rec('toast', [card.alive, card.opacity, uiCanvasCount()]);")
            .step(label);
    }
    s.finish();
}
