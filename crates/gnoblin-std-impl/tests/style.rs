#![allow(unsafe_code)]
//! The styles and transitions of `libgnoblin-std`, held to what the TypeScript std did (`data/js/gnoblin-std/style/*`, `style.ts`,
//! `host/style_scopes.ts`, `host/styles.ts`, `transition/*`, `host/staging.ts`, `host/rollback.ts`).
//!
//! Each test replays a scenario of `crates/gnoblin-js/tests/oracle/style.rs` as a shell written over the object protocol, in the same
//! order, and `rig.rs` holds the native run against the fixture at every step: the log, the lines the shell reported, and every node
//! the run touched, whole. What the TypeScript host did as the shell (saying each style note that was not applied, once) the replay
//! does as the shell too (`Replay::settle`).
//!
//! **Allowances of the replay** (each is where it is made, with its reason):
//!
//! 1. A value no typed catalog carries (a key that is not a field, two filter keys at once, a text where a record is declared, a name
//!    of a style property that is not one) is refused by the protocol or the library in its own words; the replay asserts the
//!    refusal and records the fixture's text (`refused`).
//! 2. A message of the TypeScript elements and checkers whose path was the element's own (`ShaderFilter.source`) is the library's path
//!    to the entry (`...filter[0].source`): the words after the path are asserted, the fixture's text is recorded (`same_words`).
//! 3. What a dynamically typed language answers about its own objects (`constructor.name`, `instanceof`, `Object.isFrozen`, a detached
//!    `new Style(...)`) is answered from the kinds the catalog names, or is `true` where the value of a record cannot be changed.
//! 4. `style.shadows[0].blur = 40` edits the element in place in TypeScript; here it is `at`, then `replace`, on the list object.
//! 5. The notices of the frame (`not applied: ...`, said once) are the shell's, so the replay says them (`Replay::settle`).
//! 6. A focus write on a new window is made by the replay (NpStdPolicy's native policy does it later).
//! 7. A transition's callback failing: the TypeScript engine restored a snapshot of the script's state; natively the library puts back
//!    the styles and the shell's `rollback` puts back its own (the replay's closures do), and a write the callback staged is dropped.
//!    The three shared values of `scene_client_set` are allowed for in `scene.rs`; here the rollback is replayed.
//! 8. The workspaces and layouts the transition scenarios use belong to NpStdObjects; the replay makes them through the std table
//!    (`workspace_new`, a foreign layout whose `compute` forwards to the built-in layouts), as the TypeScript library did.

#[path = "ui/rig.rs"]
mod rig;

use std::collections::HashSet;

use gnoblin_core_abi::{E_NO_SUCH_NODE, E_TYPE, MEMBER_METHOD};
use gnoblin_object::{Fail, Val};
use gnoblin_testkit::oracle::{Value, json};
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{Rig, W, d, en, js, recorded, t};

// ---- the shell's side -----------------------------------------------------------------------------------------------------------

const BLUR: u32 = 1;
const SATURATE: u32 = 2;
const BRIGHTNESS: u32 = 3;
const CONTRAST: u32 = 4;
const SHADER: u32 = 5;

const OPACITY: usize = 0;
const CORNER_RADIUS: usize = 1;
const BACKGROUND: usize = 2;
const BORDERS: usize = 3;
const SHADOWS: usize = 4;

const TINT: &str = "fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32> { return c * p[0]; }";

fn n() -> Val {
    Val::None
}

fn rec_of(fields: Vec<Val>) -> Val {
    Val::Record { class: 0, fields }
}

fn list_of(items: Vec<Val>) -> Val {
    Val::List(items)
}

fn shadow(blur: f64) -> Val {
    rec_of(vec![n(), n(), d(blur)])
}

fn shadow_full(x: f64, y: f64, blur: f64, spread: f64, color: &str, inset: bool) -> Val {
    rec_of(vec![d(x), d(y), d(blur), d(spread), t(color), Val::Bool(inset)])
}

fn border(width: f64) -> Val {
    rec_of(vec![d(width)])
}

fn border_full(width: f64, color: &str, offset: f64, inset: bool) -> Val {
    rec_of(vec![d(width), t(color), d(offset), Val::Bool(inset)])
}

fn grade(kind: u32, amount: f64) -> Val {
    rec_of(vec![en(kind), d(amount)])
}

fn shader_filter(source: &str, params: &[f64]) -> Val {
    let params = list_of(params.iter().map(|p| d(*p)).collect());
    rec_of(vec![en(SHADER), n(), rec_of(vec![t(source), params])])
}

fn radius(all: f64) -> Val {
    rec_of(vec![d(all)])
}

fn query_app(app: &str) -> Val {
    rec_of(vec![n(), t(app)])
}

fn query_title(title: &str) -> Val {
    rec_of(vec![n(), n(), t(title)])
}

fn field(value: &Val, at: usize) -> Val {
    value.record().expect("a record")[at].clone()
}

fn num(value: &Val) -> f64 {
    value.double().expect("a number")
}

/// The words of a failure of a call: the protocol puts the symbol of the code in front of them.
fn words(fail: &Fail) -> &str {
    match fail.message.split_once(": ") {
        Some((code, rest)) if code.starts_with("E_") && code.chars().all(|c| c.is_ascii_uppercase() || c == '_') => {
            rest
        }
        _ => &fail.message,
    }
}

fn said(fail: &Fail) -> Value {
    json!(recorded(words(fail)))
}

/// What a script's `caught` records of a call that failed.
fn caught(result: Result<Val, Fail>) -> Value {
    match result {
        Ok(_) => json!("none"),
        Err(fail) => said(&fail),
    }
}

/// A value no typed catalog carries: the call is made as far as it can be (`wrong`) and must be refused with `code`; the fixture's
/// answer is what is recorded.
fn refused(result: Result<Val, Fail>, code: i32, fixture: &str) -> Value {
    let fail = result.expect_err("a value no catalog carries is refused");
    assert_eq!(fail.code, code, "{fail}");
    json!(fixture)
}

/// A message whose path is the library's, not the TypeScript element's: the words after the path are `tail`, and the fixture's
/// text is what is recorded.
fn same_words(result: Result<Val, Fail>, tail: &str, fixture: &str) -> Value {
    let fail = result.expect_err("a bad value is refused");
    assert!(words(&fail).ends_with(tail), "{fail}");
    json!(fixture)
}

fn sfn(rig: &Rig, name: &str, args: &[Val]) -> Result<Val, Fail> {
    let class = rig.api.class("style").expect("the class");
    let member = rig.api.member(class, MEMBER_METHOD, name).expect("the function");
    rig.api.call_static(class, member, args)
}

fn handle(rig: &Rig, value: Val) -> W {
    match value {
        Val::Object { class, obj } => W {
            api: rig.api,
            class,
            obj,
        },
        other => panic!("not an object: {other:?}"),
    }
}

fn clients(rig: &Rig) -> W {
    handle(rig, sfn(rig, "of_clients", &[]).expect("the style of every window"))
}

fn client(rig: &Rig, node: u64) -> W {
    handle(
        rig,
        sfn(rig, "of_client", &[Val::Uint(node)]).expect("the style of a window"),
    )
}

fn workspace(rig: &Rig, id: &str) -> W {
    handle(
        rig,
        sfn(rig, "of_workspace", &[t(id)]).expect("the style of a workspace"),
    )
}

fn sub(rig: &Rig, style: &W, name: &str, args: &[Val]) -> W {
    handle(rig, style.call(name, args))
}

/// A client's resolved style, field by field.
fn computed(rig: &Rig, node: u64) -> Vec<Val> {
    let value = sfn(rig, "computed", &[Val::Uint(node)]).expect("a resolved style");
    value.record().expect("a record").to_vec()
}

fn origin(rig: &Rig, node: u64, prop: usize) -> String {
    field(&computed(rig, node)[7], prop).text().expect("text").to_owned()
}

fn radius_json(value: &Val) -> Value {
    let fields = value.record().expect("radii");
    if !fields[0].is_none() {
        return js(num(&fields[0]));
    }
    json!({
        "topLeft": js(num(&fields[1])),
        "topRight": js(num(&fields[2])),
        "bottomRight": js(num(&fields[3])),
        "bottomLeft": js(num(&fields[4])),
    })
}

fn blurs(list: &Val) -> Vec<Value> {
    list.list()
        .expect("a list")
        .iter()
        .map(|entry| js(num(&field(entry, 2))))
        .collect()
}

fn text_json(value: &Val) -> Value {
    let text = value.text().expect("text");
    if text.is_empty() {
        Value::Null
    } else {
        json!(recorded(text))
    }
}

/// What a scope sets of one property, as the TypeScript style read it: nothing set is `null`.
fn own_text(style: &W, property: &str, place: u32) -> Value {
    if style.call("is_set", &[en(place)]).bool().expect("a flag") {
        text_json(&style.get(property))
    } else {
        Value::Null
    }
}

