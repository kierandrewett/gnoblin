//! The object model of `gnoblin-core` (`host/*`, `subscriptions.ts`, `session/*`): which listener hears an event and in what order, how a
//! collection follows its members, `gnoblin.find`, the focus history and what a dead object still says.
//!
//! These scenarios read `gnoblin-core` alone. The order of the log lines is the fixture: the native model must call its listeners in
//! this order.

use crate::scn::{At, Home, Scn, V, World};

/// A 1000 by 600 output `DP-1`, and the two windows `3` and `4` that were there at boot.
fn world() -> World {
    World::desktop(1000.0, 600.0)
        .node(20, "name", V::Text("output-0"))
        .node(20, "connector", V::Text("DP-1"))
        .node(20, "x", V::Num(0.0))
        .node(20, "y", V::Num(0.0))
        .window(3, "one", "app.one", "server")
        .window(4, "two", "app.two", "server")
}

/// Subscribes the three ways a member event can be heard, each logging `where:method:id`. `object` and `collection` are expressions; `facade` is
/// the name on `gnoblin`.
const ARM: &str = "globalThis.arm = (object: string, collection: string, facade: string, method: string) => {\n\
    const G = globalThis as any;\n\
    G.offs = [\n\
        G.eval(object)[method]((o: any) => G.log.push('object:' + method + ':' + o.id)),\n\
        G.eval(collection)[method]((o: any) => G.log.push('collection:' + method + ':' + o.id)),\n\
        G.gnoblin[facade]((o: any) => G.log.push('facade:' + method + ':' + o.id)),\n\
    ];\n\
};\n";

/// What the compositor does to raise the repeating events of a window.
fn client_cause(s: &mut Scn, method: &str, count: u32) {
    let id = At::Id(3);
    match method {
        "onTitleChanged" => {
            s.set(3, "title", V::Str(format!("t{count}")));
        }
        "onAppIdChanged" => {
            s.set(3, "app_id", V::Str(format!("a{count}")));
        }
        "onDecorationChanged" => {
            s.set(
                3,
                "decoration",
                V::Text(if count % 2 == 1 { "client" } else { "server" }),
            );
        }
        "onFocusChanged" => {
            s.set(3, "focused", V::Bool(count % 2 == 1));
        }
        "onGeometryChanged" => {
            s.set(3, "width", V::Num(300.0 + f64::from(count)));
        }
        "onPointerEnter" => {
            s.pointer(id, "enter", 1.0, 2.0, 0);
        }
        "onPointerMotion" => {
            s.pointer(id, "motion", 1.0, 2.0, 0);
        }
        "onPointerLeave" => {
            s.pointer(id, "leave", 1.0, 2.0, 0);
        }
        "onPointerPress" => {
            s.pointer(id, "press", 1.0, 2.0, 0);
        }
        "onPointerRelease" => {
            s.pointer(id, "release", 1.0, 2.0, 0);
        }
        other => panic!("no cause for {other}"),
    }
}

#[test]
fn client_events() {
    let mut s = Scn::new("model_client_events", Home::Model, &world(), ARM);
    s.boot();
    for method in [
        "onTitleChanged",
        "onAppIdChanged",
        "onDecorationChanged",
        "onFocusChanged",
        "onGeometryChanged",
        "onPointerEnter",
        "onPointerMotion",
        "onPointerLeave",
        "onPointerPress",
        "onPointerRelease",
    ] {
        let facade = format!("onClient{}", &method[2..]);
        s.eval(&format!(
            "arm(\"gnoblin.clients.get('3')\", 'gnoblin.clients', '{facade}', '{method}');"
        ))
        .step(&format!("{method}_subscribed"));
        client_cause(&mut s, method, 1);
        s.step(&format!("{method}_heard"));
        s.eval("offs.forEach((off) => { off(); off(); });")
            .step(&format!("{method}_unsubscribed"));
        client_cause(&mut s, method, 2);
        s.step(&format!("{method}_heard_by_none"));
    }
    s.finish();
}

