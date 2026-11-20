//! The window frame and the stock titlebar (`gnoblin-std/frame/*`, `titlebar.ts`): the room each edge takes, where the content sits,
//! the outlets and layers, the parts, the titlebar's look in each state and what its buttons and clicks do.

use crate::scn::{At, Home, LEFT, MIDDLE, RIGHT, Scn, V, World};

fn world() -> World {
    World::desktop(1000.0, 800.0)
}

/// A frame with a different room on every side, so no side can be mistaken for another: 30 above, 20 right, 40 below and 10 left.
const FOUR: &str = "startDecorations({ frame(client, frame) {\n\
    if (!frame.server) return false;\n\
    frame.top.size = 30; frame.right.size = 20; frame.bottom.size = 40; frame.left.size = 10;\n\
    frame.left.hideWhen = (c) => c.maximised;\n\
} });\n\
globalThis.inset = () => { const i = gnoblin.clients.at(0).frameInset; return [i.top, i.right, i.bottom, i.left]; };\n\
globalThis.bounds = () => { const c = gnoblin.clients.at(0); return [c.x, c.y, c.width, c.height]; };\n";

const WINDOW: At = At::Id(3);

#[test]
fn four_edges() {
    let mut s = Scn::new("frame_four_edges", Home::Reference, &world(), FOUR);
    s.boot();
    s.open(3, "term", "foot")
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("opened");
    s.tree("frame", WINDOW);
    s.set(3, "fullscreen", V::Bool(true))
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("fullscreen");
    s.tree("frame", WINDOW);
    s.set(3, "fullscreen", V::Bool(false))
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("windowed");
    s.request(WINDOW, "maximize", 0.0, 0.0)
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("maximised");
    s.tree("frame", WINDOW);
    s.request(WINDOW, "unmaximize", 0.0, 0.0)
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("unmaximised");
    s.eval("const c = gnoblin.clients.at(0); c.bounds = { x: 300, y: 200, width: 400, height: 300 }; c.floatingBounds = { ...c.bounds };")
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("moved");
    s.set(3, "decoration", V::Text("client"))
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("client_drawn");
    s.tree("frame", WINDOW);
    s.set(3, "decoration", V::Text("server"))
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("server_drawn");
    s.tree("frame", WINDOW);
    s.eval("const c = gnoblin.clients.at(0); c.bounds = { x: 600, y: 500, width: 400, height: 300 }; c.floatingBounds = { ...c.bounds };")
        .set(3, "decoration", V::Text("client"))
        .set(3, "decoration", V::Text("server"))
        .eval("rec('inset', inset()); rec('bounds', bounds());")
        .step("kept_inside_the_area");
    s.destroy(3).step("closed");
    s.finish();
}

#[test]
fn edge_size_changes() {
    let mut s = Scn::new(
        "frame_edge_resize",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) { frame.top.size = 30; globalThis.top = frame.top; } });\n\
         gnoblin.workspaces.add(new Workspace({ id: '1', layout: new MasterStackLayout() }));\n\
         globalThis.bounds = () => { const c = gnoblin.clients.at(0); return [c.x, c.y, c.width, c.height]; };",
    );
    s.boot();
    s.open(3, "term", "foot").eval("rec('bounds', bounds());").step("tiled");
    s.eval("top.size = 50;")
        .eval("rec('bounds', bounds());")
        .step("top_grew");
    s.open(4, "second", "foot")
        .eval("rec('bounds', gnoblin.clients.map((c) => [c.x, c.y, c.width, c.height]));")
        .step("two_tiles");
    s.tree("frame", WINDOW);
    s.finish();
}