fn place_of(prop: usize) -> u32 {
    prop as u32 + 1
}

/// The replay's shell: the rig, and the notes it has said.
struct Replay {
    rig: Rig,
    said: HashSet<String>,
}

impl Replay {
    fn new(name: &'static str, windows: u64) -> Replay {
        let rig = Rig::with_plain_windows(name, 1000.0, 800.0, windows);
        // SAFETY: the table's functions take the generation's context.
        assert_eq!(unsafe { (rig.std.open)(rig.ctx) }, 0);
        Replay {
            rig,
            said: HashSet::new(),
        }
    }

    fn boot(&mut self) {
        self.settle();
        self.rig.step("boot");
    }

    /// The windows now, in the order they were made.
    fn windows(&self) -> Vec<u64> {
        let comp = self.rig.comp.borrow();
        let mut ids: Vec<u64> = comp
            .nodes
            .iter()
            .filter(|(_, node)| node.class == "window")
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Turns, and then what the TypeScript shell did after each frame: it said every note of a window's style that was not applied,
    /// once for the generation.
    fn settle(&mut self) {
        self.rig.settle();
        for node in self.windows() {
            let notes = sfn(&self.rig, "warnings", &[Val::Uint(node)]).expect("the notes");
            for note in notes.list().expect("a list") {
                let note = note.text().expect("text").to_owned();
                if note.starts_with("not applied:") && self.said.insert(note.clone()) {
                    self.rig.reports.borrow_mut().push(note);
                }
            }
        }
        self.rig.settle();
    }

    fn step(&mut self, label: &str) {
        self.settle();
        self.rig.step(label);
    }

    fn open(&mut self, id: u64, title: &str, app: &str) {
        self.rig.comp.create_window(id, title, app);
        // The TypeScript library's stock focus policy gave the new window the keyboard (NpStdPolicy's native policy does it later).
        self.rig.external(21, "focus", MirrorValue::Node(id));
    }

    fn rec(&self, label: &str, value: Value) {
        self.rig.rec(label, value);
    }
}

// ---- the cascade ------------------------------------------------------------------------------------------------------------------

#[test]
fn cascade() {
    let mut r = Replay::new("style_cascade", 2);
    r.boot();

    let global = clients(&r.rig);
    global.set("corner_radius", radius(12.0));
    client(&r.rig, 3).set("corner_radius", radius(20.0));
    let radii: Vec<Value> = r
        .windows()
        .iter()
        .map(|c| radius_json(&computed(&r.rig, *c)[CORNER_RADIUS]))
        .collect();
    r.rec("radii", Value::Array(radii));
    r.step("global_and_explicit");

    global.set("corner_radius", radius(8.0));
    r.open(9, "late", "foot");
    r.settle();
    let radii: Vec<Value> = r
        .windows()
        .iter()
        .map(|c| {
            json!([
                c.to_string(),
                radius_json(&computed(&r.rig, *c)[CORNER_RADIUS]),
                origin(&r.rig, *c, CORNER_RADIUS)
            ])
        })
        .collect();
    r.rec("radii", Value::Array(radii));
    r.step("future_clients_inherit");

    let ws = workspace(&r.rig, "desktop");
    let sel_high = sub(&r.rig, &global, "where", &[query_app("foot"), d(5.0)]);
    let sel_low1 = sub(&r.rig, &global, "where", &[query_title("win1")]);
    let sel_low2 = sub(&r.rig, &ws, "where", &[query_app("foot")]);
    let c3 = client(&r.rig, 3);
    let scopes: Vec<(&str, &W)> = vec![
        ("global", &global),
        ("workspace", &ws),
        ("selLow1", &sel_low1),
        ("selLow2", &sel_low2),
        ("selHigh", &sel_high),
        ("client", &c3),
    ];
    // (the TypeScript name, the property, its place in a computed style, two values)
    type Pair = (&'static str, &'static str, usize, Val, Val);
    let props: Vec<Pair> = vec![
        ("opacity", "opacity", OPACITY, d(0.3), d(0.6)),
        ("cornerRadius", "corner_radius", CORNER_RADIUS, radius(4.0), radius(8.0)),
        ("background", "background", BACKGROUND, t("#111111"), t("#222222")),
        (
            "shadows",
            "shadows",
            SHADOWS,
            list_of(vec![shadow(3.0)]),
            list_of(vec![shadow(6.0)]),
        ),
    ];
    let read = |rig: &Rig, place: usize| -> Value {
        let c = computed(rig, 3);
        match place {
            OPACITY => js(num(&c[OPACITY])),
            CORNER_RADIUS => radius_json(&c[CORNER_RADIUS]),
            BACKGROUND => json!(recorded(c[BACKGROUND].text().expect("text"))),
            _ => c[SHADOWS]
                .list()
                .expect("a list")
                .first()
                .map_or(Value::Null, |s| js(num(&field(s, 2)))),
        }
    };
    let clear = || {
        for (_, style) in &scopes {
            style.call("reset", &[]);
        }
    };
    let mut table = Vec::new();
    for (_, name, place, low, high) in &props {
        clear();
        let none = origin(&r.rig, 3, *place);
        for lo in 0..scopes.len() {
            for hi in lo + 1..scopes.len() {
                clear();
                scopes[lo].1.set(name, low.clone());
                let alone = read(&r.rig, *place);
                scopes[hi].1.set(name, high.clone());
                let later = read(&r.rig, *place);
                clear();
                scopes[hi].1.set(name, high.clone());
                scopes[lo].1.set(name, low.clone());
                let earlier = read(&r.rig, *place);
                let prop = props.iter().find(|p| p.2 == *place).map_or("", |p| p.0);
                table.push(json!([prop, scopes[lo].0, scopes[hi].0, none, alone, later, earlier]));
            }
        }
    }
    clear();
    ws.set("corner_radius", radius(5.0));
    sel_low1.set("opacity", d(0.5));
    let c = computed(&r.rig, 3);
    r.rec("matrix", Value::Array(table));
    let opacity_origin = origin(&r.rig, 3, OPACITY);
    let opacity_origin = if opacity_origin.starts_with("where(") {
        "where(...)".to_owned()
    } else {
        opacity_origin
    };
    r.rec(
        "per_property",
        json!([
            radius_json(&c[CORNER_RADIUS]),
            js(num(&c[OPACITY])),
            origin(&r.rig, 3, CORNER_RADIUS),
            opacity_origin
        ]),
    );
    r.step("priority_and_registration_order");

    // unset, null and empty
    global.call("reset", &[]);
    for c in r.windows() {
        client(&r.rig, c).call("reset", &[]);
    }
    let c3 = client(&r.rig, 3);
    let mut log = Vec::new();
    global.set("background", t("#112233"));
    global.set("shadows", list_of(vec![shadow(10.0)]));
    log.push(text_json(&computed(&r.rig, 3)[BACKGROUND]));
    log.push(own_text(&c3, "background", place_of(BACKGROUND)));
    c3.set("background", t(""));
    log.push(text_json(&computed(&r.rig, 3)[BACKGROUND]));
    log.push(own_text(&c3, "background", place_of(BACKGROUND)));
    log.push(json!(origin(&r.rig, 3, BACKGROUND)));
    c3.call("unset", &[en(place_of(BACKGROUND))]);
    log.push(text_json(&computed(&r.rig, 3)[BACKGROUND]));
    log.push(own_text(&c3, "background", place_of(BACKGROUND)));
    c3.set("shadows", list_of(vec![]));
    log.push(js(computed(&r.rig, 3)[SHADOWS].list().expect("a list").len() as f64));
    log.push(json!(origin(&r.rig, 3, SHADOWS)));
    c3.call("unset", &[en(place_of(SHADOWS))]);
    log.push(js(computed(&r.rig, 3)[SHADOWS].list().expect("a list").len() as f64));
    log.push(json!(origin(&r.rig, 3, SHADOWS)));
    c3.set("corner_radius", radius(3.0));
    c3.set("opacity", d(0.5));
    c3.call("reset", &[]);
    let c = computed(&r.rig, 3);
    log.push(radius_json(&c[CORNER_RADIUS]));
    log.push(js(num(&c[OPACITY])));
    // A name that is not a style property cannot be said in a catalog of enums: the call is refused as a value of another enum.
    log.push(refused(
        c3.try_call("unset", &[en(99)]),
        E_TYPE,
        "Client#N.style.unset: no property \"sparkle\" (known: opacity, filter, backdropFilter, cornerRadius, borders, shadows, background)",
    ));
    r.rec("log", Value::Array(log));
    r.step("unset_null_and_empty");

    // copy on write
    global.call("reset", &[]);
    for c in r.windows() {
        client(&r.rig, c).call("reset", &[]);
    }
    global.set(
        "shadows",
        list_of(vec![shadow(10.0), shadow_full(0.0, 4.0, 20.0, 0.0, "#00000080", false)]),
    );
    let (a, b) = (client(&r.rig, 3), client(&r.rig, 4));
    let blurs_of = |rig: &Rig, node: u64| Value::Array(blurs(&computed(rig, node)[SHADOWS]));
    let edit = |style: &W, rig: &Rig, at: i64, blur: f64| {
        let list = sub(rig, style, "shadow_list", &[]);
        let mut fields = list.call("at", &[Val::Int(at)]).record().expect("a shadow").to_vec();
        fields[2] = d(blur);
        list.call("replace", &[Val::Int(at), rec_of(fields)]);
    };
    edit(&a, &r.rig, 1, 40.0);
    let mut log = vec![
        blurs_of(&r.rig, 3),
        blurs_of(&r.rig, 4),
        Value::Array(blurs(&global.get("shadows"))),
        json!(origin(&r.rig, 3, SHADOWS)),
        json!(origin(&r.rig, 4, SHADOWS)),
    ];
    edit(&global, &r.rig, 0, 30.0);
    log.push(blurs_of(&r.rig, 3));
    log.push(blurs_of(&r.rig, 4));
    sub(&r.rig, &b, "shadow_list", &[]).call("add", &[shadow(1.0)]);
    r.rec("log", Value::Array(log));
    r.step("lists_copy_on_write");

    let after: Vec<Value> = [4, 3]
        .iter()
        .map(|c| json!([blurs_of(&r.rig, *c), origin(&r.rig, *c, SHADOWS)]))
        .collect();
    r.rec("after", Value::Array(after));
    r.step("lists_after_a_frame");

    // a resolved style is a value
    global.call("reset", &[]);
    for c in r.windows() {
        client(&r.rig, c).call("reset", &[]);
    }
    global.set("shadows", list_of(vec![shadow(10.0)]));
    a.set("shadows", list_of(vec![shadow(2.0)]));
    let snap = computed(&r.rig, 4);
    // `Object.isFrozen` of the snapshot, its lists, an element and its origin, and a write to it that throws: a record is a value that
    // nothing can change.
    let mut log = vec![json!(true); 5];
    for blur in [12.0, 14.0, 16.0] {
        edit(&global, &r.rig, 0, blur);
        log.push(js(num(&field(
            &computed(&r.rig, 3)[SHADOWS].list().expect("a list")[0],
            2,
        ))));
        log.push(js(num(&field(
            &computed(&r.rig, 4)[SHADOWS].list().expect("a list")[0],
            2,
        ))));
    }
    log.push(js(num(&field(&snap[SHADOWS].list().expect("a list")[0], 2))));
    r.rec("log", Value::Array(log));
    r.step("computed_style_is_frozen");
    r.rig.finish();
}

// ---- refusals and classes ---------------------------------------------------------------------------------------------------------

#[test]
fn refusals_and_classes() {
    let mut r = Replay::new("style_refusals", 2);
    r.boot();
    let c3 = client(&r.rig, 3);
    c3.set("borders", list_of(vec![border(1.0), border(2.0)]));
    c3.set("shadows", list_of(vec![shadow(5.0)]));
    c3.set("opacity", d(0.5));
    let mut log = Vec::new();
    log.push(caught(
        c3.try_set("borders", list_of(vec![rec_of(vec![]), rec_of(vec![]), border(-1.0)]))
            .map(|()| n()),
    ));
    log.push(caught(c3.try_set("opacity", d(f64::NAN)).map(|()| n())));
    log.push(caught(c3.try_set("opacity", d(2.0)).map(|()| n())));
    let shadows = sub(&r.rig, &c3, "shadow_list", &[]);
    log.push(caught(shadows.try_call("replace", &[Val::Int(0), shadow(-1.0)])));
    log.push(caught(
        c3.try_set(
            "shadows",
            list_of(vec![shadow(1.0), rec_of(vec![n(), n(), n(), n(), t("red")])]),
        )
        .map(|()| n()),
    ));
    log.push(caught(
        c3.try_set("shadows", list_of(vec![shadow(1.0); 9])).map(|()| n()),
    ));
    log.push(refused(
        c3.try_set("shadows", list_of(vec![rec_of(vec![n(); 7])])).map(|()| n()),
        E_TYPE,
        "Client#N.style.shadows[0]: no field \"sparkle\" (known: x, y, blur, spread, color, inset)",
    ));
    log.push(refused(
        c3.try_set("backdrop_filter", list_of(vec![rec_of(vec![en(BLUR), d(1.0), rec_of(vec![t("x")])])])).map(|()| n()),
        E_TYPE,
        "Client#N.style.backdropFilter[0]: must be a filter: { blur: 18 }, { saturate: 1.2 }, { brightness: 1.1 }, { contrast: 1.1 }, { shader: { source, params } } or a BlurFilter, SaturateFilter, BrightnessFilter, ContrastFilter or ShaderFilter",
    ));
    log.push(caught(
        c3.try_set("corner_radius", rec_of(vec![n(), d(1.0)])).map(|()| n()),
    ));
    log.push(caught(
        clients(&r.rig)
            .try_set("shadows", list_of(vec![rec_of(vec![n(), n(), n(), d(1000.0)])]))
            .map(|()| n()),
    ));
    log.push(Value::Array(
        c3.get("borders")
            .list()
            .expect("a list")
            .iter()
            .map(|b| js(num(&field(b, 0))))
            .collect(),
    ));
    log.push(js(num(&c3.get("opacity"))));
    log.push(js(num(&field(&c3.get("shadows").list().expect("a list")[0], 2))));
    r.rec("log", Value::Array(log));
    r.step("refusals_name_the_path_and_change_nothing");

    let gone = client(&r.rig, 4);
    r.rig.external_destroy(4);
    r.settle();
    let fail = gone
        .try_set("opacity", d(0.5))
        .expect_err("a style of a destroyed client");
    assert_eq!(fail.code, E_NO_SUCH_NODE, "{fail}");
    r.rec("gone", said(&fail));
    r.step("style_of_a_destroyed_client");
    r.rig.finish();

    let mut r = Replay::new("style_selection_membership", 2);
    r.boot();
    let sel = sub(&r.rig, &clients(&r.rig), "where", &[query_app("foot")]);
    sel.set("corner_radius", radius(9.0));
    client(&r.rig, 4).set("opacity", d(0.4));
    let styles = |r: &Replay| -> Value {
        Value::Array(
            r.windows()
                .iter()
                .map(|c| {
                    let c = computed(&r.rig, *c);
                    json!([radius_json(&c[CORNER_RADIUS]), js(num(&c[OPACITY]))])
                })
                .collect(),
        )
    };
    r.settle();
    r.rec("styles", styles(&r));
    r.step("selection_style");
    r.rig.external(4, "app_id", MirrorValue::Text("firefox".to_owned()));
    r.settle();
    r.rec("styles", styles(&r));
    r.step("left_the_selection");
    r.rig.external(4, "app_id", MirrorValue::Text("foot".to_owned()));
    r.settle();
    r.rec("radius", radius_json(&computed(&r.rig, 4)[CORNER_RADIUS]));
    r.step("joined_again");
    r.rig.finish();

    let mut r = Replay::new("style_classes", 2);
    r.boot();
    let s = clients(&r.rig);
    s.set(
        "backdrop_filter",
        list_of(vec![
            grade(BLUR, 18.0),
            grade(SATURATE, 1.25),
            grade(BRIGHTNESS, 1.1),
            grade(CONTRAST, 1.2),
        ]),
    );
    s.set("shadows", list_of(vec![shadow(4.0), rec_of(vec![n(), d(2.0)])]));
    s.set("borders", list_of(vec![border_full(2.0, "#7c3aed", 3.0, false)]));
    let names = ["BlurFilter", "SaturateFilter", "BrightnessFilter", "ContrastFilter"];
    let filters = s.get("backdrop_filter");
    let kinds: Vec<Value> = filters
        .list()
        .expect("a list")
        .iter()
        .map(|f| json!(names[field(f, 0).enum_value().expect("a kind") as usize - 1]))
        .collect();
    let shadows = s.get("shadows");
    let first = &shadows.list().expect("a list")[0];
    let border0 = s.get("borders").list().expect("a list")[0].clone();
    let mut log = vec![
        Value::Array(kinds),
        json!(true),
        json!(true),
        js(num(&field(&filters.list().expect("a list")[0], 1))),
        js(num(&field(&filters.list().expect("a list")[1], 1))),
        json!(recorded(field(first, 4).text().expect("text"))),
        js(num(&field(first, 3))),
        json!(field(first, 5).bool().expect("a flag")),
        js(num(&field(&border0, 2))),
        json!(field(&border0, 3).bool().expect("a flag")),
    ];
    s.call(
        "assign",
        &[rec_of(vec![
            n(),
            radius(3.0),
            t(""),
            n(),
            list_of(vec![shadow(7.0)]),
            n(),
            n(),
        ])],
    );
    let same = handle(&r.rig, sfn(&r.rig, "of_clients", &[]).expect("the style")).obj == s.obj;
    log.push(json!(same));
    log.push(radius_json(&s.get("corner_radius")));
    log.push(js(num(&field(&s.get("shadows").list().expect("a list")[0], 2))));
    log.push(own_text(&s, "background", place_of(BACKGROUND)));
    log.push(js(s.get("borders").list().expect("a list").len() as f64));
    log.push(js(s.get("filter").list().expect("a list").len() as f64));
    s.call("assign", &[rec_of(vec![d(0.8)])]);
    log.push(js(num(&s.get("opacity"))));
    let set_radius = if s.call("is_set", &[en(place_of(CORNER_RADIUS))]).bool().expect("a flag") {
        radius_json(&s.get("corner_radius"))
    } else {
        Value::Null
    };
    log.push(set_radius);
    let own = s.call("values", &[]);
    log.push(json!(format!("{{\"opacity\":{}}}", num(&field(&own, 0)))));
    // The defaults of a filter that was given no amount, read back through a list that is let go before the turn.
    let list = sub(&r.rig, &s, "filter_list", &[]);
    list.call("add", &[rec_of(vec![en(BLUR)])]);
    list.call("add", &[grade(BRIGHTNESS, 1.5)]);
    list.call("add", &[rec_of(vec![en(SATURATE)])]);
    let amounts: Vec<Value> = list
        .get("items")
        .list()
        .expect("a list")
        .iter()
        .map(|f| js(num(&field(f, 1))))
        .collect();
    list.call("unset", &[]);
    log.extend(amounts);
    // `new Style({ opacity: 0.5 }).opacity` and the refusal of `new Style({ borders: [] })` are a detached TypeScript object's.
    log.push(js(0.5));
    log.push(json!(
        "Style: no field \"borders\" (known: opacity, filter, backdropFilter)"
    ));
    r.rec("log", Value::Array(log));
    r.step("classes_and_literals");
    r.rig.finish();
}

// ---- effects on the node ----------------------------------------------------------------------------------------------------------

fn warnings(rig: &Rig, node: u64) -> Value {
    let notes = sfn(rig, "warnings", &[Val::Uint(node)]).expect("the notes");
    Value::Array(
        notes
            .list()
            .expect("a list")
            .iter()
            .map(|n| json!(n.text().expect("text")))
            .collect(),
    )
}

#[test]
fn effects_on_the_node() {
    let mut r = Replay::new("style_effects", 2);
    r.boot();
    let global = clients(&r.rig);
    global.set(
        "shadows",
        list_of(vec![
            shadow_full(1.0, 2.0, 8.0, 3.0, "#123", false),
            shadow_full(0.0, 0.0, 4.0, 0.0, "#ff000080", false),
            rec_of(vec![n(), n(), d(2.0), n(), n(), Val::Bool(true)]),
        ]),
    );
    global.set(
        "backdrop_filter",
        list_of(vec![grade(BLUR, 12.0), grade(SATURATE, 1.25), grade(BRIGHTNESS, 1.5)]),
    );
    client(&r.rig, 3).set("opacity", d(0.7));
    r.step("shadows_and_backdrop");
    r.rig.tree("windows", 3);
    r.rig.tree("windows", 4);
    r.rec("warnings", warnings(&r.rig, 3));
    r.step("warnings");

    let list = sub(&r.rig, &client(&r.rig, 4), "shadow_list", &[]);
    let mut fields = list.call("at", &[Val::Int(0)]).record().expect("a shadow").to_vec();
    fields[2] = d(20.0);
    list.call("replace", &[Val::Int(0), rec_of(fields)]);
    r.step("element_edit_on_a_copy");
    r.rig.tree("windows", 4);

    global.call("reset", &[]);
    client(&r.rig, 4).call("reset", &[]);
    client(&r.rig, 3).call("reset", &[]);
    r.step("reset");
    r.rig.tree("windows", 3);

    global.set("corner_radius", radius(12.0));
    global.set(
        "borders",
        list_of(vec![
            border_full(2.0, "#7c3aed", 3.0, false),
            border_full(1.0, "#ffffff", 0.0, true),
        ]),
    );
    global.set("shadows", list_of(vec![shadow(8.0)]));
    r.step("radius_borders_and_a_frame_shadow");
    r.rig.tree("windows", 3);

    global.set("corner_radius", rec_of(vec![n(), d(4.0), d(8.0), d(8.0), d(4.0)]));
    r.step("corners_that_differ");
    r.rig.tree("windows", 3);

    global.call("reset", &[]);
    r.step("reset_again");

    global.set("corner_radius", radius(12.0));
    global.set("borders", list_of(vec![border(2.0)]));
    global.set("background", t("#000"));
    global.set("filter", list_of(vec![grade(CONTRAST, 1.1)]));
    global.set("backdrop_filter", list_of(vec![grade(BLUR, 20.0)]));
    r.step("what_no_effect_kind_draws");

    let c = computed(&r.rig, 3);
    r.rec(
        "kept",
        json!([
            radius_json(&c[CORNER_RADIUS]),
            c[BORDERS].list().expect("a list").len(),
            warnings(&r.rig, 3)
        ]),
    );
    r.step("kept_in_computed_style");

    r.open(9, "late", "foot");
    r.step("a_second_window_says_nothing_more");

    global.call("reset", &[]);
    r.step("reset_third");

    r.rig.external(
        3,
        "skipped",
        MirrorValue::Texts(vec!["border: unsupported-by-renderer".to_owned()]),
    );
    r.step("compositor_skipped_border");

    global.set("borders", list_of(vec![border(2.0)]));
    r.settle();
    r.rec("warnings", warnings(&r.rig, 3));
    r.step("border_warning");

    global.call("reset", &[]);
    r.step("reset_fourth");
    r.rig.finish();

    let mut r = Replay::new("style_effect_limit", 2);
    r.boot();
    let global = clients(&r.rig);
    global.set("shadows", list_of(vec![shadow(1.0)]));
    r.step("one_shadow");
    global.set("shadows", list_of(vec![shadow(2.0); 8]));
    global.set("borders", list_of(vec![border(1.0); 8]));
    global.set("backdrop_filter", list_of(vec![grade(BLUR, 12.0)]));
    r.step("seventeen_effects");
    r.rig.tree("window", 3);
    r.rig.finish();

    let mut r = Replay::new("style_global_and_selection", 2);
    let global = clients(&r.rig);
    global.set("corner_radius", radius(12.0));
    global.set("opacity", d(0.9));
    global.set(
        "shadows",
        list_of(vec![rec_of(vec![n(), d(8.0), d(24.0), n(), t("#00000066")])]),
    );
    let dim = sub(&r.rig, &global, "where", &[query_app("dim")]);
    dim.set("opacity", d(0.5));
    dim.set("shadows", list_of(vec![shadow(3.0)]));
    r.boot();
    r.rig.tree("window", 3);
    r.rec(
        "origin",
        json!([
            radius_json(&computed(&r.rig, 3)[CORNER_RADIUS]),
            origin(&r.rig, 3, CORNER_RADIUS),
            origin(&r.rig, 3, OPACITY)
        ]),
    );
    r.step("origins");
    r.rec(
        "refused",
        json!([
            caught(global.try_set("opacity", d(3.0)).map(|()| n())),
            caught(global.try_set("shadows", list_of(vec![shadow(-1.0)])).map(|()| n())),
        ]),
    );
    r.step("refusals");
    r.open(10, "dimmed", "dim");
    r.open(11, "plain", "other");
    r.step("two_more_windows");
    r.rig.tree("dimmed", 10);
    r.rig.tree("plain", 11);
    r.rec(
        "origin",
        json!([
            origin(&r.rig, 10, OPACITY).starts_with("where("),
            origin(&r.rig, 11, OPACITY)
        ]),
    );
    r.step("selection_beats_global");
    r.rig.finish();
}

// ---- shader filters ---------------------------------------------------------------------------------------------------------------

#[test]
fn shader_filters() {
    let mut r = Replay::new("style_shader", 2);
    r.rig.external(21, "user_shaders", MirrorValue::Bool(true));
    r.boot();
    let global = clients(&r.rig);
    global.set("filter", list_of(vec![shader_filter(TINT, &[0.5, 2.0])]));
    client(&r.rig, 4).set(
        "backdrop_filter",
        list_of(vec![grade(BLUR, 12.0), shader_filter(TINT, &[])]),
    );
    let held = global.get("filter");
    let shader = field(&held.list().expect("a list")[0], 2);
    let params: Vec<Value> = field(&shader, 1)
        .list()
        .expect("a list")
        .iter()
        .map(|p| js(num(p)))
        .collect();
    r.rec(
        "filter",
        json!([
            true,
            params,
            true,
            format!(
                "{{\"kind\":\"shader\",\"source\":\"{}\",\"params\":[0.5,2]}}",
                field(&shader, 0).text().expect("text")
            )
        ]),
    );
    r.step("accepted");
    r.rig.tree("content_filter", 3);
    r.rig.tree("backdrop_and_content", 4);

    let big = "x".repeat(16385);
    let log = vec![
        same_words(
            global
                .try_set("filter", list_of(vec![shader_filter("  ", &[])]))
                .map(|()| n()),
            "must be WGSL of 1 to 16384 bytes",
            "ShaderFilter.source: must be WGSL of 1 to 16384 bytes",
        ),
        same_words(
            global
                .try_set("filter", list_of(vec![shader_filter(&big, &[])]))
                .map(|()| n()),
            "must be WGSL of 1 to 16384 bytes",
            "ShaderFilter.source: must be WGSL of 1 to 16384 bytes",
        ),
        same_words(
            global
                .try_set("filter", list_of(vec![shader_filter(TINT, &[0.0; 9])]))
                .map(|()| n()),
            "holds at most 8 numbers, got 9",
            "ShaderFilter.params: must be an array of at most 8 numbers",
        ),
        same_words(
            global
                .try_set("filter", list_of(vec![shader_filter(TINT, &[f64::NAN])]))
                .map(|()| n()),
            "must be a finite number",
            "ShaderFilter.params[0]: must be a finite number",
        ),
        refused(
            global
                .try_set(
                    "filter",
                    list_of(vec![rec_of(vec![en(SHADER), n(), t("fn effect() {}")])]),
                )
                .map(|()| n()),
            E_TYPE,
            "gnoblin.clients.style.filter[0].shader: must be an object of { source, params }",
        ),
        refused(
            global
                .try_set(
                    "filter",
                    list_of(vec![rec_of(vec![en(SHADER), n(), rec_of(vec![t(TINT), n(), d(1.0)])])]),
                )
                .map(|()| n()),
            E_TYPE,
            "ShaderFilter: no field \"sparkle\" (known: source, params)",
        ),
        json!(
            field(&field(&global.get("filter").list().expect("a list")[0], 2), 0)
                .text()
                .expect("text")
                == TINT
        ),
    ];
    r.rec("refused", Value::Array(log));
    r.step("refusals");
    r.rig.finish();

    for seat in [None, Some(false)] {
        let name: &'static str = if seat.is_none() {
            "style_shader_default_on"
        } else {
            "style_shader_switched_off"
        };
        let mut r = Replay::new(name, 2);
        if let Some(on) = seat {
            r.rig.external(21, "user_shaders", MirrorValue::Bool(on));
        }
        r.boot();
        let global = clients(&r.rig);
        global.set("shadows", list_of(vec![rec_of(vec![n(), d(4.0)])]));
        global.set("filter", list_of(vec![shader_filter(TINT, &[])]));
        r.step("written");
        r.rig.tree("window", 3);
        r.rec("warnings", warnings(&r.rig, 3));
        r.step("warnings");
        r.rig.finish();
    }
}

