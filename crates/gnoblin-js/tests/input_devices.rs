//! Input contexts and devices (owner-spec.md sections 10 and 17, `data/js/gnoblin/{devices,input,input_list}.ts`) over the
//! fake compositor of `tests/common`: the device classes a seat's devices are, their settings reaching the
//! configuration document, a second `InputContext` holding devices, the registry of extensions, and the style lists'
//! `add`.

use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use gnoblin_js::{JsConfigCommit, JsConfigLink, JsLinkError, JsShell};
use gnoblin_tree_client::tree_mirror::MirrorValue;

mod common;
use common::{FakeCompositor, ROOT, text, timed_config};

const SEAT: u64 = 21;
const XR_SEAT: u64 = 50;

/// The device list of the main seat: a keyboard, a mouse, a touchpad, a touch screen, a tablet and a lid switch.
const DEVICES: [&str; 6] = [
    "1:USB Keyboard:keyboard",
    "2:Logitech MX Master:pointer",
    "3:Touchpad:pointer,gesture",
    "4:Screen:touch",
    "5:Wacom Pen:tablet-tool",
    "6:Lid Switch:switch",
];

/// The configuration document the shell commits to: every commit, in order.
#[derive(Clone, Default)]
struct Doc(Rc<RefCell<Vec<String>>>);

impl Doc {
    fn commits(&self) -> Vec<serde_json::Value> {
        self.0
            .borrow()
            .iter()
            .map(|text| serde_json::from_str(text).expect("a JSON document"))
            .collect()
    }
}

impl JsConfigLink for Doc {
    fn current(&mut self) -> Result<Option<Vec<u8>>, JsLinkError> {
        Ok(self.0.borrow().last().map(|text| text.clone().into_bytes()))
    }

    fn commit(&mut self, document: &[u8]) -> Result<JsConfigCommit, JsLinkError> {
        let mut commits = self.0.borrow_mut();
        commits.push(String::from_utf8(document.to_vec()).expect("a UTF-8 document"));
        Ok(JsConfigCommit {
            revision: commits.len() as u64,
            notices: Vec::new(),
        })
    }
}

struct World {
    shell: JsShell,
    comp: FakeCompositor,
    doc: Doc,
    _dir: tempfile::TempDir,
}

impl World {
    fn frame(&mut self) -> Vec<String> {
        self.shell.frame().expect("a frame").diagnostics
    }

    fn eval(&mut self, source: &str) -> String {
        self.shell
            .eval(source)
            .expect("the script runs")
            .unwrap_or_else(|| "undefined".to_owned())
    }
}

/// A shell over a screen, the main seat with `DEVICES`, a window, and `xr` when the XR program's seat is wanted, whose
/// entry is `body` (TypeScript) after the imports. Returns the diagnostics of loading the entry too.
fn boot(body: &str, xr: bool) -> (World, Vec<String>) {
    let comp = FakeCompositor::new();
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
    comp.add_node(
        SEAT,
        ROOT,
        "seat",
        vec![
            ("name", text("seat0")),
            (
                "device",
                MirrorValue::Texts(DEVICES.iter().map(|device| (*device).to_owned()).collect()),
            ),
        ],
    );
    comp.add_node(22, ROOT, "lock", Vec::new());
    comp.add_node(
        3,
        ROOT,
        "window",
        vec![("title", text("term")), ("app_id", text("foot"))],
    );
    if xr {
        comp.add_node(XR_SEAT, 0, "seat", vec![("name", text("xr"))]);
    }
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    let imports = "import * as lib from \"gnoblin\";\nconst { gnoblin, ...rest } = lib;\nconst g = globalThis as any;\ng.g = g;\nObject.assign(g, rest);\n";
    fs::write(root.join("init.ts"), format!("{imports}{body}\n")).expect("init.ts is written");
    let (config, _clock) = timed_config(&root);
    let mut shell = JsShell::new(Box::new(comp.clone()), config).expect("the shell boots");
    let doc = Doc::default();
    shell.set_config_link(Box::new(doc.clone()));
    let diagnostics = shell.load_entry(Path::new("init.ts")).expect("init.ts loads");
    (
        World {
            shell,
            comp,
            doc,
            _dir: dir,
        },
        diagnostics,
    )
}