#[test]
fn tiled_and_snapped_frames() {
    let mut s = Scn::new(
        "frame_tiled_and_snapped",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 30; frame.right.size = 20; frame.bottom.size = 40; frame.left.size = 10;\n\
         } });\n\
         globalThis.bounds = () => gnoblin.clients.map((c) => [c.x, c.y, c.width, c.height]);",
    );
    s.boot();
    s.open_decorated(3, "term", "foot", "client")
        .eval("snapClient(gnoblin.clients.at(0), 'left', { edgeTiling: true, area: { x: 0, y: 0, width: 1000, height: 800 } }); rec('bounds', bounds());")
        .step("snapped_client_drawn");
    s.set(3, "decoration", V::Text("server"))
        .eval("rec('bounds', bounds());")
        .step("snapped_framed");
    s.set(3, "decoration", V::Text("client"))
        .eval("rec('bounds', bounds());")
        .step("snapped_unframed");
    s.finish();

    let mut s = Scn::new(
        "frame_tiled",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 30; frame.right.size = 20; frame.bottom.size = 40; frame.left.size = 10;\n\
         } });\n\
         gnoblin.workspaces.add(new Workspace({ id: '1', layout: new MasterStackLayout() }));\n\
         globalThis.bounds = () => gnoblin.clients.map((c) => [c.x, c.y, c.width, c.height]);",
    );
    s.boot();
    s.open_decorated(3, "term", "foot", "client")
        .eval("rec('bounds', bounds());")
        .step("tile_client_drawn");
    s.set(3, "decoration", V::Text("server"))
        .eval("rec('bounds', bounds());")
        .step("tile_framed");
    s.open(4, "second", "foot")
        .open(5, "third", "foot")
        .eval("rec('bounds', bounds());")
        .step("three_tiles");
    s.finish();
}

#[test]
fn layers_of_a_client_drawn_window() {
    let mut s = Scn::new(
        "frame_layers",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            frame.underlay.bleed = 20;\n\
            frame.underlay.add(new Box({ x: 20, y: 20, width: 400, height: 300, fill: '#102030' }));\n\
            const badge = new Box({ x: 4, y: 4, width: 10, height: 10, fill: '#ff0000' });\n\
            frame.overlay.add(badge);\n\
            try { frame.top; } catch (error) { rec('refused', String(error.message)); }\n\
            rec('kind', frame.kind);\n\
         } });",
    );
    s.boot();
    s.open_decorated(3, "term", "foot", "client")
        .eval("rec('inset', gnoblin.clients.at(0).frameInset);")
        .step("opened");
    s.tree("frame", WINDOW);
    s.paint("layers", WINDOW);
    s.finish();
}

#[test]
fn layers_of_a_server_window_and_a_ring() {
    let mut s = Scn::new(
        "frame_ring",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 30; frame.left.size = 10;\n\
            frame.overlay.add(new FrameBorder(frame, { thickness: 2, color: (c) => c.focused ? '#89b4fa' : '#45475a' }));\n\
         } });",
    );
    s.boot();
    s.open(3, "term", "foot").step("opened");
    s.tree("frame", WINDOW);
    s.paint("unfocused", WINDOW);
    s.set(3, "focused", V::Bool(true)).step("focused");
    s.paint("focused", WINDOW);
    s.finish();
}

#[test]
fn zones_and_stock_parts() {
    let mut s = Scn::new(
        "frame_zones",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 28;\n\
            frame.start.add(new Box({ width: 40, height: 16 }));\n\
            frame.center.add(new Box({ grow: 1, height: 16 }));\n\
            frame.end.add(new Box({ width: 20, height: 16 }));\n\
         } });",
    );
    s.boot();
    s.open(3, "term", "foot").step("opened");
    s.tree("frame", WINDOW);
    s.finish();

    let mut s = Scn::new(
        "frame_parts",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 28;\n\
            frame.start.add(new WindowButtons(frame, { buttons: ['close', 'minimise'], gap: 6 }));\n\
            frame.center.add(new TitleText(frame));\n\
            frame.end.add(new AppIcon(frame, { size: 16 }));\n\
         } });",
    );
    s.boot();
    s.open(3, "term", "org.example.Files")
        .open(4, "other", "org.gnome.Nautilus")
        .open(5, "three", "firefox")
        .open(6, "four", "")
        .answer_measures()
        .step("opened");
    s.tree("frame", At::Id(3));
    s.paint("files", At::Under(3, &[0]));
    s.paint("nautilus", At::Under(4, &[0]));
    s.paint("firefox", At::Under(5, &[0]));
    s.paint("no_app_id", At::Under(6, &[0]));
    s.set(3, "title", V::Text("renamed")).answer_measures().step("renamed");
    s.paint("renamed", At::Under(3, &[0]));
    s.finish();
}