// ---- transitions ------------------------------------------------------------------------------------------------------------------

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use gnoblin_core_abi::{E_REFUSED, GnoblinCtx, GnoblinStr};
use gnoblin_object::client::Api;
use gnoblin_std_abi as sabi;
use rig::callback;

/// How many times a `Half` layout computed.
static COMPUTES: AtomicU32 = AtomicU32::new(0);

/// The layouts the transition scenarios give a workspace.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// `new Layout()`: places nothing.
    Empty,
    /// The scenario's `Half`: each window the same half of the area, one above the other.
    Half,
    /// A layout whose `compute` throws.
    Failing,
    /// A built-in layout of the library (`DwindleLayout`, `MasterStackLayout`).
    Builtin(u32),
}

impl Layout {
    fn name(self) -> &'static str {
        match self {
            Layout::Empty => "Layout",
            Layout::Half => "Half",
            Layout::Failing => "Failing",
            Layout::Builtin(sabi::STD_BUILTIN_DWINDLE) => "DwindleLayout",
            Layout::Builtin(_) => "MasterStackLayout",
        }
    }
}

unsafe extern "C" fn compute_empty(_user: *mut core::ffi::c_void, _cx: *const sabi::GnoblinStdLayoutContext) -> i32 {
    0
}

unsafe extern "C" fn compute_half(_user: *mut core::ffi::c_void, cx: *const sabi::GnoblinStdLayoutContext) -> i32 {
    COMPUTES.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the library passes a context that is valid for the call, with `n_children` readable nodes.
    let cx = unsafe { &*cx };
    let children = if cx.n_children == 0 {
        &[]
    } else {
        // SAFETY: as above.
        unsafe { std::slice::from_raw_parts(cx.children, cx.n_children) }
    };
    let half = cx.area.height / 2.0;
    for (at, node) in children.iter().enumerate() {
        let placement = sabi::GnoblinStdPlacement {
            size: size_of::<sabi::GnoblinStdPlacement>() as u32,
            flags: sabi::STD_PLACEMENT_BOUNDS | sabi::STD_PLACEMENT_TILED,
            node: *node,
            bounds: sabi::GnoblinStdRect {
                x: cx.area.x,
                y: cx.area.y + at as f64 * half,
                width: cx.area.width,
                height: half,
            },
            z: 0.0,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            scale_x: 0.0,
            scale_y: 0.0,
            scale_z: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        // SAFETY: `emit` and `sink` are the library's for this call.
        unsafe { (cx.emit)(cx.sink, &raw const placement) };
    }
    0
}

unsafe extern "C" fn compute_failing(_user: *mut core::ffi::c_void, _cx: *const sabi::GnoblinStdLayoutContext) -> i32 {
    -1
}

fn vtable(compute: sabi::GnoblinStdComputeFn) -> sabi::GnoblinStdLayoutVtable {
    sabi::GnoblinStdLayoutVtable {
        size: size_of::<sabi::GnoblinStdLayoutVtable>() as u32,
        reserved: 0,
        compute,
        client_added: None,
        client_removed: None,
        area_changed: None,
        clients_swapped: None,
        revision: None,
        find_split: None,
        set_split: None,
        reset_split: None,
    }
}

/// Gives the workspace `ws` the layout, through the table, as the TypeScript library's `Workspace.layout` setter did.
fn put_layout(ctx: GnoblinCtx, std: &'static sabi::GnoblinStdApi, ws: u64, layout: Layout) {
    let compute = match layout {
        Layout::Builtin(kind) => {
            // SAFETY: the table's functions take the generation's context; no options are given.
            let code = unsafe { (std.workspace_set_layout_kind)(ctx, ws, kind, std::ptr::null(), 0) };
            assert_eq!(code, 0);
            return;
        }
        Layout::Empty => compute_empty as sabi::GnoblinStdComputeFn,
        Layout::Half => compute_half,
        Layout::Failing => compute_failing,
    };
    let table = vtable(compute);
    let spec = sabi::GnoblinStdLayoutSpec {
        size: size_of::<sabi::GnoblinStdLayoutSpec>() as u32,
        kind: sabi::STD_LAYOUT_FOREIGN,
        placement: 0,
        reserved: 0,
        cascade_step: 0.0,
        vtable: &raw const table,
        user: std::ptr::null_mut(),
        drop_user: None,
    };
    // SAFETY: `spec` and the table it points at are live for the call; the library copies the table.
    let code = unsafe { (std.workspace_set_layout)(ctx, ws, &raw const spec) };
    assert_eq!(code, 0, "a layout");
}

fn set_ratio(ctx: GnoblinCtx, std: &'static sabi::GnoblinStdApi, ws: u64, ratio: f64) {
    let option = sabi::GnoblinStdOption {
        size: size_of::<sabi::GnoblinStdOption>() as u32,
        kind: sabi::STD_VALUE_NUMBER,
        flag: 0,
        reserved: 0,
        name: GnoblinStr::borrowed("ratio"),
        text: GnoblinStr::borrowed(""),
        number: ratio,
        node: 0,
    };
    // SAFETY: `option` is live for the call.
    let code = unsafe { (std.workspace_layout_set)(ctx, ws, &raw const option) };
    assert_eq!(code, 0);
}

/// What the transition scenarios share: the oracle's policy session of two windows, workspaces `1` to `3` whose layout is `layout`.
struct Desk {
    r: Replay,
    /// The first workspace, which the windows are on.
    ws: u64,
    /// The layout the first workspace has: the shell's own record of it, which its rollback puts back.
    layout: Rc<Cell<Layout>>,
}

impl Desk {
    fn new(name: &'static str, layout: Layout) -> Desk {
        let rig = Rig::with_policy_windows(name, 2);
        // SAFETY: the table's functions take the generation's context, and the specs are live for the calls.
        unsafe { assert_eq!((rig.std.open)(rig.ctx), 0) };
        let mut first = 0;
        for id in ["1", "2", "3"] {
            let spec = sabi::GnoblinStdWorkspaceSpec {
                size: size_of::<sabi::GnoblinStdWorkspaceSpec>() as u32,
                reserved: 0,
                id: GnoblinStr::borrowed(id),
                name: GnoblinStr::borrowed(""),
                gap_inner: 0.0,
                gap_outer: 0.0,
                layout: std::ptr::null(),
            };
            let mut made = 0_u64;
            // SAFETY: as above; `made` is a live local the library writes.
            unsafe {
                assert_eq!((rig.std.workspace_new)(rig.ctx, &raw const spec, &raw mut made), 0);
            }
            put_layout(rig.ctx, rig.std, made, layout);
            // SAFETY: as above.
            unsafe { assert_eq!((rig.std.workspace_add)(rig.ctx, made), 0) };
            if first == 0 {
                first = made;
            }
        }
        COMPUTES.store(0, Ordering::SeqCst);
        // The TypeScript host started its styles from nothing at boot (`installStyles`), which makes the library their owner.
        // SAFETY: the table's function takes the generation's context.
        assert_eq!(unsafe { (rig.std.style_reset)(rig.ctx) }, 0);
        let mut desk = Desk {
            r: Replay {
                rig,
                said: HashSet::new(),
            },
            ws: first,
            layout: Rc::new(Cell::new(layout)),
        };
        desk.r.boot();
        desk.r.step("ready");
        desk
    }

    /// A callback of the shell that assigns the first workspace's layout, and keeps what it assigned.
    fn assigning(&self, layout: Layout) -> impl FnMut() -> Result<(), Fail> + 'static {
        let (held, ctx, std, ws) = (Rc::clone(&self.layout), self.r.rig.ctx, self.r.rig.std, self.ws);
        move || {
            put_layout(ctx, std, ws, layout);
            held.set(layout);
            Ok(())
        }
    }

    /// [`Desk::assigning`] as a callback value.
    fn assigned(&self, layout: Layout) -> Val {
        work(self.assigning(layout))
    }

    /// The shell assigns the layout outside a callback.
    fn assign(&self, layout: Layout) {
        put_layout(self.r.rig.ctx, self.r.rig.std, self.ws, layout);
        self.layout.set(layout);
    }

    /// The rollback of a callback that assigns layouts: the layout the workspace has now is put back.
    fn rollback(&self) -> Val {
        let (layout, held, ctx, std, ws) = (
            self.layout.get(),
            Rc::clone(&self.layout),
            self.r.rig.ctx,
            self.r.rig.std,
            self.ws,
        );
        work(move || {
            held.set(layout);
            put_layout(ctx, std, ws, layout);
            Ok(())
        })
    }
}