#[test]
fn client_lifecycle() {
    let mut s = Scn::new("model_client_lifecycle", Home::Model, &world(), "export {};\n");
    s.boot();
    s.eval(
        "globalThis.offs = [\n\
           gnoblin.clients.get('3').onClosed((c) => log.push('object:onClosed:' + c.id)),\n\
           gnoblin.clients.onClosed((c) => log.push('collection:onClosed:' + c.id)),\n\
           gnoblin.onClientClosed((c) => log.push('facade:onClientClosed:' + c.id)),\n\
           gnoblin.clients.onRemoved((c, from) => log.push('collection:onRemoved:' + c.id + ':' + (from === gnoblin.clients))),\n\
         ];",
    )
    .step("subscribed_to_closing");
    s.destroy(3).step("window_closed");
    s.eval("offs.forEach((off) => off());")
        .destroy(4)
        .step("unsubscribed_window_closed");
    s.eval(
        "globalThis.offs = [\n\
           gnoblin.clients.onAdded((c, to) => log.push('collection:onAdded:' + c.id + ':' + (to === gnoblin.clients))),\n\
           gnoblin.onClientCreated((c) => log.push('facade:onClientCreated:' + c.id)),\n\
           gnoblin.clients.onReady((c) => log.push('collection:onReady:' + c.id)),\n\
           gnoblin.onClientReady((c) => log.push('facade:onClientReady:' + c.id)),\n\
         ];",
    )
    .step("subscribed_to_arrivals");
    s.open(100, "new", "app.new").step("window_arrived");
    s.eval("offs.forEach((off) => off());")
        .open(101, "newer", "app.newer")
        .step("unsubscribed_window_arrived");
    s.finish();

    let mut s = Scn::new(
        "model_created_replays",
        Home::Model,
        &world(),
        "globalThis.off = gnoblin.onClientCreated((client) => log.push('created ' + client.id));\n",
    );
    s.boot();
    s.open(5, "three", "app.three").step("window_arrived");
    s.eval("off();")
        .open(6, "four", "app.four")
        .step("detached_then_arrived");
    s.finish();

    let mut s = Scn::new(
        "model_ready_at_boot",
        Home::Model,
        &world(),
        "gnoblin.clients.get('3').onReady((c) => log.push('object:' + c.id));\n\
         gnoblin.clients.onReady((c) => log.push('collection:' + c.id));\n\
         gnoblin.onClientReady((c) => log.push('facade:' + c.id));\n",
    );
    s.boot();
    s.finish();
}

#[test]
fn output_events() {
    let mut s = Scn::new("model_output_events", Home::Model, &world(), ARM);
    s.boot();
    s.eval("rec('id', gnoblin.outputs.at(0).id);").step("output_id");
    for (method, cause) in [
        ("onModeChanged", "mode"),
        ("onScaleChanged", "scale"),
        ("onGeometryChanged", "geometry"),
        ("onApplied", "apply"),
    ] {
        let facade = format!("onOutput{}", &method[2..]);
        s.eval(&format!(
            "arm('gnoblin.outputs.at(0)', 'gnoblin.outputs', '{facade}', '{method}');"
        ))
        .step(&format!("{method}_subscribed"));
        for round in [1_u32, 2] {
            if round == 2 {
                s.eval("offs.forEach((off) => { off(); off(); });")
                    .step(&format!("{method}_unsubscribed"));
            }
            match cause {
                "mode" => {
                    s.set(20, "mode", V::Str(format!("{}x600@60", 800 + round)));
                }
                "scale" => {
                    s.set(20, "output_scale", V::Num(1.0 + f64::from(round)));
                }
                "geometry" => {
                    s.set(20, "width", V::Num(1000.0 + f64::from(round)));
                }
                _ => {
                    s.eval(&format!("gnoblin.outputs.at(0).apply({{ scale: 1.{round} }});"));
                    s.answer_apply("", "");
                }
            }
            s.step(&format!("{method}_round_{round}"));
        }
    }
    s.finish();

    let mut s = Scn::new("model_output_hotplug", Home::Model, &world(), "export {};\n");
    s.boot();
    s.eval(
        "globalThis.offs = [\n\
           gnoblin.outputs.onConnected((o) => log.push('collection:onConnected:' + o.name)),\n\
           gnoblin.onOutputConnected((o) => log.push('facade:onOutputConnected:' + o.name)),\n\
           gnoblin.outputs.onAdded((o, to) => log.push('collection:onAdded:' + o.name + ':' + (to === gnoblin.outputs))),\n\
           gnoblin.onOutputAdded((o) => log.push('facade:onOutputAdded:' + o.name)),\n\
           gnoblin.outputs.onRemoved((o) => log.push('collection:onRemoved:' + o.name)),\n\
           gnoblin.onOutputRemoved((o) => log.push('facade:onOutputRemoved:' + o.name)),\n\
           gnoblin.outputs.onDisconnected((o) => log.push('collection:onDisconnected:' + o.name + ':' + o.connected)),\n\
           gnoblin.onOutputDisconnected((o) => log.push('facade:onOutputDisconnected:' + o.name)),\n\
         ];",
    )
    .step("subscribed");
    s.plug(30, "output-1", (1000.0, 0.0, 600.0, 400.0), false)
        .step("plugged_in");
    s.destroy(30).step("unplugged");
    s.eval("offs.forEach((off) => off());")
        .plug(31, "output-2", (1000.0, 0.0, 600.0, 400.0), false)
        .step("unsubscribed_plugged_in");
    s.finish();
}

