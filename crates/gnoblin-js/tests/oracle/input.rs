//! The input matcher of `gnoblin-core` (`bindings/*`, `input/*`): the trigger grammar, claims and their sharing, tiers and priority,
//! decline, pointer drags and gestures. The order of the log and the claims the shell makes are the fixture.

use crate::scn::{Home, Scn, World};

const SESSION: &str = "gnoblin.workspaces.add(new Workspace({ id: 'main', layout: 'monocle' }));\n\
    globalThis.input = gnoblin.input.primary;\n";

const PROBE: &str = "class Probe extends PointerDragBinding {\n\
        label: string;\n\
        constructor(label: string, trigger: string) { super({ id: label, trigger }); this.label = label; }\n\
        onStart(e: any) { rec('start', [this.label, e.client?.id, e.position.x, e.position.y]); return { n: 0 }; }\n\
        onUpdate(e: any, s: any) { s.n++; rec('update', [this.label, e.position.x, e.position.y]); }\n\
        onEnd(e: any, s: any) { rec('end', [this.label, s.n]); }\n\
        onCancel(e: any, s: any) { rec('cancel', [this.label, s.n]); }\n\
    }\n\
    (globalThis as any).Probe = Probe;\n";

#[test]
fn trigger_grammar() {
    let mut s = Scn::new(
        "input_grammar",
        Home::Model,
        &World::desktop(1000.0, 800.0),
        "export {};\n",
    );
    s.boot();
    s.eval(
        "const cases = ['Super+Return', 'super+return', 'Super+Shift+1', 'Shift+Super+1', 'Ctrl+Alt+Delete', 'control+alt+delete', 'Primary+q', 'Super+Q',\n\
           'Super+pagedown', 'Super+Page_Down', 'Super+plus', 'XF86AudioMute', 'Super', 'Logo+Return', 'Mod1+Tab', 'Super+MouseLeft+Drag',\n\
           'super+mouseleft+drag', 'Super+MouseLeft', 'Super+MouseLeft+Click', 'MouseMiddle', 'Super+Wheel', 'Super+wheelup', 'Swipe3Left', 'swipe4',\n\
           'pinch2in', 'Pinch5Out', 'Swipe2Up'];\n\
         rec('canonical', cases.map((c) => [c, parseTrigger(c).text, parseTrigger(c).modifiers.join()]));\n\
         rec('claims', ['Super+1', 'Super+Shift+1', 'Ctrl+Alt+Delete', 'Super+MouseLeft+Drag', 'Super+MouseLeft', 'Super', 'Swipe3Left', 'Super+Wheel'].map((t) => [t, JSON.stringify(claimOf(parseTrigger(t)))]));",
    )
    .step("accepted");
    s.eval(
        "const bad = ['', 'Super+', 'Super++1', '+Super', 'Super+Shift', 'Shift', 'Super+Super+1', 'Super+a+b', 'a+Super', 'Super+!', 'Super+Mod4+1', 'Super+Drag',\n\
           'MouseLeft+MouseRight', 'Super+MouseLeft+a', 'Swipe1Left', 'Swipe6', 'Swipe3In', 'Pinch3Left', 'Super+Swipe3', 'Super + 1', 'Super+Ctrl-1',\n\
           'Super+MouseLeft+Drag+Drag', 'Super+Wheel+Wheel', 'Super+Return+Drag'];\n\
         rec('refused', bad.map((c) => { try { parseTrigger(c, 'trigger'); return [c, 'accepted']; } catch (e) { return [c, e.name + ': ' + e.message + ' [' + e.token + ']']; } }));",
    )
    .step("refused");
    s.eval(
        "const list = (layout) => [...layout.bindings].map((b) => b.id + '=' + b.trigger).join(',');\n\
         const defaults = new FloatingLayout();\n\
         const out = [list(new FloatingLayout()), list(new FloatingLayout({ triggers: { resize: null } })), list(new FloatingLayout({ triggers: { move: undefined } })),\n\
           list(new FloatingLayout({ triggers: { move: 'Alt+MouseLeft+Drag', resize: null } }))];\n\
         defaults.bindings.get('move').trigger = 'Alt+MouseLeft+Drag';\n\
         out.push(list(defaults), defaults.bindings.remove('resize'), String(defaults.bindings.get('resize')));\n\
         defaults.bindings.clear(); out.push(defaults.bindings.size);\n\
         out.push(fail(() => new FloatingLayout({ triggers: { move: 'Super+MouseLeft+Drag', resize: 'Super+MouseLeft+Drag' } })));\n\
         rec('floating', out);",
    )
    .step("floating_bindings");
    s.finish();
}