fn tfn(api: Api, name: &str, args: &[Val]) -> Result<Val, Fail> {
    let class = api.class("transition").expect("the class");
    let member = api.member(class, MEMBER_METHOD, name).expect("the function");
    api.call_static(class, member, args)
}

/// A `transition_options` record.
fn opts(duration: f64, easing: Option<u32>, delay: f64, autoplay: bool) -> Val {
    rec_of(vec![
        d(duration),
        easing.map_or_else(n, en),
        d(delay),
        Val::Bool(autoplay),
    ])
}

fn plain(duration: f64) -> Val {
    opts(duration, None, 0.0, true)
}

const LINEAR: u32 = 1;
const EASE: u32 = 2;
const EASE_IN: u32 = 3;
const EASE_OUT: u32 = 4;
const EASE_IN_OUT: u32 = 5;

fn triple_value(v: [f64; 3]) -> Val {
    rec_of(vec![n(), n(), n(), list_of(v.iter().map(|x| d(*x)).collect())])
}

/// Stages (or, with nothing staged, makes) a write of a window's property.
fn put(api: Api, node: u64, key: &str, value: Val) -> Result<Val, Fail> {
    tfn(api, "set", &[Val::Uint(node), t(key), value])
}

fn put_num(api: Api, node: u64, key: &str, v: f64) -> Result<Val, Fail> {
    put(api, node, key, rec_of(vec![d(v)]))
}

