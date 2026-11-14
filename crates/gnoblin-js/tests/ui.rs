//! The shell UI toolkit (`data/js/gnoblin/ui.ts`, `titlebar.ts`) over a fake compositor.
//!
//! `FakeCompositor` is an in-memory `TreeLink` that behaves as far as the toolkit needs: it applies `New`,
//! `Insert`, `Set` and `Destroy` the way the compositor does and echoes each as a change, answers a transaction with
//! a reply, answers a `measure` call when a test says how, and destroys a window's canvases with the window. What it
//! cannot show (the compositor decoding the paint list and drawing it) is `crates/gnoblin/tests/js_shell.rs`'s.
//!
//! The Box layout cases are the Lua shell's arithmetic (`Box:arrange` in `data/lua/gnoblin/shell/ui.lua`), worked by
//! hand: the Lua corpus (`crates/gnoblin-lua/tests/lua/ui_test.lua`) tests the paint and the measure but has no
//! layout case, so each expected rectangle below is derived from that function. The paint cases are the Lua
//! test's byte for byte (`string.pack` in that file is `pack` here) and one list of every op is compared with
//! `gnoblin_paint::encode`, which the compositor's own decoder reads.

use std::fs;
use std::path::Path;

use gnoblin_js::JsShell;
use gnoblin_paint::{PaintList, PaintOp, Rgba8, decode, encode};
mod common;
use common::*;

/// What every init file imports.
const IMPORTS: &str = "import { Box, Canvas, Grid, Icon, Text, Titlebar, defaultTitlebarStyle, defineConfig, \
                       uiCanvasCount } from \"gnoblin\";\n\
                       (globalThis as any).uiCanvasCount = uiCanvasCount;\n";

fn rig(init_body: &str) -> Rig {
    rig_with(IMPORTS, init_body)
}

// ---- Box ----------------------------------------------------------------------------------------------------------

#[test]
fn a_row_places_its_children_with_padding_and_gap_and_stretches_them_across() {
    // Derived from `Box:arrange`: the children start at the padding, a gap apart, and are as tall as the row's inside.
    let mut rig = rig(
        "const root = new Box({ width: 300, height: 50, direction: 'row', gap: 10, padding: 5 });\n\
         const a = new Box({ width: 40, height: 10 });\n\
         const b = new Box({ width: 60, height: 10 });\n\
         root.add(a).add(b);\n\
         (globalThis as any).ids = [root, a, b].map((box) => box.node);",
    );
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    assert_eq!(rig.settle(), Vec::<String>::new());
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["0,0,300,50", "5,5,40,40", "55,5,60,40"]);
    assert_eq!(
        rig.comp.children(ids[0]),
        [ids[1], ids[2]],
        "the children are children of the node"
    );
}