fn world(body: &str) -> World {
    let (mut world, diagnostics) = boot(body, false);
    let diagnostics: Vec<&String> = diagnostics
        .iter()
        .filter(|line| !line.starts_with("not applied"))
        .collect();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for _ in 0..3 {
        let frame: Vec<String> = world
            .frame()
            .into_iter()
            .filter(|line| !line.starts_with("not applied"))
            .collect();
        assert!(frame.is_empty(), "{frame:?}");
    }
    world
}

#[test]
fn a_seats_devices_are_the_class_their_capabilities_name() {
    let mut world = world("");
    assert_eq!(
        world.eval(
            "gnoblin.input.devices.map((d) => `${d.constructor.name}:${d.name}:${d instanceof g.InputDevice}`).join('|')"
        ),
        "Keyboard:USB Keyboard:true|Mouse:Logitech MX Master:true|Touchpad:Touchpad:true|Touchscreen:Screen:true|Tablet:Wacom Pen:true|InputDevice:Lid Switch:true"
    );
    assert_eq!(
        world.eval("gnoblin.input.devices.filter((d) => !(d instanceof g.NativeObject && d.alive && d.context === gnoblin.input.primary)).length === 0"),
        "true"
    );
    assert_eq!(world.eval("gnoblin.input.devices.length"), "6");
    assert_eq!(world.eval("gnoblin.input.devices.get('2').name"), "Logitech MX Master");
    assert_eq!(world.eval("gnoblin.input.devices.at(-1).name"), "Lid Switch");
}

#[test]
fn device_settings_are_committed_in_the_input_section_of_the_document() {
    let mut world = world(
        "for (const device of gnoblin.input.devices) {\n\
           if (device instanceof g.Mouse) { device.sensitivity = 0.5; device.acceleration = 'flat'; device.naturalScroll = true; }\n\
           if (device instanceof g.Keyboard) { device.repeatRate = 40; device.repeatDelay = 250; }\n\
           if (device instanceof g.Touchpad) { device.naturalScroll = true; device.tapToClick = true; device.sensitivity = -0.25; }\n\
         }\n",
    );
    let commits = world.doc.commits();
    assert_eq!(commits.len(), 1, "{commits:?}");
    assert_eq!(
        commits[0],
        serde_json::json!({
            "input": {
                "keyboard": { "repeat-interval": 25, "delay": 250 },
                "mouse": { "speed": 0.5, "accel-profile": "flat", "natural-scroll": true },
                "touchpad": { "natural-scroll": true, "tap-to-click": true, "speed": -0.25 },
            }
        })
    );
    // A setting is the kind's, not the device's: another keyboard reads it too.
    assert_eq!(
        world.eval("gnoblin.input.devices.filter((d) => d instanceof g.Mouse).map((d) => d.sensitivity).join()"),
        "0.5"
    );
}

#[test]
fn a_device_setting_written_later_is_committed_on_the_next_frame_and_undefined_gives_it_up() {
    let mut world =
        world("g.mouse = gnoblin.input.devices.find((d) => d instanceof g.Mouse);\ng.mouse.sensitivity = 0.5;\n");
    assert_eq!(world.doc.commits().len(), 1);

    world.eval("(() => { g.mouse.sensitivity = 0.25; })()");
    assert!(world.frame().is_empty());
    let commits = world.doc.commits();
    assert_eq!(commits.len(), 2, "{commits:?}");
    assert_eq!(
        commits[1],
        serde_json::json!({ "input": { "mouse": { "speed": 0.25 } } })
    );

    world.eval("(() => { g.mouse.sensitivity = undefined; })()");
    assert!(world.frame().is_empty());
    let commits = world.doc.commits();
    assert_eq!(commits.len(), 3, "{commits:?}");
    assert_eq!(
        commits[2],
        serde_json::json!({}),
        "the setting went back to the system's"
    );

    assert!(world.frame().is_empty());
    assert_eq!(world.doc.commits().len(), 3, "nothing moved, nothing is committed");
}