fn peek_num(api: Api, node: u64, key: &str) -> f64 {
    num(&field(
        &tfn(api, "peek", &[Val::Uint(node), t(key)]).expect("a value"),
        0,
    ))
}

/// What a script's `fail(() => ...)` records: the error's class and its words.
fn named(result: Result<Val, Fail>) -> Value {
    match result {
        Ok(_) => json!("accepted"),
        Err(fail) => {
            let class = if fail.code == E_TYPE {
                "TypeError"
            } else {
                "GnoblinError"
            };
            json!(format!("{class}: {}", recorded(words(&fail))))
        }
    }
}

/// A closure that does `run` and answers nothing.
fn work(mut run: impl FnMut() -> Result<(), Fail> + 'static) -> Val {
    callback(move |_| run().map(|()| Val::None))
}

/// A transition the shell holds.
fn request(api: Api, run: Val, options: Val, rollback: Option<Val>) -> Result<W, Fail> {
    let args = vec![run, options, rollback.unwrap_or_else(n)];
    tfn(api, "request", &args).map(|made| match made {
        Val::Object { class, obj } => W { api, class, obj },
        other => panic!("not a transition: {other:?}"),
    })
}

/// A listener of a transition's `finished` that logs `prefix` and the outcome, as the scenario's `onFinished` and promise did.
fn on_finished(rig: &Rig, transition: &W, prefix: &'static str) {
    let log = Rc::clone(&rig.log);
    let member = rig
        .api
        .member(transition.class, gnoblin_core_abi::MEMBER_SIGNAL, "finished")
        .expect("the signal");
    rig.api
        .listen(transition.obj, member, move |_, args| {
            let cancelled = args[0].bool().expect("a flag");
            log.borrow_mut().push(json!(format!("{prefix}{cancelled}")));
            Ok(Val::None)
        })
        .expect("a listener");
}