#[test]
fn a_row_shares_what_is_left_among_the_children_that_grow() {
    // 300 wide, padding 5 and gap 10 around three children: 300 - 10 - 20 - 40 = 230 left for the two that grow, in
    // the ratio 1 to 2: 230 * 1 // 3 = 76 and 230 * 2 // 3 = 153. The pixel the floors lose stays unused.
    let mut rig = rig(
        "const root = new Box({ width: 300, height: 50, direction: 'row', gap: 10, padding: 5 });\n\
         const a = new Box({ width: 40 });\n\
         const b = new Box({ grow: 1 });\n\
         const c = new Box({ grow: 2 });\n\
         root.add(a).add(b).add(c);\n\
         (globalThis as any).ids = [a, b, c].map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["5,5,40,40", "55,5,76,40", "141,5,153,40"]);
}

#[test]
fn a_column_stacks_its_children_and_stretches_them_across() {
    // 200 high, padding 4, gap 6: 200 - 8 - 12 - 50 = 130 for the one that grows; every child is 100 - 8 wide.
    let mut rig = rig(
        "const root = new Box({ width: 100, height: 200, direction: 'column', gap: 6, padding: 4 });\n\
         const a = new Box({ height: 30 });\n\
         const b = new Box({ grow: 1 });\n\
         const c = new Box({ height: 20 });\n\
         root.add(a).add(b).add(c);\n\
         (globalThis as any).ids = [a, b, c].map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["4,4,92,30", "4,40,92,130", "4,176,92,20"]);
}

#[test]
fn a_box_with_no_direction_leaves_its_children_where_they_are_and_nests() {
    let mut rig = rig("const root = new Box({ width: 100, height: 100 });\n\
         const inner = new Box({ x: 7, y: 9, width: 30, height: 20, direction: 'column', padding: 2 });\n\
         const leaf = new Box({ height: 5 });\n\
         root.add(inner);\n\
         inner.add(leaf);\n\
         (globalThis as any).ids = [inner, leaf].map((box) => box.node);");
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["7,9,30,20", "2,2,26,5"]);
}

#[test]
fn a_grid_puts_its_children_in_a_near_square_of_equal_cells() {
    // Three children: 2 columns and 2 rows; (100 - 8 - 2) // 2 = 45 each, 47 apart from the padding.
    let mut rig = rig(
        "const grid = new Grid({ width: 100, height: 100, padding: 4, gap: 2 });\n\
         const cells = [0, 1, 2].map(() => new Box());\n\
         cells.forEach((cell) => grid.add(cell));\n\
         (globalThis as any).ids = cells.map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let rects: Vec<String> = ids.iter().map(|id| rig.comp.rect(*id)).collect();
    assert_eq!(rects, ["4,4,45,45", "51,4,45,45", "4,51,45,45"]);
}

#[test]
fn nothing_is_written_again_for_a_frame_that_changed_nothing() {
    let mut rig = rig("const root = new Box({ width: 10, height: 5, fill: '#ff0000' });\n\
         (globalThis as any).root = root;");
    rig.settle();
    let sent = rig.comp.sent_count();
    rig.settle();
    assert_eq!(rig.comp.sent_count(), sent, "an unchanged tree sends no transaction");
    rig.eval("root.width = 20; root.x = 3; 1");
    rig.settle();
    assert_eq!(
        rig.comp.sent_count(),
        sent + 1,
        "a changed field is one more transaction"
    );
}

// ---- Text and the measure call ------------------------------------------------------------------------------------

#[test]
fn a_text_asks_the_compositor_to_measure_and_takes_the_answer() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'Hello', size: 10 });");
    // Until the answer the metrics are the estimate: half the size a glyph, 3/2 a line, 11/10 to the baseline.
    assert_eq!(rig.eval("[t.width, t.height, t.ascent].join()"), "25,15,11");
    rig.settle();
    let node = rig.ids("[t.node]")[0];
    let asked = rig.comp.measures();
    assert_eq!(
        asked,
        [Measure {
            token: asked[0].token,
            node,
            size: 10.0,
            text: "sans\nHello".to_owned(),
        }],
        "one call, on the text's own node, with the size as a double and `font\\ntext`"
    );
    rig.comp.answer(asked[0].token, "31 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("[t.width, t.height, t.ascent].join()"), "31,15,12");
    assert_eq!(rig.comp.measures().len(), 1, "an answered text is not asked again");
}

#[test]
fn changing_the_text_falls_back_to_the_estimate_asks_again_and_ignores_a_late_answer() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'Hello', size: 10 });");
    rig.settle();
    let first = rig.comp.measures()[0].token;
    rig.comp.answer(first, "31 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("t.width"), "31");
    rig.eval("t.text = 'Hello, world'; 1");
    assert_eq!(
        rig.eval("t.width"),
        "60",
        "while the new answer is out the width is the estimate"
    );
    rig.settle();
    let asked = rig.comp.measures();
    assert_eq!(asked.len(), 2, "new text asks again");
    assert_eq!(asked[1].text, "sans\nHello, world");
    // The first question's answer arrives late: it is for text the node no longer has.
    rig.comp.answer(first, "99 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("t.width"), "60");
    rig.comp.answer(asked[1].token, "74 15 12", "");
    rig.settle();
    assert_eq!(rig.eval("t.width"), "74");
}

#[test]
fn a_refused_measure_keeps_the_estimate_is_reported_and_is_not_asked_again() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'Hi', size: 10 });");
    rig.settle();
    let token = rig.comp.measures()[0].token;
    rig.comp.answer(token, "", "unavailable");
    let diagnostics = rig.settle();
    assert!(
        diagnostics
            .iter()
            .any(|line| line.contains("measure failed: unavailable")),
        "{diagnostics:?}"
    );
    assert_eq!(rig.eval("t.width"), "10", "the estimate stays");
    rig.settle();
    assert_eq!(
        rig.comp.measures().len(),
        1,
        "a refusal is not asked again for the same text"
    );
}