#[test]
fn a_bad_device_setting_throws_where_it_is_written_and_leaves_the_setting() {
    let mut world = world(
        "g.errors = [];\n\
         const attempt = (run: () => void) => { try { run(); } catch (e: any) { g.errors.push(`${e.name}${e.code ? '(' + e.code + ')' : ''}: ${e.message}`); } };\n\
         const mouse: any = gnoblin.input.devices.find((d) => d instanceof g.Mouse);\n\
         const keyboard: any = gnoblin.input.devices.find((d) => d instanceof g.Keyboard);\n\
         attempt(() => { mouse.sensitivity = 2; });\n\
         attempt(() => { mouse.acceleration = 'custom'; });\n\
         attempt(() => { keyboard.repeatRate = 0; });\n\
         attempt(() => { keyboard.repeatDelay = 1.5; });\n\
         attempt(() => { mouse.naturalScroll = 'yes'; });\n\
         attempt(() => { mouse.enabled = false; });\n\
         attempt(() => { mouse.enabled = true; });\n\
         attempt(() => { mouse.context = {}; });\n\
         g.after = [mouse.sensitivity, keyboard.repeatRate, mouse.enabled];\n",
    );
    assert_eq!(
        world.eval("errors.join('\\n')"),
        [
            "RangeError: Mouse#2.sensitivity: must be a number from -1 to 1, got 2",
            "RangeError: Mouse#2.acceleration: must be one of default, flat, adaptive, got custom",
            "RangeError: Keyboard#1.repeatRate: must be a number of repeats per second from 0.1 to 1000, got 0",
            "RangeError: Keyboard#1.repeatDelay: must be a whole number of milliseconds from 1 to 10000, got 1.5",
            "RangeError: Mouse#2.naturalScroll: must be true or false, got yes",
            "GnoblinError(unsupported): Mouse#2.enabled: the compositor cannot disable one device; make it binding-only to keep its keys from clients",
            "GnoblinError(refused): Mouse#2.context: assign a context that is in gnoblin.input.contexts",
        ]
        .join("\n")
    );
    assert_eq!(world.eval("after.join()"), ",,true");
    assert!(
        world.doc.commits().is_empty(),
        "a refused write leaves nothing to commit"
    );
}

#[test]
fn a_context_a_script_made_holds_the_devices_assigned_to_it_and_binds_for_them_only() {
    let mut world = world(
        "g.log = [];\n\
         g.second = new g.InputContext({ name: 'secondary' });\n\
         gnoblin.input.contexts.add(g.second);\n\
         g.second.bind('Super+Return', (e: any) => { g.log.push(`${e.context.name}:${e.device?.name}`); });\n\
         g.keyboard = gnoblin.input.devices.find((d) => d.name === 'USB Keyboard');\n",
    );
    assert_eq!(
        world.eval("gnoblin.input.contexts.map((c) => c.name).join()"),
        "seat0,secondary"
    );
    assert_eq!(
        world.eval("second instanceof g.NativeObject && second.alive && second.id"),
        "secondary"
    );
    let base = world.comp.claimed();

    world.eval("(() => { g.keyboard.context = g.second; })()");
    world.frame();
    let mut expected = base.clone();
    expected.push("USB Keyboard|<Super>Return".to_owned());
    assert_eq!(
        world.comp.claimed(),
        expected,
        "a context with no device claimed nothing before"
    );
    assert_eq!(world.eval("second.devices.map((d) => d.name).join()"), "USB Keyboard");
    assert_eq!(
        world.eval("gnoblin.input.primary.devices.map((d) => d.name).includes('USB Keyboard')"),
        "false"
    );
    assert_eq!(
        world.eval("gnoblin.input.devices.length"),
        "6",
        "a device is in one context, not two"
    );

    let claim = world.comp.claim_id("USB Keyboard\n<Super>Return");
    world.comp.chord(claim, "<Super>Return", "press");
    world.frame();
    assert_eq!(world.eval("log.join()"), "secondary:USB Keyboard");

    world.eval("(() => { g.keyboard.context = gnoblin.input.primary; })()");
    world.frame();
    assert_eq!(world.comp.claimed(), base, "the claim went back with the device");
    world.eval("(() => { g.log.length = 0; })()");

    world.eval("(() => { g.keyboard.context = g.second; })()");
    world.frame();
    assert_eq!(world.eval("gnoblin.input.contexts.remove(second)"), "true");
    world.frame();
    assert_eq!(world.comp.claimed(), base);
    assert_eq!(
        world.eval("[second.alive, keyboard.context === gnoblin.input.primary].join()"),
        "false,true"
    );
    assert_eq!(world.eval("gnoblin.input.contexts.map((c) => c.name).join()"), "seat0");
}