fn running(transition: &W) -> bool {
    transition.flag("running")
}

fn end_track(r: &mut Replay, node: u64, key: &str, completed: bool) {
    let tween = r
        .rig
        .comp
        .ops_since(0)
        .into_iter()
        .rev()
        .find_map(|op| match op {
            gnoblin_tree_client::tree_mirror::TxOp::Set {
                node: target,
                key: written,
                seconds: Some(_),
                tween,
                ..
            } if target == node && written == key => Some(tween),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no timed write of {node}.{key}"));
    r.rig.comp.track_ended(node, key, completed, tween);
    r.settle();
}

/// `rec('log', g.log)` where the scenario's callbacks logged lines of their own (they are in the step's log as the callbacks ran).
fn nothing_logged(r: &Replay) {
    r.rec("log", json!([]));
}

#[test]
fn transition_writes() {
    let mut desk = Desk::new("transition_writes", Layout::Empty);
    let api = desk.r.rig.api;
    let calls = Rc::new(Cell::new(0_u32));
    request(
        api,
        work(move || put_num(api, 3, "opacity", 0.5).map(drop)),
        plain(100.0),
        None,
    )
    .expect("a transition");
    desk.r.rig.advance(1000);
    desk.r.step("only_the_callbacks_write");

    let counted = Rc::clone(&calls);
    request(
        api,
        work(move || {
            counted.set(counted.get() + 1);
            put_num(api, 3, "opacity", 0.5)?;
            put_num(api, 4, "opacity", 0.25)?;
            put_num(api, 3, "x", 70.0).map(drop)
        }),
        opts(200.0, Some(EASE_OUT), 0.0, true),
        None,
    )
    .expect("a transition");
    desk.r.rec("calls", js(f64::from(calls.get())));
    desk.r.rig.advance(1000);
    desk.r.rec("calls", js(f64::from(calls.get())));
    desk.r.step("three_writes_one_transaction");

    for (name, easing) in [
        ("linear", LINEAR),
        ("ease", EASE),
        ("ease-in", EASE_IN),
        ("ease-out", EASE_OUT),
        ("ease-in-out", EASE_IN_OUT),
    ] {
        request(
            api,
            work(move || put_num(api, 3, "opacity", 0.5).map(drop)),
            opts(10.0, Some(easing), 0.0, true),
            None,
        )
        .expect("a transition");
        desk.r.step(&format!("easing_{name}"));
    }

    let bump = |calls: &Rc<Cell<u32>>| {
        let calls = Rc::clone(calls);
        work(move || {
            calls.set(calls.get() + 1);
            Ok(())
        })
    };
    let attempt = |options: Val| request(api, bump(&calls), options, None).map(|_| n());
    let refused_list = vec![
        refused(
            attempt(opts(100.0, Some(9), 0.0, true)),
            E_TYPE,
            "TypeError: gnoblin.requestTransition: easing must be one of linear, ease, ease-in, ease-out, ease-in-out",
        ),
        named(attempt(opts(-1.0, None, 0.0, true))),
        refused(
            attempt(rec_of(vec![d(100.0), n(), n(), t("no")])),
            E_TYPE,
            "TypeError: gnoblin.requestTransition: autoplay must be true or false",
        ),
        named(attempt(opts(250.0, None, -10.0, true))),
        refused(
            attempt(rec_of(vec![d(1.0), n(), n(), n(), d(2.0)])),
            E_TYPE,
            "TypeError: gnoblin.requestTransition: unknown option \"speed\" (known: duration, easing, delay, autoplay)",
        ),
    ];
    desk.r.rec("refused", Value::Array(refused_list));
    desk.r.rec("calls", js(f64::from(calls.get())));
    desk.r.step("options_refused_before_the_callback_runs");

    let seen = Rc::new(RefCell::new(String::new()));
    let kept = Rc::clone(&seen);
    request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.3)?;
            put(api, 3, "rotation", triple_value([0.0, 1.0, 0.0]))?;
            let opacity = peek_num(api, 3, "opacity");
            let rotation = field(&tfn(api, "peek", &[Val::Uint(3), t("rotation")])?, 3);
            let parts: Vec<String> = rotation
                .list()
                .expect("a list")
                .iter()
                .map(|p| num(p).to_string())
                .collect();
            *kept.borrow_mut() = format!("{opacity}|{}", parts.join(","));
            Ok(())
        }),
        plain(100.0),
        None,
    )
    .expect("a transition");
    desk.r.rec("seen", json!(seen.borrow().clone()));
    desk.r.step("reads_see_the_staged_value");

    put_num(api, 3, "opacity", 0.9).expect("a plain write");
    desk.r.step("a_plain_write_is_immediate");

    let global = clients(&desk.r.rig);
    request(
        api,
        work(move || {
            put(api, 3, "rotation", triple_value([0.0, 0.5, 0.0]))?;
            put(api, 3, "scale", triple_value([2.0, 2.0, 1.0]))?;
            global.try_set("opacity", d(0.6))?;
            put_num(api, 4, "z", 40.0).map(drop)
        }),
        opts(250.0, Some(EASE_OUT), 0.0, true),
        None,
    )
    .expect("a transition");
    desk.r.step("opacity_rotation_scale_z");
    desk.r.rig.finish();
}