#[test]
fn a_multi_line_text_is_as_tall_as_its_lines_and_as_wide_as_its_widest() {
    let mut rig = rig("(globalThis as any).t = new Text({ text: 'a\\nbbbb\\ncc', size: 10 });");
    assert_eq!(rig.eval("[t.width, t.height].join()"), "20,45");
}

#[test]
fn a_text_that_ellipsizes_is_cut_to_the_room_its_node_has() {
    // No answer: the estimate is 19 glyphs at 5 pixels, 95 wide; 50 pixels keep 19 * 50 // 95 - 1 = 9 glyphs.
    let mut rig = rig(
        "const roomy = new Text({ text: 'A long window title', size: 10, ellipsize: true });\n\
         const cut = new Text({ text: 'A long window title', size: 10, ellipsize: true, width: 50 });\n\
         const plain = new Text({ text: 'A long window title', size: 10, width: 50 });\n\
         (globalThis as any).ids = [roomy, cut, plain].map((t) => t.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let shown = |id: u64| {
        let list = decode(&rig.comp.content(id).expect("a paint list")).expect("it decodes");
        match &list.ops[..] {
            [PaintOp::Text { text, .. }] => text.clone(),
            other => panic!("one text op, got {other:?}"),
        }
    };
    assert_eq!(
        shown(ids[0]),
        "A long window title",
        "with room enough the whole text is painted"
    );
    assert_eq!(
        shown(ids[1]),
        "A long wi\u{2026}",
        "a text over its room ends in an ellipsis"
    );
    assert_eq!(
        shown(ids[2]),
        "A long window title",
        "a text that does not ellipsize paints all of it"
    );
}

// ---- the paint list -----------------------------------------------------------------------------------------------