#[test]
fn stock_titlebar() {
    let mut s = Scn::new(
        "frame_titlebar",
        Home::Reference,
        &world(),
        "startDecorations();\n\
         globalThis.bounds = () => gnoblin.clients.map((c) => [c.x, c.y, c.width, c.height, c.opacity, c.maximised, c.minimised]);",
    );
    let bar = At::Under(3, &[0]);
    s.boot();
    s.open(3, "terminal", "foot")
        .answer_measures()
        .eval("rec('bounds', bounds());")
        .step("opened");
    s.tree("frame", WINDOW);
    s.paint("unfocused", bar);
    s.set(3, "focused", V::Bool(true)).answer_measures().step("focused");
    s.paint("focused", bar);
    s.set(
        3,
        "title",
        V::Text("A very long window title that cannot fit in the room the drag area has left over for it"),
    )
    .answer_measures()
    .step("long_title");
    s.paint("long_title", bar);
    s.set(3, "width", V::Num(200.0)).answer_measures().step("narrow");
    s.paint("narrow", bar);
    s.set(3, "width", V::Num(400.0))
        .set(3, "title", V::Text("terminal"))
        .answer_measures()
        .step("restored");
    s.request(WINDOW, "maximize", 0.0, 0.0)
        .answer_measures()
        .eval("rec('bounds', bounds());")
        .step("maximised");
    s.paint("maximised", bar);
    s.request(WINDOW, "unmaximize", 0.0, 0.0)
        .eval("rec('bounds', bounds());")
        .step("unmaximised");
    s.set(3, "fullscreen", V::Bool(true)).step("fullscreen");
    s.paint("fullscreen", bar);
    s.set(3, "fullscreen", V::Bool(false)).step("windowed");

    // The buttons: maximise toggles, minimise hides, close fades the window and then closes it.
    let minimise = At::Under(3, &[0, 0, 1]);
    let maximise = At::Under(3, &[0, 0, 2]);
    let close = At::Under(3, &[0, 0, 3]);
    s.pointer(maximise, "press", 4.0, 4.0, LEFT)
        .eval("rec('bounds', bounds());")
        .step("maximise_pressed");
    s.pointer(maximise, "press", 4.0, 4.0, RIGHT)
        .step("maximise_right_pressed");
    s.pointer(maximise, "press", 4.0, 4.0, LEFT)
        .eval("rec('bounds', bounds());")
        .step("maximise_pressed_again");
    s.pointer(minimise, "press", 4.0, 4.0, LEFT)
        .eval("rec('bounds', bounds());")
        .step("minimise_pressed");
    s.set(3, "minimised", V::Bool(false)).step("unminimised");
    s.pointer(close, "press", 4.0, 4.0, LEFT)
        .eval("rec('bounds', bounds());")
        .step("close_pressed");
    s.advance(100).eval("rec('bounds', bounds());").step("fading");
    s.advance(100).eval("rec('bounds', bounds());").step("fade_ended");
    s.advance(1500)
        .eval("rec('bounds', bounds());")
        .step("restored_after_staying");
    s.pointer(close, "press", 4.0, 4.0, LEFT)
        .pointer(close, "press", 4.0, 4.0, LEFT)
        .advance(200)
        .step("close_twice");
    s.destroy(3).step("closed");
    s.finish();
}