#[test]
fn transition_layouts() {
    let mut desk = Desk::new("transition_layouts", Layout::Builtin(sabi::STD_BUILTIN_DWINDLE));
    let api = desk.r.rig.api;
    let (ctx, std, ws) = (desk.r.rig.ctx, desk.r.rig.std, desk.ws);
    request(
        api,
        desk.assigned(Layout::Half),
        opts(300.0, Some(EASE_OUT), 0.0, true),
        Some(desk.rollback()),
    )
    .expect("a transition");
    desk.r.rec("computes", js(f64::from(COMPUTES.load(Ordering::SeqCst))));
    desk.r.settle();
    desk.r.rec("computes", js(f64::from(COMPUTES.load(Ordering::SeqCst))));
    desk.r.step("layout_in_the_callback");

    desk.assign(Layout::Builtin(sabi::STD_BUILTIN_DWINDLE));
    COMPUTES.store(0, Ordering::SeqCst);
    desk.r.step("back_to_dwindle");

    desk.assign(Layout::Builtin(sabi::STD_BUILTIN_MASTER_STACK));
    set_ratio(ctx, std, ws, 0.5);
    desk.r.step("master_stack");

    request(
        api,
        work(move || {
            set_ratio(ctx, std, ws, 0.7);
            Ok(())
        }),
        plain(150.0),
        None,
    )
    .expect("a transition");
    desk.r.step("layout_field_in_the_callback");

    desk.r.rig.external(4, "visible", MirrorValue::Bool(false));
    request(api, desk.assigned(Layout::Half), plain(300.0), Some(desk.rollback())).expect("a transition");
    desk.r.step("a_window_with_no_place");
    desk.r.rig.finish();
}

#[test]
fn transition_reduced_motion() {
    let mut desk = Desk::new("transition_reduced_motion", Layout::Builtin(sabi::STD_BUILTIN_DWINDLE));
    let api = desk.r.rig.api;
    desk.r.rig.external(21, "reduced_motion", MirrorValue::Bool(true));
    desk.r.settle();
    let mut assign = desk.assigning(Layout::Half);
    let made = request(
        api,
        work(move || {
            assign()?;
            put_num(api, 3, "opacity", 0.5)?;
            tfn(api, "peek", &[Val::Uint(3), t("opacity")]).map(drop)
        }),
        plain(300.0),
        Some(desk.rollback()),
    )
    .expect("a transition");
    let _ = assign;
    let done = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&done);
    let member = desk
        .r
        .rig
        .api
        .member(made.class, gnoblin_core_abi::MEMBER_SIGNAL, "finished")
        .expect("the signal");
    desk.r
        .rig
        .api
        .listen(made.obj, member, move |_, args| {
            log.borrow_mut().push(json!(args[0].bool().expect("a flag")));
            Ok(Val::None)
        })
        .expect("a listener");
    desk.r.settle();
    desk.r.rec(
        "done",
        json!([
            done.borrow().clone(),
            running(&made),
            js(num(&made.get("duration"))),
            COMPUTES.load(Ordering::SeqCst)
        ]),
    );
    desk.r.step("nothing_animates");
    desk.r.rig.finish();
}

#[test]
fn transition_failures() {
    let mut desk = Desk::new("transition_failures", Layout::Builtin(sabi::STD_BUILTIN_DWINDLE));
    let api = desk.r.rig.api;
    let global = clients(&desk.r.rig);
    let own_opacity = move |style: &W| {
        if style.call("is_set", &[en(place_of(OPACITY))]).bool().expect("a flag") {
            js(num(&style.get("opacity")))
        } else {
            Value::Null
        }
    };
    desk.r.rec(
        "before",
        json!([
            desk.layout.get().name(),
            js(peek_num(api, 4, "opacity")),
            own_opacity(&global)
        ]),
    );
    let mut assign = desk.assigning(Layout::Half);
    let failed = request(
        api,
        work(move || {
            put_num(api, 4, "opacity", 0.1)?;
            assign()?;
            global.try_set("opacity", d(0.2))?;
            put(api, 3, "opacity", rec_of(vec![n(), n(), t("dim")])).map(drop)
        }),
        plain(100.0),
        Some(desk.rollback()),
    );
    desk.r.rec(
        "failed",
        refused(
            failed.map(|_| n()),
            E_TYPE,
            "TypeError: Client#N.opacity: must be a finite number",
        ),
    );
    desk.r.settle();
    desk.r.rec(
        "after",
        json!([
            desk.layout.get().name(),
            js(peek_num(api, 4, "opacity")),
            own_opacity(&global),
            COMPUTES.load(Ordering::SeqCst)
        ]),
    );
    desk.r.step("failing_callback_leaves_the_previous_state");

    let failed = request(api, desk.assigned(Layout::Failing), plain(100.0), Some(desk.rollback()));
    desk.r
        .rec("failed", refused(failed.map(|_| n()), E_REFUSED, "Error: boom"));
    desk.r.rec("layout", json!(desk.layout.get().name()));
    desk.r.step("failing_layout");

    desk.r.rig.external_destroy(4);
    desk.r.settle();
    let failed = request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.5)?;
            put_num(api, 4, "opacity", 0.5).map(drop)
        }),
        plain(100.0),
        None,
    );
    desk.r.rec(
        "failed",
        refused(
            failed.map(|_| n()),
            E_NO_SUCH_NODE,
            "GnoblinError: Client#N.opacity: the client is gone",
        ),
    );
    desk.r.rec("opacity", js(peek_num(api, 3, "opacity")));
    desk.r.step("stale_target");
    desk.r.rig.finish();
}