fn f32le(value: f32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn u32le(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

/// `string.pack("<s2", s)`.
fn str2(value: &str) -> Vec<u8> {
    let mut out = u16::try_from(value.len()).expect("short").to_le_bytes().to_vec();
    out.extend(value.as_bytes());
    out
}

/// `string.pack("<s4", s)`.
fn str4(value: &str) -> Vec<u8> {
    let mut out = u32le(u32::try_from(value.len()).expect("short"));
    out.extend(value.as_bytes());
    out
}

fn list(ops: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = b"GNPL".to_vec();
    out.push(1);
    out.extend(u32le(ops.len() as u32));
    out.extend(ops.into_iter().flatten());
    out
}

#[test]
fn a_box_paints_its_fill_and_paints_again_when_the_fill_changes() {
    // `a Box paints its fill into its content` of the Lua tests, byte for byte.
    let mut rig = rig(
        "const b = new Box({ width: 10, height: 5, fill: '#ff0000', radius: 2 });\n\
         (globalThis as any).b = b;\n\
         (globalThis as any).ids = [b.node];",
    );
    rig.settle();
    let id = rig.ids("ids")[0];
    let rect = |fill: u32| {
        let mut op = vec![1_u8];
        for value in [0.0, 0.0, 10.0, 5.0, 2.0] {
            op.extend(f32le(value));
        }
        op.extend(u32le(fill));
        op.extend(u32le(0));
        op.extend(f32le(0.0));
        list(vec![op])
    };
    assert_eq!(rig.comp.content(id), Some(rect(0xff00_00ff)));
    rig.eval("b.fill = '#00ff0080'; 1");
    rig.settle();
    assert_eq!(
        rig.comp.content(id),
        Some(rect(0x00ff_0080)),
        "a new fill is painted again"
    );
}

#[test]
fn a_text_and_an_icon_paint_themselves_with_their_colours() {
    let mut rig = rig(
        "const t = new Text({ text: 'ab', size: 13, color: '#c6d0f5', font: 'mono' });\n\
         const i = new Icon({ icon: 'window-close-symbolic', size: 16, color: '#fff' });\n\
         (globalThis as any).ids = [t, i].map((box) => box.node);",
    );
    rig.settle();
    let ids = rig.ids("ids");
    let mut text_op = vec![3_u8];
    for value in [0.0, 0.0, 13.0] {
        text_op.extend(f32le(value));
    }
    text_op.extend(u32le(0xc6d0_f5ff));
    text_op.extend(str2("mono"));
    text_op.extend(str4("ab"));
    let mut icon_op = vec![4_u8];
    for value in [0.0, 0.0, 16.0] {
        icon_op.extend(f32le(value));
    }
    icon_op.extend(u32le(0xffff_ffff));
    icon_op.extend(str2("window-close-symbolic"));
    assert_eq!(rig.comp.content(ids[0]), Some(list(vec![text_op])));
    assert_eq!(rig.comp.content(ids[1]), Some(list(vec![icon_op])));
    assert_eq!(rig.comp.rect(ids[1]), "0,0,16,16", "an icon is as big as its size");
}

#[test]
fn a_text_squeezed_shorter_than_its_line_is_centred_in_its_node() {
    // The estimate of a 13 pixel line is 19 high; a node 13 high trims 3 from each end: y is -3.
    let mut rig = rig(
        "const t = new Text({ text: 'ab', size: 13, color: '#fff', height: 13 });\n\
         (globalThis as any).ids = [t.node];",
    );
    rig.settle();
    let id = rig.ids("ids")[0];
    let mut op = vec![3_u8];
    for value in [0.0, -3.0, 13.0] {
        op.extend(f32le(value));
    }
    op.extend(u32le(0xffff_ffff));
    op.extend(str2("sans"));
    op.extend(str4("ab"));
    assert_eq!(rig.comp.content(id), Some(list(vec![op])));
}

#[test]
fn a_canvas_paints_the_ops_its_draw_returns_in_the_encoding_the_compositor_decodes() {
    let mut rig = rig("class Chart extends Canvas {\n\
             draw() {\n\
                 return [\n\
                     { op: 'rect', x: 1, y: 2, width: 30, height: 40, radius: 3, fill: '#102030', stroke: '#f0a8', strokeWidth: 1.5 },\n\
                     { op: 'line', x1: 0, y1: 0, x2: 10, y2: 20, width: 2, color: '#ffffff80' },\n\
                     { op: 'text', text: 'h\\u00e9llo', x: 4, y: 5, size: 13, color: '#c6d0f5', font: 'mono' },\n\
                     { op: 'icon', name: 'window-close-symbolic', x: 1, y: 1, size: 16, color: '#fff' },\n\
                     { op: 'pushClip', x: 0, y: 0, width: 8, height: 8, radius: 2 },\n\
                     { op: 'popClip' },\n\
                     { op: 'pushOpacity', alpha: 0.5 },\n\
                     { op: 'popOpacity' },\n\
                 ];\n\
             }\n\
         }\n\
         const chart = new Chart({ width: 40, height: 40 });\n\
         (globalThis as any).ids = [chart.node];");
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    rig.settle();
    let id = rig.ids("ids")[0];
    let expected = PaintList {
        ops: vec![
            PaintOp::Rect {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 40.0,
                radius: 3.0,
                fill: Rgba8::new(0x10, 0x20, 0x30, 0xff),
                stroke: Rgba8::new(0xff, 0x00, 0xaa, 0x88),
                stroke_width: 1.5,
            },
            PaintOp::TextLine {
                x1: 0.0,
                y1: 0.0,
                x2: 10.0,
                y2: 20.0,
                width: 2.0,
                colour: Rgba8::new(0xff, 0xff, 0xff, 0x80),
            },
            PaintOp::Text {
                x: 4.0,
                y: 5.0,
                size: 13.0,
                colour: Rgba8::new(0xc6, 0xd0, 0xf5, 0xff),
                font: "mono".to_owned(),
                text: "h\u{e9}llo".to_owned(),
            },
            PaintOp::Icon {
                x: 1.0,
                y: 1.0,
                size: 16.0,
                colour: Rgba8::new(0xff, 0xff, 0xff, 0xff),
                name: "window-close-symbolic".to_owned(),
            },
            PaintOp::PushClip {
                x: 0.0,
                y: 0.0,
                width: 8.0,
                height: 8.0,
                radius: 2.0,
            },
            PaintOp::PopClip,
            PaintOp::PushOpacity(0.5),
            PaintOp::PopOpacity,
        ],
    };
    let content = rig.comp.content(id).expect("a paint list");
    assert_eq!(
        content,
        encode(&expected),
        "the same bytes the compositor's encoder writes"
    );
    assert_eq!(decode(&content).expect("it decodes"), expected);
    assert_eq!(rig.comp.rect(id), "0,0,40,40");
}

#[test]
fn an_op_that_cannot_be_encoded_is_reported_and_leaves_the_content_alone() {
    let mut rig = rig("class Bad extends Canvas {\n\
             fixed = false;\n\
             draw() {\n\
                 return [{ op: 'rect', x: 0, y: 0, width: 4, height: 4, fill: this.fixed ? '#000' : 'red' }];\n\
             }\n\
         }\n\
         const bad = new Bad({ width: 4, height: 4 });\n\
         (globalThis as any).bad = bad;\n\
         (globalThis as any).ids = [bad.node];");
    let diagnostics = rig.settle();
    let id = rig.ids("ids")[0];
    assert!(
        diagnostics.iter().any(|line| line.contains("draw op 1")),
        "the bad op is named: {diagnostics:?}"
    );
    assert_eq!(rig.comp.content(id), None, "nothing is written for a bad op");
    rig.eval("bad.fixed = true; 1");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert!(
        rig.comp.content(id).is_some(),
        "a good list is written once it is fixed"
    );
}

// ---- the titlebar -------------------------------------------------------------------------------------------------

/// A client the titlebar can decorate, kept in the script.
const STUB_CLIENT: &str = "const client = { title: 'My title', focused: true, fullscreen: false, width: 400, x: 10, y: 20, \
                           maximised: false, minimised: false, closed: 0, close() { this.closed += 1; } };\n\
                           (globalThis as any).client = client;\n";

fn texts_of(list: &PaintList) -> Vec<String> {
    list.ops
        .iter()
        .filter_map(|op| match op {
            PaintOp::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_titlebar_is_a_box_of_a_drag_area_with_the_title_and_the_three_buttons() {
    let mut rig = rig(&format!(
        "{STUB_CLIENT}const bar = new Titlebar(client);\n\
         bar.mount({ROOT});\n\
         (globalThis as any).bar = bar;\n\
         (globalThis as any).ids = [bar, bar.dragArea, bar.title, ...bar.buttons.values()].map((c) => c.node);"
    ));
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    rig.settle();
    let ids = rig.ids("ids");
    let (bar, drag, title, minimise, maximise, close) = (ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]);
    assert_eq!(
        rig.comp.children(ROOT),
        [bar],
        "the bar hangs under the node it was mounted on"
    );
    assert_eq!(rig.comp.children(bar), [drag, minimise, maximise, close]);
    assert_eq!(rig.comp.children(drag), [title]);
    // The bar lies above the window's content: its y is minus its height.
    // 400 wide, padding 6: the three 16 pixel buttons and the gaps of 8 leave 316 for the drag area.
    assert_eq!(rig.comp.rect(bar), "0,-28,400,28");
    assert_eq!(rig.comp.rect(drag), "6,6,316,16");
    assert_eq!(rig.comp.rect(title), "0,0,316,16");
    assert_eq!(
        [rig.comp.rect(minimise), rig.comp.rect(maximise), rig.comp.rect(close)],
        ["330,6,16,16", "354,6,16,16", "378,6,16,16"]
    );
    let title_list = decode(&rig.comp.content(title).expect("the title is painted")).expect("it decodes");
    assert_eq!(texts_of(&title_list), ["My title"]);
    let bar_list = decode(&rig.comp.content(bar).expect("the bar is painted")).expect("it decodes");
    assert!(
        matches!(&bar_list.ops[..], [PaintOp::Rect { fill, .. }] if *fill == Rgba8::new(0x41, 0x45, 0x59, 0xff)),
        "a focused bar is filled with the focused colour: {bar_list:?}"
    );
    let close_list = decode(&rig.comp.content(close).expect("the button is painted")).expect("it decodes");
    assert!(matches!(&close_list.ops[..], [PaintOp::Icon { name, .. }] if name == "window-close-symbolic"));
}

#[test]
fn a_titlebar_follows_its_client_and_takes_its_look_from_plain_fields() {
    let mut rig = rig(&format!(
        "{STUB_CLIENT}const style = {{ ...defaultTitlebarStyle, height: 40, buttons: ['close'], focusedFill: '#112233', \
         unfocusedFill: '#445566' }};\n\
         const bar = new Titlebar(client, style);\n\
         bar.mount({ROOT});\n\
         (globalThis as any).ids = [bar, bar.title].map((c) => c.node);"
    ));
    rig.settle();
    let ids = rig.ids("ids");
    let (bar, title) = (ids[0], ids[1]);
    assert_eq!(rig.comp.rect(bar), "0,-40,400,40");
    assert_eq!(
        rig.comp.children(bar).len(),
        2,
        "the drag area and the one button the style names"
    );
    let fill = |rig: &Rig| match &decode(&rig.comp.content(bar).expect("painted")).expect("decodes").ops[..] {
        [PaintOp::Rect { fill, .. }] => *fill,
        other => panic!("one rect, got {other:?}"),
    };
    assert_eq!(fill(&rig), Rgba8::new(0x11, 0x22, 0x33, 0xff));
    rig.eval("client.focused = false; client.width = 600; client.title = 'Renamed'; 1");
    rig.settle();
    assert_eq!(
        fill(&rig),
        Rgba8::new(0x44, 0x55, 0x66, 0xff),
        "the focus changed the fill"
    );
    assert_eq!(rig.comp.rect(bar), "0,-40,600,40", "the bar is as wide as its client");
    let title_list = decode(&rig.comp.content(title).expect("painted")).expect("decodes");
    assert_eq!(texts_of(&title_list), ["Renamed"]);
    rig.eval("client.fullscreen = true; 1");
    rig.settle();
    assert_eq!(
        rig.comp.flag(bar, "visible"),
        Some(false),
        "a fullscreen window has no titlebar"
    );
}

#[test]
fn the_buttons_and_the_bar_answer_a_press() {
    let mut rig = rig(&format!(
        "{STUB_CLIENT}const bar = new Titlebar(client);\n\
         bar.mount({ROOT});\n\
         (globalThis as any).clock = 0;\n\
         bar.now = () => (globalThis as any).clock;\n\
         (globalThis as any).ids = [bar, bar.title, bar.buttons.get('minimise'), bar.buttons.get('maximise'), \
         bar.buttons.get('close')].map((c) => c.node);"
    ));
    rig.settle();
    let ids = rig.ids("ids");
    let (title, minimise, maximise, close) = (ids[1], ids[2], ids[3], ids[4]);
    let state =
        |rig: &mut Rig| rig.eval("[client.closed, client.maximised, client.minimised, client.x, client.y].join()");

    rig.comp.pointer(close, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,false,false,10,20",
        "the close button closes the client"
    );

    rig.comp.pointer(minimise, "press", 5.0, 5.0, LEFT);
    rig.comp.pointer(maximise, "press", 5.0, 5.0, LEFT);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,true,true,10,20",
        "minimise and maximise write the client's state"
    );

    // A press on the title starts a drag: it moves the client by how far the pointer goes in the bar's coordinates.
    // The title sits 6 pixels in and down in the bar, and the event carries the title's own coordinates.
    rig.comp.pointer(title, "press", 3.0, 3.0, LEFT);
    rig.comp.pointer(title, "motion", 13.0, 5.0, 0);
    rig.settle();
    assert_eq!(state(&mut rig), "1,true,true,20,22");
    rig.comp.pointer(title, "release", 13.0, 5.0, LEFT);
    rig.comp.pointer(title, "motion", 50.0, 50.0, 0);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,true,true,20,22",
        "a released drag does not move the client"
    );

    // A second press on the bar within half a second toggles the maximised state instead of starting a drag.
    rig.eval("clock = 1000; 1");
    rig.comp.pointer(title, "press", 3.0, 3.0, LEFT);
    rig.comp.pointer(title, "release", 3.0, 3.0, LEFT);
    rig.eval("clock = 1200; 1");
    rig.comp.pointer(title, "press", 3.0, 3.0, LEFT);
    rig.settle();
    assert_eq!(
        state(&mut rig),
        "1,false,true,20,22",
        "a double click restores the maximised window"
    );
}

// ---- decorations --------------------------------------------------------------------------------------------------

#[test]
fn a_window_gets_one_titlebar_and_loses_it_when_the_window_closes() {
    let mut rig = rig("defineConfig({ decorations: true });");
    assert!(rig.load_diagnostics.is_empty(), "{:?}", rig.load_diagnostics);
    rig.settle();
    assert!(rig.comp.canvases().is_empty(), "no window, no titlebar");

    rig.comp.create_window(3, "term", "foot");
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    let bars = rig.comp.children(3);
    assert_eq!(bars.len(), 1, "exactly one titlebar node under the window");
    assert_eq!(
        rig.comp.canvases().len(),
        6,
        "the bar, its drag area, its title and its three buttons"
    );
    assert_eq!(
        rig.comp.rect(bars[0]),
        "0,-28,400,28",
        "the bar is as wide as the window"
    );
    assert_eq!(rig.eval("uiCanvasCount()"), "6");

    rig.settle();
    assert_eq!(rig.comp.children(3).len(), 1, "later frames do not add another");

    let title = rig.comp.children(rig.comp.children(bars[0])[0])[0];
    rig.comp.external_set(3, "title", text("renamed"));
    rig.settle();
    let list = decode(&rig.comp.content(title).expect("painted")).expect("decodes");
    assert_eq!(texts_of(&list), ["renamed"], "the title follows the window's");

    rig.comp.destroy(3);
    let diagnostics = rig.settle();
    assert_eq!(diagnostics, Vec::<String>::new());
    assert!(
        rig.comp.canvases().is_empty(),
        "the compositor took the titlebar's nodes with the window"
    );
    assert_eq!(rig.eval("uiCanvasCount()"), "0", "and the toolkit forgot them");
    assert_eq!(
        rig.comp.destroys_of(bars[0]),
        0,
        "the shell does not destroy what is already gone"
    );
}

#[test]
fn every_window_has_a_titlebar_of_its_own_including_the_ones_there_at_boot() {
    let comp = FakeCompositor::new();
    comp.add_window(3, "one", "a");
    comp.add_window(4, "two", "b");
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    fs::write(
        root.join("init.ts"),
        format!("{IMPORTS}defineConfig({{ decorations: true }});\n"),
    )
    .expect("init.ts");
    let mut shell = JsShell::new(Box::new(comp.clone()), honest_config(&root)).expect("the shell boots");
    shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    for _ in 0..6 {
        shell.frame().expect("a frame");
    }
    assert_eq!(comp.children(3).len(), 1);
    assert_eq!(comp.children(4).len(), 1);
    assert_ne!(comp.children(3), comp.children(4));
    assert_eq!(comp.canvases().len(), 12);
}

#[test]
fn a_window_has_no_titlebar_unless_the_config_asks_for_one() {
    for init in [
        "export {};",
        "defineConfig({});",
        "defineConfig({ decorations: false });",
    ] {
        let mut rig = rig(init);
        rig.settle();
        rig.comp.create_window(3, "term", "foot");
        rig.settle();
        assert!(rig.comp.children(3).is_empty(), "{init}");
        assert!(rig.comp.canvases().is_empty(), "{init}");
    }
}

#[test]
fn the_close_button_of_a_decorated_window_closes_that_window() {
    let mut rig = rig("defineConfig({ decorations: true });");
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let bar = rig.comp.children(3)[0];
    let close = *rig.comp.children(bar).last().expect("the close button is last");
    rig.comp.pointer(close, "press", 4.0, 4.0, LEFT);
    rig.settle();
    assert!(
        rig.comp.calls().is_empty(),
        "the window fades out before it is asked to close"
    );
    rig.advance(260);
    rig.settle();
    assert_eq!(rig.comp.calls(), [(3, "close".to_owned())]);
}

#[test]
fn retiring_a_generation_takes_its_titlebars_away() {
    let mut rig = rig("defineConfig({ decorations: true });");
    rig.comp.create_window(3, "term", "foot");
    rig.settle();
    let bar = rig.comp.children(3)[0];
    rig.shell.retire().expect("retire");
    rig.settle();
    assert_eq!(rig.comp.destroys_of(bar), 1);
    assert!(
        rig.comp.canvases().is_empty(),
        "no canvas of the old generation is left behind"
    );
}