#[test]
fn titlebar_clicks_and_drag() {
    let mut s = Scn::new(
        "frame_titlebar_clicks",
        Home::Reference,
        &world(),
        "startDecorations(); startWindowMenu();\n\
         globalThis.bounds = () => gnoblin.clients.map((c) => [c.x, c.y, c.width, c.height, c.maximised, c.minimised]);",
    );
    // A press on the bar itself: the stock titlebar is a drag area, and a press on its title reaches it too.
    let drag = At::Under(3, &[0, 0]);
    s.boot();
    s.open(3, "terminal", "foot")
        .answer_measures()
        .eval("rec('bounds', bounds());")
        .step("opened");
    s.pointer(drag, "press", 20.0, 10.0, LEFT)
        .pointer(drag, "motion", 50.0, 25.0, 0)
        .eval("rec('bounds', bounds());")
        .step("dragged");
    s.pointer(drag, "release", 50.0, 25.0, LEFT).step("released");
    s.advance(1000)
        .pointer(drag, "press", 20.0, 10.0, LEFT)
        .advance(200)
        .pointer(drag, "press", 20.0, 10.0, LEFT)
        .eval("rec('bounds', bounds());")
        .step("double_click_toggles_maximise");
    s.advance(1000)
        .pointer(drag, "press", 20.0, 10.0, MIDDLE)
        .step("middle_click_lowers");
    s.pointer(drag, "press", 20.0, 10.0, RIGHT)
        .step("right_click_opens_the_menu");
    s.seat("double_click", V::Text("minimize"))
        .seat("middle_click", V::Text("none"))
        .seat("right_click", V::Text("toggle-maximize"));
    s.advance(1000)
        .pointer(drag, "press", 20.0, 10.0, LEFT)
        .advance(100)
        .pointer(drag, "press", 20.0, 10.0, LEFT)
        .eval("rec('bounds', bounds());")
        .step("double_click_minimises");
    s.pointer(drag, "press", 20.0, 10.0, MIDDLE)
        .step("middle_click_is_nothing");
    s.pointer(drag, "press", 20.0, 10.0, RIGHT)
        .eval("rec('bounds', bounds());")
        .step("right_click_maximises");
    s.seat("double_click", V::Text("bogus"));
    s.advance(1000)
        .pointer(drag, "press", 20.0, 10.0, LEFT)
        .advance(100)
        .pointer(drag, "press", 20.0, 10.0, LEFT)
        .step("bad_action");
    s.finish();
}

#[test]
fn drag_areas_and_resize_handles() {
    let mut s = Scn::new(
        "frame_handles",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 28;\n\
            const bar = new DragArea(frame, { grow: 1, direction: 'row', fill: '#333333' });\n\
            frame.top.add(bar);\n\
            frame.left.size = 0; frame.right.size = 0;\n\
            frame.left.add(new ResizeHandle(frame, 'left', { grow: 1 }));\n\
            frame.right.add(new ResizeHandle(frame, 'right', { grow: 1 }));\n\
         } });\n\
         globalThis.bounds = () => { const c = gnoblin.clients.at(0); return [c.x, c.y, c.width, c.height]; };",
    );
    let bar = At::Under(3, &[2, 0]);
    s.boot();
    s.open(3, "term", "foot")
        .eval("rec('bounds', bounds());")
        .step("opened");
    s.tree("frame", WINDOW);
    s.pointer(bar, "press", 20.0, 10.0, LEFT)
        .pointer(bar, "motion", 50.0, 25.0, 0)
        .eval("rec('bounds', bounds());")
        .step("dragged");
    s.pointer(bar, "release", 50.0, 25.0, LEFT);
    let right = At::Under(3, &[1, 0]);
    s.pointer(right, "press", 0.0, 50.0, LEFT)
        .pointer(right, "motion", 20.0, 50.0, 0)
        .eval("rec('bounds', bounds());")
        .step("right_handle");
    s.pointer(right, "release", 20.0, 50.0, LEFT);
    let left = At::Under(3, &[0, 0]);
    s.pointer(left, "press", 0.0, 50.0, LEFT)
        .pointer(left, "motion", -10.0, 50.0, 0)
        .eval("rec('bounds', bounds());")
        .step("left_handle");
    s.pointer(left, "release", -10.0, 50.0, LEFT);
    s.pointer(left, "press", 0.0, 50.0, LEFT)
        .pointer(left, "motion", 900.0, 50.0, 0)
        .eval("rec('bounds', bounds());")
        .step("left_handle_to_the_least");
    s.finish();

    let mut s = Scn::new(
        "frame_resize_handles",
        Home::Reference,
        &world(),
        "startDecorations({ resizeHandles: 6 });\n\
         globalThis.bounds = () => { const c = gnoblin.clients.at(0); return [c.x, c.y, c.width, c.height]; };",
    );
    s.boot();
    s.open(3, "term", "foot")
        .eval("rec('bounds', bounds());")
        .step("opened");
    s.tree("frame", WINDOW);
    let right = At::Under(3, &[1, 0, 7]);
    s.pointer(right, "press", 2.0, 50.0, LEFT)
        .pointer(right, "motion", 12.0, 50.0, 0)
        .eval("rec('bounds', bounds());")
        .step("right_handle");
    s.request(WINDOW, "maximize", 0.0, 0.0).step("maximised");
    s.tree("frame", WINDOW);
    s.finish();
}