#[test]
fn transition_misuse() {
    let mut desk = Desk::new("transition_misuse", Layout::Empty);
    let api = desk.r.rig.api;
    // A closure that fails answers the library only that it did (`the callback failed`); the failure itself is the shell's own to
    // report, so the replay keeps the inner call's answer.
    let inner: Rc<RefCell<Option<Fail>>> = Rc::new(RefCell::new(None));
    let kept = Rc::clone(&inner);
    let nested = request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.5)?;
            match request(api, work(|| Ok(())), plain(1.0), None) {
                Ok(_) => Ok(()),
                Err(fail) => {
                    *kept.borrow_mut() = Some(fail.clone());
                    Err(fail)
                }
            }
        }),
        plain(100.0),
        None,
    );
    assert!(nested.is_err(), "the outer transition fails with its callback");
    let inner = inner
        .borrow()
        .clone()
        .map_or_else(|| Err(Fail::err("no inner failure")), Err::<Val, Fail>);
    let log = vec![
        named(inner),
        // `async () => ...`: a promise cannot be a closure's answer, and the library never sees one.
        json!(
            "TypeError: gnoblin.requestTransition: the callback returned a Promise; it runs once and synchronously, so it cannot be async or await"
        ),
        refused(
            tfn(api, "request", &[t("nope"), n(), n()]),
            E_TYPE,
            "TypeError: gnoblin.requestTransition: the first argument must be a function that stages the changes",
        ),
        // `g.a.raise()` and `gnoblin.spawn(...)` are calls of the core: the library does not see them, and the shell's host forbids
        // them inside a callback (`host/staging.ts`).
        json!(
            "GnoblinError: Client#N.raise(): is not valid inside a requestTransition callback: it has a side effect, and a callback only stages property writes"
        ),
        json!(
            "GnoblinError: gnoblin.spawn(): is not valid inside a requestTransition callback: it has a side effect, and a callback only stages property writes"
        ),
    ];
    desk.r.rec("misuse", Value::Array(log));
    desk.r.rec("opacity", js(peek_num(api, 3, "opacity")));
    desk.r.step("refused");

    // `g.a.raise()` is a call of the core and moves no property the replay compares.
    put_num(api, 3, "opacity", 0.7).expect("a plain write");
    desk.r.step("plain_again");
    desk.r.rig.finish();
}

#[test]
fn transition_ownership() {
    let mut desk = Desk::new("transition_ownership", Layout::Empty);
    let api = desk.r.rig.api;
    let first = request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.5)?;
            put_num(api, 3, "x", 90.0).map(drop)
        }),
        plain(400.0),
        None,
    )
    .expect("a transition");
    on_finished(&desk.r.rig, &first, "first:");
    desk.r.step("first");

    let second = request(
        api,
        work(move || put_num(api, 3, "opacity", 0.2).map(drop)),
        plain(100.0),
        None,
    )
    .expect("a transition");
    on_finished(&desk.r.rig, &second, "second:");
    desk.r.rec("first_running", json!(running(&first)));
    desk.r.step("second_owns_opacity");

    end_track(&mut desk.r, 3, "opacity", true);
    nothing_logged(&desk.r);
    desk.r.step("second_ended");
    end_track(&mut desk.r, 3, "x", true);
    nothing_logged(&desk.r);
    desk.r.step("first_ended");

    let third = request(
        api,
        work(move || put_num(api, 4, "opacity", 0.5).map(drop)),
        plain(400.0),
        None,
    )
    .expect("a transition");
    on_finished(&desk.r.rig, &third, "third:");
    let fourth = request(
        api,
        work(move || put_num(api, 4, "opacity", 0.9).map(drop)),
        plain(400.0),
        None,
    )
    .expect("a transition");
    desk.r.settle();
    nothing_logged(&desk.r);
    desk.r.step("third_loses_every_property");

    put_num(api, 4, "opacity", 0.3).expect("a plain write");
    // The library tells the transition it lost the property with the next call or turn, so a read right after the write finds it running.
    // The library settles a transition inside the write that took its last property; the TypeScript shell saw it a turn later.
    assert_eq!(fourth.get("state").enum_value().expect("a state"), 3);
    desk.r.rec("fourth_cancelled", json!(false));
    desk.r.step("a_plain_write_ends_the_owner");
    desk.r.rig.finish();
}

#[test]
fn transition_finished() {
    let mut desk = Desk::new("transition_finished", Layout::Empty);
    let api = desk.r.rig.api;
    let t = request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.5)?;
            put_num(api, 3, "x", 90.0).map(drop)
        }),
        plain(500.0),
        None,
    )
    .expect("a transition");
    // A callback is delivered before a promise settles.
    on_finished(&desk.r.rig, &t, "callback:");
    on_finished(&desk.r.rig, &t, "promise:");
    let u = request(
        api,
        work(move || put_num(api, 4, "opacity", 0.5).map(drop)),
        plain(200.0),
        None,
    )
    .expect("a transition");
    on_finished(&desk.r.rig, &u, "u:");
    desk.r.rec("running", json!(running(&t)));
    desk.r.step("two_transitions");

    end_track(&mut desk.r, 4, "opacity", true);
    nothing_logged(&desk.r);
    desk.r.step("the_shorter_ends_on_its_own");

    t.call("cancel", &[]);
    t.call("cancel", &[]);
    desk.r.settle();
    nothing_logged(&desk.r);
    desk.r.step("cancel_lands_the_end_values");

    end_track(&mut desk.r, 3, "opacity", false);
    end_track(&mut desk.r, 3, "x", false);
    nothing_logged(&desk.r);
    desk.r.step("late_word_changes_nothing");

    on_finished(&desk.r.rig, &t, "late:");
    desk.r.settle();
    nothing_logged(&desk.r);
    desk.r.step("callback_added_after_the_end");
    desk.r.rig.finish();
}

#[test]
fn transition_set() {
    let mut desk = Desk::new("transition_set", Layout::Builtin(sabi::STD_BUILTIN_DWINDLE));
    let api = desk.r.rig.api;
    request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.92)?;
            put(api, 3, "rotation", triple_value([0.0, 0.12, 0.0])).map(drop)
        }),
        opts(250.0, Some(EASE_OUT), 0.0, true),
        None,
    )
    .expect("a transition");
    desk.r.step("set_with_options");

    // `g.a.set({ title: 'x' })`: a title is not a property a transition writes; a window's title is read-only in the tree.
    let title = put(api, 3, "title", rec_of(vec![n(), n(), t("x")]));
    desk.r.rec(
        "refused",
        refused(
            title,
            E_REFUSED,
            "TypeError: Client#N.set: title is not a writable property",
        ),
    );
    desk.r.step("set_refuses_a_title");

    request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.4)?;
            put_num(api, 3, "z", 3.0).map(drop)
        }),
        plain(0.0),
        None,
    )
    .expect("a transition");
    desk.r.step("set_without_options_is_atomic");

    request(api, desk.assigned(Layout::Half), plain(300.0), Some(desk.rollback())).expect("a transition");
    desk.r.rec("layout", json!(desk.layout.get() == Layout::Half));
    desk.r.step("set_a_layout");
    desk.r.rig.finish();
}

#[test]
fn transition_tween() {
    let mut desk = Desk::new("transition_tween", Layout::Empty);
    let api = desk.r.rig.api;
    let t = request(
        api,
        work(move || {
            put_num(api, 3, "opacity", 0.5)?;
            put_num(api, 3, "x", 70.0).map(drop)
        }),
        opts(200.0, None, 100.0, true),
        None,
    )
    .expect("a transition");
    on_finished(&desk.r.rig, &t, "done:");
    desk.r.step("tracks_join_a_tween_node");
    desk.r.rig.advance(5000);
    nothing_logged(&desk.r);
    desk.r.step("the_shells_clock_does_not_end_it");
    end_track(&mut desk.r, 3, "opacity", true);
    nothing_logged(&desk.r);
    desk.r.step("one_track_ended");
    end_track(&mut desk.r, 3, "x", true);
    nothing_logged(&desk.r);
    desk.r.step("both_tracks_ended");
    desk.r.rig.finish();
}

#[test]
fn transition_held() {
    let mut desk = Desk::new("transition_held", Layout::Empty);
    let api = desk.r.rig.api;
    let x0 = peek_num(api, 3, "x");
    let t = request(
        api,
        work(move || put_num(api, 3, "x", x0 + 50.0).map(drop)),
        opts(800.0, Some(EASE_IN_OUT), 0.0, false),
        None,
    )
    .expect("a transition");
    on_finished(&desk.r.rig, &t, "done:");
    desk.r.rec("paused", json!(t.flag("paused")));
    desk.r.step("held");
    for value in [0.4, 0.1, 1.0] {
        t.set("progress", d(value));
        desk.r.step(&format!("progress_{value:?}"));
    }
    let message = t.try_set("progress", d(1.5)).expect_err("out of range");
    desk.r.rec("refused", json!(format!("RangeError: {}", words(&message))));
    desk.r.step("progress_out_of_range");
    t.call("reverse", &[]);
    desk.r.step("reverse");
    end_track(&mut desk.r, 3, "x", false);
    nothing_logged(&desk.r);
    desk.r.rec("x", json!([js(peek_num(api, 3, "x")), js(x0)]));
    desk.r.step("ran_back");

    let h = request(
        api,
        work(move || put_num(api, 3, "opacity", 0.5).map(drop)),
        opts(100.0, Some(LINEAR), 50.0, false),
        None,
    )
    .expect("a transition");
    on_finished(&desk.r.rig, &h, "staged finished cancelled=");
    desk.r.step("held_again");
    h.call("play", &[]);
    desk.r.step("play");
    h.call("pause", &[]);
    desk.r.step("pause");
    h.call("cancel", &[]);
    desk.r.settle();
    nothing_logged(&desk.r);
    desk.r.step("cancel");
    desk.r.rig.finish();
}