#[test]
fn listeners_and_subscriptions_are_one_list() {
    let mut s = Scn::new(
        "model_listener_order",
        Home::Model,
        &world(),
        "const c = gnoblin.clients.get('3');\n\
         const note = (who: string) => (...args: any[]) => (globalThis as any).log.push(who + ':' + args.length + ':' + args[0].id + ':' + args[1]);\n\
         c.addListener({ onTitleChanged: note('A') });\n\
         c.onTitleChanged(note('B'));\n\
         c.addListener({ onTitleChanged: note('C'), onFocusChanged: note('c') });\n\
         gnoblin.clients.addListener({ onTitleChanged: note('D') });\n\
         gnoblin.clients.onTitleChanged(note('E'));\n\
         gnoblin.onClientTitleChanged(note('F'));\n\
         gnoblin.clients.addListener({ onTitleChanged: note('G') });\n\
         gnoblin.clients.onFocusChanged(note('f'));\n\
         c.onFocusChanged(note('b'));\n",
    );
    s.boot();
    s.set(3, "title", V::Text("t1")).step("title_one");
    s.set(3, "title", V::Text("t2")).step("title_two");
    s.set(3, "focused", V::Bool(true)).step("focus");
    s.finish();

    let mut s = Scn::new(
        "model_listener_removal",
        Home::Model,
        &world(),
        "const c = gnoblin.clients.get('3');\n\
         const G = globalThis as any;\n\
         G.keep = { onTitleChanged() { G.log.push('keep'); } };\n\
         G.gone = { onTitleChanged() { G.log.push('gone'); } };\n\
         c.addListener(G.gone);\n\
         G.off = c.onTitleChanged(() => G.log.push('sub'));\n\
         c.addListener(G.keep);\n\
         c.addListener(G.keep);\n",
    );
    s.boot();
    s.set(3, "title", V::Text("first")).step("first");
    s.eval("gnoblin.clients.get('3').removeListener(gone); off();")
        .set(3, "title", V::Text("second"))
        .step("after_removal");
    s.finish();

    let mut s = Scn::new(
        "model_listener_errors",
        Home::Model,
        &world(),
        "globalThis.fail = (f: () => void) => { try { f(); return 'accepted'; } catch (e) { return (e as Error).name + ': ' + (e as Error).message; } };\n\
         globalThis.explode = { onTitleChanged() { throw new Error('boom'); } };\n",
    );
    s.boot();
    s.eval(
        "const c = gnoblin.clients.get('3');\n\
         rec('refused', [fail(() => c.onTitleChanged(7)), fail(() => c.addListener(null)), fail(() => c.addListener({ onNothing() {} })),\n\
           fail(() => gnoblin.clients.onAdded('x'))]);\n\
         c.addListener(explode); c.onTitleChanged(() => log.push('after the throw'));",
    )
    .step("registration");
    s.set(3, "title", V::Text("x")).step("a_throwing_listener");
    s.set(3, "title", V::Text("y")).step("the_same_failure_again");
    s.finish();
}

#[test]
fn collections_and_find() {
    let mut s = Scn::new("model_collections", Home::Model, &world(), "export {};\n");
    s.boot();
    s.eval(
        "globalThis.foot = gnoblin.clients.where({ appId: 'app.one' });\n\
         globalThis.either = gnoblin.clients.where({ appId: ['app.one', 'app.two'] });\n\
         globalThis.titled = gnoblin.clients.where({ title: 'two', focused: false });\n\
         const ids = (c) => c.map((x) => x.id);\n\
         rec('lengths', [gnoblin.clients.length, foot.length, either.length, titled.length]);\n\
         rec('members', [ids(gnoblin.clients), ids(foot), ids(either), ids(titled)]);\n\
         rec('lookup', [gnoblin.clients.get('3').title, gnoblin.clients.get('99') === undefined, gnoblin.clients.at(-1).id, gnoblin.clients.at(5) === undefined]);\n\
         rec('snapshots', [gnoblin.clients.filter((c) => c.width > 0).length, [...gnoblin.clients].map((c) => c.id), gnoblin.clients.find((c) => c.title === 'two').id]);\n\
         rec('refused', [caught(() => gnoblin.clients.where({ sparkle: 1 })), caught(() => gnoblin.clients.where([])), caught(() => gnoblin.clients.where({ title: {} })),\n\
           caught(() => gnoblin.clients.where({ title: 'x' }, { priority: NaN })), caught(() => gnoblin.clients.add)]);",
    )
    .step("selections");
    s.set(3, "app_id", V::Text("app.two"))
        .eval("rec('members', [foot.length, either.length, gnoblin.clients.get('3').appId]);")
        .step("a_property_leaves_a_selection");
    s.set(4, "title", V::Text("renamed"))
        .eval("rec('titled', titled.length);")
        .step("a_property_leaves_another");
    s.open(5, "two", "app.one")
        .eval("rec('members', [foot.map((c) => c.id), titled.map((c) => c.id)]);")
        .step("a_window_joins_selections");
    s.eval(
        "globalThis.heard = [];\n\
         foot.addListener({ onAdded: (c, to) => log.push('replay:' + c.id + ':' + (to === foot)), onRemoved: (c) => log.push('removed:' + c.id) });\n\
         foot.addListener({ onTitleChanged: (c) => log.push('title:' + c.id) });",
    )
    .step("a_collection_listener_is_told_of_the_members");
    s.set(5, "title", V::Text("moved")).step("title_of_a_member");
    s.set(5, "app_id", V::Text("elsewhere")).step("a_member_leaves");
    s.set(5, "title", V::Text("not heard")).step("no_longer_heard");
    s.destroy(5).step("closed");
    s.eval("rec('find', [gnoblin.find('Window').map((c) => c.id), gnoblin.find('Window', { appId: 'app.two' }).map((c) => c.id), gnoblin.find('Window', (c) => c.width === 400).map((c) => c.id), gnoblin.find('Output').length]);")
        .step("find");
    s.eval(
        "rec('find_refused', [caught(() => gnoblin.find('Spaceship')), caught(() => gnoblin.find('Window', { sparkle: 1 })), caught(() => gnoblin.find('Window', { title: {} })),\n\
           caught(() => gnoblin.find('Window', [])), gnoblin.find('Window', { app_id: 'app.two' }).length]);",
    )
    .step("find_refusals");
    s.finish();
}