#[test]
fn contexts_add_and_remove_refuse_what_cannot_work() {
    let mut world = world(
        "g.errors = [];\n\
         const attempt = (run: () => void) => { try { run(); } catch (e: any) { g.errors.push(`${e.name}${e.code ? '(' + e.code + ')' : ''}: ${e.message}`); } };\n\
         const one = gnoblin.input.contexts.add(new g.InputContext({ name: 'one' }));\n\
         g.again = gnoblin.input.contexts.add(one) === one;\n\
         attempt(() => gnoblin.input.contexts.add(new g.InputContext({ name: 'one' })));\n\
         attempt(() => gnoblin.input.contexts.add(new g.InputContext({ name: '' })));\n\
         attempt(() => gnoblin.input.contexts.add({} as any));\n\
         attempt(() => new g.InputContext({ name: 7 } as any));\n\
         attempt(() => gnoblin.input.contexts.remove(gnoblin.input.primary));\n\
         attempt(() => new g.InputContext({ name: 'loose' }).pointer);\n\
         g.loose = new g.InputContext({ name: 'loose' });\n\
         g.looseBind = g.loose.bind('Super+x', () => {}).trigger;\n\
         g.looseState = [g.loose.devices.length, g.loose.focusedClient, gnoblin.input.contexts.remove(g.loose)].join();\n\
         attempt(() => { gnoblin.input.devices.find((d) => d.name === 'USB Keyboard')!.context = g.loose; });\n",
    );
    assert_eq!(world.eval("again"), "true");
    assert_eq!(
        world.eval("errors.join('\\n')"),
        [
            r#"GnoblinError(duplicate): gnoblin.input.contexts.add: a context named "one" is already there"#,
            r#"TypeError: gnoblin.input.contexts.add: a context name is one non-empty line, got """#,
            "TypeError: gnoblin.input.contexts.add: expects an InputContext made with new InputContext({ name })",
            "TypeError: InputContext: the name must be text, got 7",
            r#"GnoblinError(refused): gnoblin.input.contexts.remove: "seat0" is a seat the compositor reports; only contexts a script added can be removed"#,
            r#"GnoblinError(detached): InputContext "loose".pointer: add the context to gnoblin.input.contexts first"#,
            "GnoblinError(refused): Keyboard#1.context: assign a context that is in gnoblin.input.contexts",
        ]
        .join("\n")
    );
    assert_eq!(world.eval("looseBind"), "Super+x");
    assert_eq!(world.eval("looseState"), "0,,false");
}