#[test]
fn who_gets_a_frame() {
    let mut s = Scn::new(
        "frame_rules",
        Home::Reference,
        &world(),
        "startDecorations({\n\
            rules: [\n\
                { appId: 'mpv', frame: false },\n\
                { appId: /^org\\./, frame: (client, frame) => { frame.top.size = 10; } },\n\
                { title: 'scratch', frame: (client, frame) => false },\n\
            ],\n\
         });\n\
         globalThis.tops = () => gnoblin.clients.map((c) => c.frameInset.top);",
    );
    s.boot();
    s.open(3, "video", "mpv")
        .open(4, "files", "org.example.Files")
        .open(5, "scratch", "foot")
        .open(6, "plain", "foot")
        .eval("rec('tops', tops());")
        .step("opened");
    for id in [3, 4, 5, 6] {
        s.tree(&format!("window_{id}"), At::Id(id));
    }
    s.finish();

    let mut s = Scn::new(
        "frame_fill_throws",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) { frame.top.size = 30; frame.top.add(new Box({ height: 30 })); throw new Error('no'); } });",
    );
    s.boot();
    s.open(3, "term", "foot")
        .eval("rec('inset', gnoblin.clients.at(0).frameInset);")
        .step("opened");
    s.tree("window", WINDOW);
    s.finish();

    let mut s = Scn::new(
        "frame_fill_again",
        Home::Reference,
        &world(),
        "globalThis.fills = [];\n\
         startDecorations({ frame(client, frame) { globalThis.fills.push(frame.kind); frame.overlay.add(new Box({ width: 4, height: 4 })); } });",
    );
    s.boot();
    s.open(3, "term", "foot")
        .eval("rec('fills', globalThis.fills.join());")
        .step("opened");
    s.tree("window", WINDOW);
    s.set(3, "decoration", V::Text("client"))
        .eval("rec('fills', globalThis.fills.join());")
        .step("client_drawn");
    s.tree("window", WINDOW);
    s.set(3, "decoration", V::Text("server"))
        .eval("rec('fills', globalThis.fills.join());")
        .step("server_drawn");
    s.destroy(3).step("closed");
    s.finish();

    let mut s = Scn::new(
        "frame_bound_node",
        Home::Reference,
        &world(),
        "startDecorations({ frame(client, frame) {\n\
            if (!frame.server) return false;\n\
            frame.top.size = 20;\n\
            const dot = new Box({ width: 10, height: 10, fill: '#ff0000' });\n\
            dot.properties.bind('visible', () => !client.focused);\n\
            frame.top.add(dot);\n\
         } });",
    );
    s.boot();
    s.open(3, "term", "foot").step("opened");
    s.tree("window", WINDOW);
    s.set(3, "focused", V::Bool(true)).step("focused");
    s.tree("window", WINDOW);
    s.destroy(3).step("closed");
    s.finish();
}