#[test]
fn chords_and_tiers() {
    let body = format!(
        "{SESSION}\
         input.bind('Super+Shift+1', (e: any) => rec('fired', [e.trigger, e.client?.id]));\n\
         input.bind('Super+k', () => rec('low'));\n\
         globalThis.high = input.bind('Super+k', new KeyBinding({{ id: 'high', trigger: 'Super+k', priority: 5, action: () => rec('high') }}));\n\
         input.bind('Super+a', () => {{ throw new Error('boom'); }});\n\
         input.bind('Super+b', () => rec('b'));\n\
         input.bind('Super+z', false);\n\
         input.bind('Super+d', () => false);\n\
         input.bind('Super+d', new KeyBinding({{ id: 'second', trigger: 'Super+d', priority: -1, action: () => rec('second') }}));\n\
         const layout = new FloatingLayout({{ triggers: {{ move: null, resize: null }} }});\n\
         layout.bindings.add(new KeyBinding({{ id: 'layout', trigger: 'Super+x', action: () => rec('layout') }}));\n\
         input.attachLayout(layout, (c: any) => c.id === '3');\n\
         input.bind('Super+x', () => rec('context'));\n\
         input.devices[0].bind('Super+x', () => {{ rec('device declines'); return false; }});\n\
         globalThis.conflict = fail(() => input.bind('Super+x', () => {{}}));\n"
    );
    let mut s = Scn::new("input_chords", Home::Model, &World::input(false), &body);
    s.boot();
    s.eval("rec('conflict', conflict);").step("claimed");
    s.chord("<Shift><Super>1", "press").step("fires");
    s.chord("<Shift><Super>1", "repeat")
        .chord("<Shift><Super>1", "release")
        .step("only_the_press_runs");
    s.chord("<Super>k", "press").step("higher_priority_first");
    s.eval("high.unbind();").step("unbind_high");
    s.chord("<Super>k", "press").step("lower_priority_after");
    s.chord("<Super>a", "press")
        .chord("<Super>b", "press")
        .step("a_throwing_handler");
    s.chord("<Super>a", "press").step("the_same_failure_again");
    s.chord("<Super>z", "press")
        .chord("<Super>d", "press")
        .step("false_swallows_and_a_returned_false_declines");
    s.chord("<Super>x", "press").step("layout_beats_context");
    s.chord("Macro Pad\n<Super>x", "press")
        .step("device_first_then_decline_falls_through");
    s.eval("input.devices[0].bindingOnly = true;").step("binding_only");
    s.eval("input.devices[0].bindingOnly = false;")
        .step("binding_only_given_up");
    s.finish();
}

#[test]
fn drags_and_gestures() {
    let body = format!(
        "{SESSION}{PROBE}\
         globalThis.layout = new FloatingLayout();\n\
         gnoblin.workspaces.get('main').layout = globalThis.layout;\n\
         class G extends GestureBinding {{\n\
             label: string;\n\
             constructor(trigger: string, label: string) {{ super({{ id: trigger, trigger }}); this.label = label; }}\n\
             onStart(e: any) {{ rec('g.start', [this.label, e.progress]); return {{}}; }}\n\
             onUpdate(e: any) {{ rec('g.update', [this.label, e.progress]); }}\n\
             onEnd(e: any) {{ rec('g.end', [this.label, e.progress]); }}\n\
             onCancel() {{ rec('g.cancel', [this.label]); }}\n\
         }}\n\
         input.bind('Swipe3Left', new G('Swipe3Left', 'left'));\n\
         input.bind('Swipe3Right', new G('Swipe3Right', 'right'));\n"
    );
    let mut s = Scn::new("input_drags", Home::Model, &World::input(false), &body);
    let bounds =
        "rec('bounds', (() => { const c = gnoblin.clients.get('3'); return [c.x, c.y, c.width, c.height]; })());";
    s.boot();
    s.eval("rec('bounds', 0);").step("claimed");
    s.pointer_at(5, 50.0, 60.0)
        .chord("<Super>Button1", "press")
        .eval(bounds)
        .step("press_moves_nothing");
    s.pointer_at(5, 52.0, 61.0).eval(bounds).step("within_the_threshold");
    s.pointer_at(5, 80.0, 100.0).eval(bounds).step("moved_by_the_delta");
    s.chord("<Super>Button1", "release")
        .pointer_at(5, 400.0, 400.0)
        .eval(bounds)
        .step("a_released_drag_follows_nothing");
    s.pointer_at(5, 20.0, 30.0)
        .chord("<Super>Button3", "press")
        .pointer_at(5, 60.0, 70.0)
        .eval(bounds)
        .step("resize_top_left");
    s.pointer_at(5, 350.0, 230.0)
        .eval(bounds)
        .step("resize_held_at_the_minimum");
    s.chord("<Super>Button3", "release").step("resize_released");
    s.pointer_at(5, 600.0, 600.0)
        .chord("<Super>Button1", "press")
        .pointer_at(5, 650.0, 650.0)
        .chord("<Super>Button1", "release")
        .eval(bounds)
        .step("a_drag_over_no_client_is_declined");
    s.eval("input.bind('Super+MouseMiddle+Drag', new Probe('p', 'Super+MouseMiddle+Drag'));")
        .step("probe_bound");
    s.pointer_at(5, 50.0, 60.0)
        .chord("<Super>Button2", "press")
        .pointer_at(5, 70.0, 80.0)
        .step("probe_started");
    s.destroy(3)
        .pointer_at(5, 90.0, 100.0)
        .chord("<Super>Button2", "release")
        .step("window_dies_mid_drag");
    s.gesture("swipe", 3, "begin", 0.0, 0.0, 1.0, 0.0)
        .gesture("swipe", 3, "update", -4.0, 1.0, 1.0, 0.0)
        .step("undecided");
    s.gesture("swipe", 3, "update", -20.0, 2.0, 1.0, 0.0)
        .gesture("swipe", 3, "update", -100.0, 5.0, 1.0, 0.0)
        .gesture("swipe", 3, "end", -120.0, 5.0, 1.0, 0.0)
        .step("left_swipe");
    s.gesture("swipe", 3, "begin", 0.0, 0.0, 1.0, 0.0)
        .gesture("swipe", 3, "update", 30.0, 0.0, 1.0, 0.0)
        .gesture("swipe", 3, "end", 40.0, 0.0, 1.0, 0.0)
        .step("right_swipe");
    s.gesture("swipe", 3, "begin", 0.0, 0.0, 1.0, 0.0)
        .gesture("swipe", 3, "update", -50.0, 0.0, 1.0, 0.0)
        .gesture("swipe", 3, "cancel", -50.0, 0.0, 1.0, 0.0)
        .step("cancelled_swipe");
    s.finish();
}