#[test]
fn nothing_but_a_subclass_makes_a_native_object() {
    let mut world = world(
        "g.errors = [];\n\
         const attempt = (run: () => void) => { try { run(); } catch (e: any) { g.errors.push(`${e.name}: ${e.message}`); } };\n\
         attempt(() => new g.NativeObject());\n\
         class Thing extends g.NativeObject { size = 1; get fixed() { return 2; } grow() {} }\n\
         const thing: any = new Thing();\n\
         thing.set({ size: 5 });\n\
         attempt(() => thing.set({ nope: 1 }));\n\
         attempt(() => thing.set({ fixed: 1 }));\n\
         attempt(() => thing.set({ grow: 1 }));\n\
         attempt(() => thing.set({ size: 1 }, { duration: 1 }));\n\
         g.size = thing.size;\n",
    );
    assert_eq!(
        world.eval("errors.join('\\n')"),
        [
            "TypeError: NativeObject is abstract: extend it",
            r#"TypeError: Thing.set: no property "nope""#,
            r#"TypeError: Thing.set: "fixed" is read-only"#,
            r#"TypeError: Thing.set: no property "grow""#,
            "TypeError: Thing.set: this object takes no transition options",
        ]
        .join("\n")
    );
    assert_eq!(world.eval("size"), "5");
    assert_eq!(
        world.eval("[gnoblin.clients.at(0) instanceof g.NativeObject, gnoblin.outputs.at(0) instanceof g.NativeObject, new g.Workspace({ id: 'w' }) instanceof g.NativeObject].join()"),
        "true,true,true"
    );
}

#[test]
fn extensions_iterate_with_an_id_and_a_status_and_xr_is_null_until_the_program_has_a_session() {
    let mut world = world("g.seen = [...gnoblin.extensions].map((e) => `${e.id}:${e.status}`).join();\n");
    assert_eq!(world.eval("seen"), "xr:unloaded");
    assert_eq!(world.eval("String(gnoblin.extensions.xr)"), "null");

    let (mut with_xr, diagnostics) = boot("", true);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    for _ in 0..6 {
        with_xr.frame();
    }
    assert_eq!(
        with_xr.eval("[...gnoblin.extensions].map((e) => `${e.id}:${e.status}`).join()"),
        "xr:ready"
    );
    assert_eq!(with_xr.eval("gnoblin.extensions.xr !== null"), "true");
}

#[test]
fn style_lists_take_add_and_remove() {
    let mut world = world(
        "g.errors = [];\n\
         const attempt = (run: () => void) => { try { run(); } catch (e: any) { g.errors.push(`${e.name}: ${e.message}`); } };\n\
         const style = new g.ClientStyle({ cornerRadius: 12 });\n\
         const border = style.borders.add(new g.Border({ width: 2, color: '#7c3aed' }));\n\
         style.shadows.add(new g.Shadow({ x: 0, y: 8, blur: 24, color: '#0006' }));\n\
         style.backdropFilter.add(new g.BlurFilter({ radius: 18 }));\n\
         style.filter.add({ saturate: 1.2 });\n\
         style.borders.add({ width: 1 });\n\
         attempt(() => style.borders.add(border));\n\
         attempt(() => style.borders.add({ width: -1 }));\n\
         attempt(() => style.borders.add('x' as any));\n\
         g.removed = [style.borders.remove(border), style.borders.remove(border), style.borders.length].join();\n\
         g.json = JSON.stringify(style);\n\
         for (let i = 0; i < 8; i++) { attempt(() => style.shadows.add({})); }\n\
         g.shadowCount = style.shadows.length;\n\
         gnoblin.clients.style = style;\n\
         g.global = gnoblin.clients.style.borders.length;\n",
    );
    assert_eq!(
        world.eval("errors.join('\\n')"),
        [
            "TypeError: ClientStyle.borders: that element is already in the list",
            "RangeError: ClientStyle.borders[2].width: must be finite and >= 0",
            "TypeError: ClientStyle.borders[2]: must be a Border or an object of its fields",
            "RangeError: ClientStyle.shadows: holds at most 8 entries, so another cannot be added",
        ]
        .join("\n")
    );
    assert_eq!(world.eval("removed"), "true,false,1");
    assert_eq!(world.eval("shadowCount"), "8");
    assert!(
        world
            .eval("json")
            .contains(r#""backdropFilter":[{"kind":"blur","radius":18}]"#)
    );
    assert_eq!(world.eval("global"), "1");
}