#[test]
fn the_recipes_of_the_window_frame_page() {
    let mut s = Scn::new(
        "frame_recipe_buttons_left",
        Home::Reference,
        &world(),
        "startDecorations({\n\
            frame(_client, frame) {\n\
                if (!frame.server) return false;\n\
                frame.top.size = 34;\n\
                frame.top.fill = '#2a2b3c';\n\
                frame.start.add(new WindowButtons(frame, { buttons: ['close', 'minimise', 'maximise'], gap: 8 }));\n\
                const bar = new DragArea(frame, { grow: 1, direction: 'row' });\n\
                bar.add(new TitleText(frame));\n\
                frame.center.add(bar);\n\
            },\n\
         });",
    );
    s.boot();
    s.open(3, "term", "foot").answer_measures().step("opened");
    s.tree("frame", WINDOW);
    s.paint("bar", At::Under(3, &[0]));
    s.finish();

    let mut s = Scn::new(
        "frame_recipe_glow",
        Home::Reference,
        &world(),
        "startDecorations({\n\
            frame(_client, frame) {\n\
                frame.underlay.bleed = 24;\n\
                frame.underlay.add(new FrameGlow({ color: '#000000', strength: 0.45, radius: 12 }));\n\
                frame.overlay.add(new FrameBorder(frame, { thickness: 2, radius: 12, color: (client) => (client.focused ? '#89b4fa' : '#45475a') }));\n\
            },\n\
         });",
    );
    s.boot();
    s.open(3, "term", "foot")
        .open_decorated(4, "own", "gtk", "client")
        .step("opened");
    s.tree("server", At::Id(3));
    s.tree("client", At::Id(4));
    s.paint("server_layers", At::Id(3));
    s.set(3, "focused", V::Bool(true)).step("focused");
    s.paint("server_layers_focused", At::Id(3));
    s.finish();

    let mut s = Scn::new(
        "frame_recipe_status_strip",
        Home::Reference,
        &world(),
        "const stockTitlebar = titlebarFill(defaultTitlebarStyle);\n\
         class StatusText extends Text {\n\
             frame: any;\n\
             constructor(frame: any) {\n\
                 super({ grow: 1, size: 11, color: '#a6adc8' });\n\
                 this.frame = frame;\n\
             }\n\
             override refresh(): void {\n\
                 const { client } = this.frame;\n\
                 this.text = `${client.appId ?? 'window'}  ${Math.round(client.width)} x ${Math.round(client.height ?? 0)}`;\n\
             }\n\
         }\n\
         startDecorations({\n\
             frame(client, frame) {\n\
                 if (stockTitlebar(client, frame) === false) return false;\n\
                 frame.bottom.size = 22;\n\
                 frame.bottom.fill = '#181825';\n\
                 frame.bottom.padding = 4;\n\
                 frame.bottom.hideWhen = (window) => window.maximised;\n\
                 frame.bottom.add(new StatusText(frame));\n\
                 frame.right.size = 12;\n\
                 frame.right.fill = '#313244';\n\
                 frame.right.add(new ResizeHandle(frame, 'right', { grow: 1 }));\n\
             },\n\
         });\n\
         globalThis.inset = () => { const i = gnoblin.clients.at(0).frameInset; return [i.top, i.right, i.bottom, i.left]; };",
    );
    s.boot();
    s.open(3, "term", "foot")
        .answer_measures()
        .eval("rec('inset', inset());")
        .step("opened");
    s.tree("frame", WINDOW);
    s.paint("strip", WINDOW);
    let handle = At::Under(3, &[0, 0]);
    s.pointer(handle, "press", 2.0, 50.0, LEFT)
        .pointer(handle, "motion", 12.0, 50.0, 0)
        .answer_measures()
        .step("handle_dragged");
    s.paint("strip_after_resize", WINDOW);
    s.request(WINDOW, "maximize", 0.0, 0.0)
        .eval("rec('inset', inset());")
        .step("maximised");
    s.finish();
}