#[test]
fn focus_history_and_timers() {
    let mut s = Scn::new(
        "model_focus_history",
        Home::Model,
        &world().window(5, "three", "app.three", "server"),
        "export {};\n",
    );
    s.boot();
    s.eval("rec('history', gnoblin.focus.history().map((c) => c.id));")
        .step("nothing_focused");
    s.set(4, "focused", V::Bool(true))
        .eval("rec('history', gnoblin.focus.history().map((c) => c.id));")
        .step("four_focused");
    s.set(4, "focused", V::Bool(false))
        .set(5, "focused", V::Bool(true))
        .eval("rec('history', gnoblin.focus.history().map((c) => c.id));")
        .step("five_focused");
    s.set(5, "focused", V::Bool(false)).set(3, "focused", V::Bool(true)).eval("rec('history', gnoblin.focus.history().map((c) => c.id)); rec('limited', gnoblin.focus.history({ limit: 2 }).map((c) => c.id));")
        .step("three_focused");
    s.destroy(4)
        .eval("rec('history', gnoblin.focus.history().map((c) => c.id));")
        .step("a_closed_window_leaves_the_history");
    s.eval(
        "rec('refused', [caught(() => gnoblin.focus.history({ limit: 0 })), caught(() => gnoblin.focus.history({ limit: 1.5 })), caught(() => gnoblin.focus.history({ limit: 257 })),\n\
           caught(() => gnoblin.focus.history({ sparkle: 1 })), caught(() => gnoblin.focus.history(7)), gnoblin.focus.history({ workspaceId: 'none' }).length]);",
    )
    .step("refusals");
    s.finish();

    let mut s = Scn::new("model_timers", Home::Model, &world(), "export {};\n");
    s.boot();
    s.eval(
        "globalThis.t0 = gnoblin.timers.now();\n\
         gnoblin.timers.after(100, () => log.push('after 100'));\n\
         const cancel = gnoblin.timers.after(50, () => log.push('after 50 cancelled')); if (typeof cancel === 'function') cancel(); else if (cancel && cancel.cancel) cancel.cancel();\n\
         gnoblin.timers.after(0, () => log.push('after 0'));\n\
         gnoblin.timers.after(100, () => log.push('after 100 again'));\n\
         rec('now', t0);",
    )
    .step("scheduled");
    s.advance(60).step("sixty");
    s.advance(60)
        .eval("rec('elapsed', gnoblin.timers.now() - t0);")
        .step("hundred_and_twenty");
    s.finish();
}

#[test]
fn dead_objects() {
    let mut s = Scn::new("model_dead_objects", Home::Model, &world(), "export {};\n");
    s.boot();
    s.eval(
        "globalThis.c = gnoblin.clients.get('3'); globalThis.o = gnoblin.outputs.at(0);\n\
         rec('alive', [c.alive, o.alive, c.title, c.width]);",
    )
    .step("alive");
    s.destroy(3).eval(
        "rec('dead', [c.alive, c.title, c.width, gnoblin.clients.length, gnoblin.clients.get('3') === undefined]);\n\
         rec('write', [caught(() => { c.opacity = 0.5; }), caught(() => { c.title = 'x'; })]);",
    )
    .step("window_closed");
    s.destroy(20)
        .eval("rec('dead', [o.alive, o.connected, gnoblin.outputs.length]);")
        .step("output_removed");
    s.finish();
}
