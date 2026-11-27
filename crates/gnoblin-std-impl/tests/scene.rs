#![allow(unsafe_code)]
//! The scene of `libgnoblin-std`, held to what the TypeScript scene did (`data/js/gnoblin-std/scene/*`, `scene.ts`, `scene_mirror.ts`,
//! `display/camera2d.ts`, `data/js/gnoblin-core/math/*` and `properties.ts`).
//!
//! Each test replays a scenario of `crates/gnoblin-js/tests/oracle/scene.rs` as a shell written over the object protocol, in the same
//! order, and `rig.rs` holds the native run against the fixture at every step: the log, and every node the shell made, whole. The math
//! tables are rounded to ten decimals and compared within a tolerance, as the fixtures' README says.
//!
//! Where the scenario asked something no typed catalog can carry, the replay says so where it records the fixture's answer (the
//! module's `refused`): a text where a vector is declared, an object with a key that is not a field, `null` for a record. The
//! protocol refuses those before the library sees them, in its own words, and the test asserts that it does.

#[path = "ui/rig.rs"]
mod rig;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gnoblin_core_abi::{E_STALE, E_TYPE, GnoblinStr, MEMBER_METHOD, MEMBER_PROPERTY};
use gnoblin_object::client::Api;
use gnoblin_object::{Fail, Obj, Val};
use gnoblin_std_abi::GnoblinStdWorkspaceSpec;
use gnoblin_testkit::oracle::{Value, json};
use gnoblin_tree_client::tree_mirror::MirrorValue;
use rig::{Rig, W, b, callback, d, en, js, recorded, t};

// ---- values --------------------------------------------------------------------------------------------------------------------

/// A `vec3` record.
fn v3(x: f64, y: f64, z: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![d(x), d(y), d(z)],
    }
}

fn quaternion(x: f64, y: f64, z: f64, w: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![d(x), d(y), d(z), d(w)],
    }
}

/// The three numbers of a `vec3` record.
fn triple(value: &Val) -> [f64; 3] {
    let fields = value.record().expect("a vec3");
    [0, 1, 2].map(|at| fields[at].double().expect("a number"))
}

/// A pose from a position, angles and a scale.
fn pose(position: [f64; 3], rotation: [f64; 3], scale: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![
            v3(position[0], position[1], position[2]),
            v3(rotation[0], rotation[1], rotation[2]),
            d(scale),
        ],
    }
}

/// A `transform_parts` record.
fn parts(
    position: Option<Val>,
    rotation: Option<Val>,
    orientation: Option<Val>,
    scale: Option<Val>,
    uniform: Option<f64>,
) -> Val {
    Val::Record {
        class: 0,
        fields: vec![
            position.unwrap_or(Val::None),
            rotation.unwrap_or(Val::None),
            orientation.unwrap_or(Val::None),
            scale.unwrap_or(Val::None),
            uniform.map_or(Val::None, d),
        ],
    }
}

fn list(numbers: &[f64]) -> Val {
    Val::List(numbers.iter().map(|number| d(*number)).collect())
}

/// A number the way a script's table shows it after `Math.round(x * 1e10) / 1e10`, and a number that is not one as `null`.
fn rounded(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    js((x * 1e10 + 0.5).floor() / 1e10)
}

fn vector_json(value: &Val) -> Value {
    Value::Array(triple(value).iter().map(|x| rounded(*x)).collect())
}

fn numbers_json(numbers: &[f64]) -> Value {
    Value::Array(numbers.iter().map(|x| rounded(*x)).collect())
}

/// The words of a failure of a call: the protocol puts the symbol of the code in front of them (`E_TYPE: `).
fn words(fail: &Fail) -> &str {
    match fail.message.split_once(": ") {
        Some((code, rest)) if code.starts_with("E_") && code.chars().all(|c| c.is_ascii_uppercase() || c == '_') => {
            rest
        }
        _ => &fail.message,
    }
}

/// What a script's `attempt` records of a call that threw.
fn threw(fail: &Fail) -> Value {
    json!(format!("throws: {}", words(fail)))
}

// ---- the library's functions ---------------------------------------------------------------------------------------------------

fn geometry(rig: &Rig, name: &str, args: &[Val]) -> Val {
    try_geometry(rig, name, args).unwrap_or_else(|fail| panic!("geometry.{name}: {fail}"))
}

fn try_geometry(rig: &Rig, name: &str, args: &[Val]) -> Result<Val, Fail> {
    let class = rig.api.class("geometry").expect("the class");
    let member = rig.api.member(class, MEMBER_METHOD, name).expect("the function");
    rig.api.call_static(class, member, args)
}

/// `geometry.<name>` with a place and a list, as a script's `attempt` shows what it answered or why it threw.
fn attempt(result: Result<Val, Fail>) -> Value {
    match result {
        Ok(value) => vector_json(&value),
        Err(fail) => threw(&fail),
    }
}

/// The answer the fixture holds for a call no typed catalog can make: the call is made as far as it can be (`wrong`, with the wrong
/// kind of value) and the protocol must refuse it as a value of the wrong kind; the fixture's answer is what is recorded.
fn refused(rig: &Rig, name: &str, wrong: &[Val], fixture: &str) -> Value {
    let fail = try_geometry(rig, name, wrong).expect_err("a value of the wrong kind is refused");
    assert_eq!(fail.code, E_TYPE, "geometry.{name}: {fail}");
    json!(fixture)
}

// ---- the math -------------------------------------------------------------------------------------------------------------------

fn vector_table(rig: &Rig) -> Value {
    let a = v3(1.0, 2.0, 3.0);
    let b_ = v3(-4.0, 5.0, 0.5);
    let z = v3(0.0, 0.0, 0.0);
    let g = |name: &str, args: &[Val]| geometry(rig, name, args);
    json!({
        "add": vector_json(&g("add", &[a.clone(), b_.clone()])),
        "subtract": vector_json(&g("subtract", &[a.clone(), b_.clone()])),
        "scale": vector_json(&g("scale", &[a.clone(), d(2.5)])),
        "dot": rounded(g("dot", &[a.clone(), b_.clone()]).double().expect("a number")),
        "cross": vector_json(&g("cross", &[a.clone(), b_.clone()])),
        "length": rounded(g("length", std::slice::from_ref(&a)).double().expect("a number")),
        "normalize": vector_json(&g("normalize", std::slice::from_ref(&a))),
        "zero": vector_json(&g("normalize", std::slice::from_ref(&z))),
        "distance": rounded(g("distance", &[a.clone(), b_.clone()]).double().expect("a number")),
        "from": vector_json(&g("vec3_of", &[list(&[7.0, 8.0, 9.0])])),
        "zeroes": vector_json(&z),
        "clone": vector_json(&a),
        "array": triple(&a).iter().map(|x| js(*x)).collect::<Vec<_>>(),
        // A list of two has a zero for the coordinate it lacks; a vector of NaN is a record the library does not check.
        "bad_from": vector_json(&g("vec3_of", &[list(&[1.0, 2.0])])),
        "bad_new": vector_json(&v3(f64::NAN, 0.0, 0.0)),
        "parallel": vector_json(&g("cross", &[v3(0.0, 0.0, 2.0), v3(0.0, 0.0, 5.0)])),
    })
}

fn ray(origin: [f64; 3], direction: [f64; 3]) -> Val {
    Val::Record {
        class: 0,
        fields: vec![
            v3(origin[0], origin[1], origin[2]),
            v3(direction[0], direction[1], direction[2]),
        ],
    }
}

fn plane(point: [f64; 3], normal: [f64; 3]) -> Val {
    Val::Record {
        class: 0,
        fields: vec![v3(point[0], point[1], point[2]), v3(normal[0], normal[1], normal[2])],
    }
}

/// Where a ray meets a plane as a script's table shows it: the point, or `null` for a miss.
fn hit_json(answer: &Val) -> Value {
    let fields = answer.record().expect("a ray hit");
    if fields[0].bool().expect("a flag") {
        vector_json(&fields[2])
    } else {
        assert!(fields[2].is_none(), "a miss has no point");
        Value::Null
    }
}

fn ray_table(rig: &Rig) -> Value {
    let wall = plane([0.0, 0.0, -2.0], [0.0, 0.0, 1.0]);
    let hit = |origin: [f64; 3], direction: [f64; 3]| {
        hit_json(&geometry(rig, "intersect", &[wall.clone(), ray(origin, direction)]))
    };
    json!({
        "at": vector_json(&geometry(rig, "ray_at", &[ray([1.0, 1.0, 1.0], [0.0, 0.0, -1.0]), d(3.0)])),
        "straight": hit([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        "slanted": hit([1.0, 2.0, 0.0], [0.5, -0.5, -1.0]),
        "parallel": hit([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        "behind": hit([0.0, 0.0, -5.0], [0.0, 0.0, -1.0]),
        "away": hit([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        "on_plane": hit([3.0, 4.0, -2.0], [1.0, 0.0, 0.0]),
        "unnormalised": hit([0.0, 0.0, 0.0], [0.0, 0.0, -4.0]),
        "tilted": hit_json(&geometry(
            rig,
            "intersect",
            &[plane([1.0, 1.0, 1.0], [1.0, 1.0, 1.0]), ray([0.0, 0.0, 0.0], [1.0, 1.0, 1.0])],
        )),
    })
}

fn pose_table(rig: &Rig) -> Value {
    use std::f64::consts::{FRAC_PI_2, PI};
    let rotations = [
        [0.0, 0.0, 0.0],
        [0.1, 0.2, 0.3],
        [FRAC_PI_2, 0.0, 0.0],
        [0.0, FRAC_PI_2, 0.0],
        [0.0, -FRAC_PI_2, 0.3],
        [3.0, 2.0, 1.0],
        [0.0, 0.0, PI],
        [-0.7, 1.2, -2.5],
    ];
    let mut out = Vec::new();
    for rotation in rotations {
        for scale in [1.0, 2.0, 0.5] {
            let placed = pose([1.0, -2.0, 3.0], rotation, scale);
            let normal = geometry(rig, "pose_normalise", std::slice::from_ref(&placed));
            let direction = |axis: [f64; 3]| {
                vector_json(&geometry(
                    rig,
                    "transform_direction",
                    &[placed.clone(), v3(axis[0], axis[1], axis[2])],
                ))
            };
            let facing = geometry(rig, "pose_plane", std::slice::from_ref(&placed));
            let facing = facing.record().expect("a plane");
            out.push(json!({
                "rotation": numbers_json(&rotation),
                "scale": js(scale),
                "euler": vector_json(&normal.record().expect("a pose")[1]),
                "x": direction([1.0, 0.0, 0.0]),
                "y": direction([0.0, 1.0, 0.0]),
                "z": direction([0.0, 0.0, 1.0]),
                "point": vector_json(&geometry(rig, "transform_point", &[placed.clone(), v3(0.5, 0.25, -1.0)])),
                "plane": [vector_json(&facing[0]), vector_json(&facing[1])],
            }));
        }
    }
    Value::Array(out)
}

/// A pose as a script's `t()` shows it.
fn pose_json(value: &Val) -> Value {
    let fields = value.record().expect("a pose");
    json!({
        "position": vector_json(&fields[0]),
        "rotation": vector_json(&fields[1]),
        "scale": rounded(fields[2].double().expect("a scale")),
    })
}

fn compose_table(rig: &Rig) -> Value {
    let p = pose([1.0, 2.0, 3.0], [0.3, -0.4, 0.5], 2.0);
    let q = pose([-1.0, 0.5, 2.0], [1.0, 0.2, -0.6], 0.5);
    let probe = v3(0.3, -0.2, 0.9);
    let g = |name: &str, args: &[Val]| geometry(rig, name, args);
    // A pose a script makes reports its rotation as the angles of its matrix; `pose_normalise` is that.
    let seen = |value: &Val| pose_json(&g("pose_normalise", std::slice::from_ref(value)));
    let inverse = g("pose_inverse", std::slice::from_ref(&p));
    let through = g(
        "transform_point",
        &[inverse.clone(), g("transform_point", &[p.clone(), probe])],
    );
    json!({
        "product": pose_json(&g("pose_multiply", &[p.clone(), q.clone()])),
        "reverse": pose_json(&g("pose_multiply", &[q.clone(), p.clone()])),
        "inverse": pose_json(&inverse),
        "round_trip": pose_json(&g("pose_multiply", &[p.clone(), inverse.clone()])),
        "inverse_point": vector_json(&through),
        "identity": pose_json(&g("pose_identity", &[])),
        "translation": pose_json(&g("pose_translation", &[d(4.0), d(5.0), d(6.0)])),
        "clone": seen(&p),
        "mutated_clone": seen(&pose([9.0, 9.0, 9.0], [0.1, 0.2, 0.3], 3.0)),
        "original": seen(&p),
    })
}

// The fixture's quaternion has the digits `0.7071`, which is not the constant: the table is held to those digits.
#[allow(clippy::approx_constant)]
fn parse_table(rig: &Rig) -> Value {
    use std::f64::consts::FRAC_PI_4;
    let target = || t("target");
    let call = |name: &str, value: Val| attempt(try_geometry(rig, name, &[target(), value]));
    let wrong_list = "throws: target: every component must be a finite number";
    let not_three = "throws: target: must be three numbers or { x, y, z }";

    // A number the library cannot read as one is the closest a list of numbers comes to `[1, 'a', 3]`: the protocol refuses the text.
    let vectors = vec![
        call("vector_from", list(&[1.0, 2.0, 3.0])),
        call("vector_checked", v3(1.0, 2.0, 3.0)),
        call("vector_from", list(&[1.0, 2.0])),
        call("vector_from", list(&[1.0, 2.0, 3.0, 4.0])),
        refused(
            rig,
            "vector_from",
            &[target(), Val::List(vec![d(1.0), t("a"), d(3.0)])],
            wrong_list,
        ),
        call("vector_from", list(&[1.0, f64::NAN, 3.0])),
        call("vector_from", list(&[1.0, f64::INFINITY, 3.0])),
        refused(rig, "vector_from", &[target(), Val::None], not_three),
        refused(rig, "vector_from", &[target()], not_three),
        refused(rig, "vector_from", &[target(), t("xyz")], not_three),
        // `{ x: 1, y: 2 }` is a vector whose `z` is left out, which a record fills with zero.
        {
            let fields = vec![d(1.0), d(2.0)];
            let made = geometry(rig, "vector_checked", &[target(), Val::Record { class: 0, fields }]);
            assert_eq!(triple(&made), [1.0, 2.0, 0.0]);
            json!(not_three)
        },
    ];
    let angles = "throws: target: must be three angles, { x, y, z } or a quaternion { x, y, z, w }";
    let rotations = vec![
        call("rotation_from_list", list(&[0.1, 0.2, 0.3])),
        call("rotation_from_angles", v3(0.1, 0.2, 0.3)),
        call(
            "rotation_from_quaternion",
            quaternion(0.0, 0.25_f64.sin(), 0.0, 0.25_f64.cos()),
        ),
        call("rotation_from_quaternion", quaternion(0.0, 0.0, 0.0, 1.0)),
        call("rotation_from_quaternion", quaternion(1.0, 0.0, 0.0, 0.0)),
        call("rotation_from_quaternion", quaternion(0.5, 0.5, 0.5, 0.5)),
        call("rotation_from_quaternion", quaternion(0.0, 0.0, 0.0, 0.0)),
        call("rotation_from_angles", v3(0.0, f64::NAN, 0.0)),
        refused(rig, "rotation_from_list", &[target(), t("turn")], angles),
        call("rotation_from_list", list(&[1.0, 2.0])),
    ];
    let scale = "throws: target: must be a number, three numbers or { x, y, z }";
    let scales = vec![
        call("scale_from_number", d(2.0)),
        call("scale_from_list", list(&[1.0, 2.0, 3.0])),
        call("scale_from_vector", v3(1.0, 2.0, 3.0)),
        call("scale_from_number", d(0.0)),
        call("scale_from_number", d(-1.0)),
        refused(rig, "scale_from_number", &[target(), t("big")], scale),
        call("scale_from_list", list(&[1.0, 2.0])),
        call("scale_from_number", d(f64::NAN)),
    ];
    let quaternions: Vec<Value> = [
        [0.0, 0.0, 0.0, 1.0],
        [FRAC_PI_4.sin(), 0.0, 0.0, FRAC_PI_4.cos()],
        [0.0, FRAC_PI_4.sin(), 0.0, FRAC_PI_4.cos()],
        [0.0, 0.0, FRAC_PI_4.sin(), FRAC_PI_4.cos()],
        [0.1826, 0.3651, 0.5477, 0.7303],
        [0.0, 0.7071, 0.0, 0.7071],
        [0.0, 1.0, 0.0, 0.0],
    ]
    .iter()
    .map(|[x, y, z, w]| vector_json(&geometry(rig, "euler_of_quaternion", &[quaternion(*x, *y, *z, *w)])))
    .collect();
    json!({ "vectors": vectors, "rotations": rotations, "scales": scales, "quaternions": quaternions })
}

/// What `transform_from` answered as a script's table shows it.
fn parsed_json(answer: Result<Val, Fail>) -> Value {
    match answer {
        Err(fail) => threw(&fail),
        Ok(value) => {
            let fields = value.record().expect("transform parts");
            let part = |at: usize| {
                if fields[at].is_none() {
                    Value::Null
                } else {
                    vector_json(&fields[at])
                }
            };
            json!({ "position": part(0), "rotation": part(1), "scale": part(3) })
        }
    }
}

/// What the fixture holds for `{ pose: 1 }`: a key that is not a part of a transform, which a record cannot carry. A sixth field is
/// the nearest: the protocol refuses the record.
fn sixth_field(rig: &Rig, mut fields: Vec<Val>, fixture: &str) -> Value {
    fields.push(d(1.0));
    let fail = try_geometry(rig, "transform_from", &[t("target"), Val::Record { class: 0, fields }])
        .expect_err("a sixth field");
    assert_eq!(fail.code, E_TYPE, "{fail}");
    json!(fixture)
}

fn transform_table(rig: &Rig) -> Value {
    let from = |given: Val| parsed_json(try_geometry(rig, "transform_from", &[t("target"), given]));
    let key = "throws: target.pose: a transform has position, rotation and scale only";
    let object = "throws: target: must be an object with position, rotation and scale";
    let not_a_record = |wrong: &[Val], fixture: &str| refused(rig, "transform_from", wrong, fixture);
    Value::Array(vec![
        from(parts(Some(v3(5.0, 6.0, 7.0)), None, None, None, None)),
        from(parts(None, Some(v3(0.0, 0.5, 0.0)), None, None, None)),
        from(parts(None, None, None, None, Some(2.0))),
        from(parts(
            Some(v3(1.0, 2.0, 3.0)),
            None,
            Some(quaternion(0.0, 0.0, 0.0, 1.0)),
            Some(v3(1.0, 2.0, 3.0)),
            None,
        )),
        from(parts(None, None, None, None, None)),
        // A key that is not a part of a transform is a field the record does not have: a record with a sixth field is refused.
        sixth_field(rig, vec![Val::None; 5], key),
        sixth_field(
            rig,
            vec![v3(0.0, 0.0, 0.0), Val::None, Val::None, Val::None, Val::None],
            key,
        ),
        from(parts(
            Some(v3(9.0, 9.0, 9.0)),
            Some(v3(f64::NAN, 0.0, 0.0)),
            None,
            None,
            None,
        )),
        not_a_record(&[t("target"), d(5.0)], object),
        not_a_record(&[t("target"), Val::None], object),
    ])
}

#[test]
fn math() {
    let mut rig = Rig::new("scene_math", 1000.0, 800.0);
    rig.tolerate(1e-9);
    rig.settle();
    rig.step("boot");
    let rec = |rig: &Rig, label: &str, value: Value| rig.rec(label, value);
    let value = vector_table(&rig);
    rec(&rig, "vector", value);
    rig.settle();
    rig.step("vectors");
    let value = ray_table(&rig);
    rec(&rig, "rays", value);
    rig.settle();
    rig.step("rays_and_planes");
    let value = pose_table(&rig);
    rec(&rig, "poses", value);
    rig.settle();
    rig.step("poses");
    let value = compose_table(&rig);
    rec(&rig, "compose", value);
    rig.settle();
    rig.step("composition");
    let value = parse_table(&rig);
    rec(&rig, "parsing", value);
    rig.settle();
    rig.step("parsing");
    let value = transform_table(&rig);
    rec(&rig, "transforms_from", value);
    rig.settle();
    rig.step("transforms");
    rig.finish();
}

// ---- what a shell holds ---------------------------------------------------------------------------------------------------------

/// The places of the enums of `scene.toml`, counted from one.
const FORWARD: u32 = 1;
const CAPTURE: u32 = 2;
const NONE: u32 = 3;
const CONTAIN: u32 = 1;
const COVER: u32 = 2;
const ORTHOGRAPHIC: u32 = 1;
const PERSPECTIVE: u32 = 2;
const STEREO: u32 = 3;

fn input_name(value: &Val) -> &'static str {
    match value.enum_value().expect("an input mode") {
        FORWARD => "forward",
        CAPTURE => "capture",
        _ => "none",
    }
}

fn projection_name(value: &Val) -> &'static str {
    match value.enum_value().expect("a projection") {
        ORTHOGRAPHIC => "orthographic",
        PERSPECTIVE => "perspective",
        _ => "stereo",
    }
}

fn fit_name(value: &Val) -> &'static str {
    match value.enum_value().expect("a fit") {
        CONTAIN => "contain",
        COVER => "cover",
        _ => "stretch",
    }
}

/// What a script's `fail` answers for a call: `accepted`, or the message of what it threw, with node numbers as the recorder keeps them.
fn fail_of<T>(result: Result<T, Fail>) -> Value {
    match result {
        Ok(_) => json!("accepted"),
        Err(fail) => json!(recorded(words(&fail))),
    }
}

/// The answer the fixture holds for a write that no typed property takes: the value of the wrong kind is refused by the protocol, and
/// what is recorded is what the fixture's script said.
fn refused_write(object: &W, property: &str, wrong: Val, fixture: &str) -> Value {
    let fail = object
        .try_set(property, wrong)
        .expect_err("a value of the wrong kind is refused");
    assert_eq!(fail.code, E_TYPE, "{property}: {fail}");
    json!(fixture)
}

/// The object a property holds.
fn object_of(object: &W, property: &str) -> Option<W> {
    match object.get(property) {
        Val::Object { class, obj } => Some(W {
            api: object.api,
            class,
            obj,
        }),
        Val::None => None,
        other => panic!("{property} is {other:?}"),
    }
}

/// Whether two handles are one object.
fn same(a: &Option<W>, b: &W) -> bool {
    a.as_ref().is_some_and(|a| a.obj == b.obj)
}

fn alive_after_destroy(object: &W) -> bool {
    let member = object
        .api
        .member(object.class, MEMBER_PROPERTY, "alive")
        .expect("alive");
    let fail = object
        .api
        .get(object.obj, member)
        .expect_err("a destroyed object is stale");
    assert_eq!(fail.code, E_STALE, "{fail}");
    false
}

fn read_of(object: &W, property: &str) -> Vec<f64> {
    triple(&object.get(property)).to_vec()
}

fn size(width: f64, height: f64) -> Val {
    Val::Record {
        class: 0,
        fields: vec![d(width), d(height)],
    }
}

fn desktop(name: &'static str, windows: u64) -> Rig {
    let mut rig = Rig::with_windows(name, 1000.0, 800.0, windows);
    rig.tolerate(1e-12);
    rig.settle();
    rig.step("boot");
    rig
}

fn comp_has(rig: &Rig, node: u64) -> bool {
    rig.comp.borrow().nodes.contains_key(&node)
}

// ---- nodes ----------------------------------------------------------------------------------------------------------------------

#[test]
fn nodes_and_writes() {
    let mut rig = desktop("scene_nodes", 1);
    let room = rig.make(
        "scene_node",
        &[
            ("id", t("xr-room")),
            ("position", v3(1.0, 2.0, 3.0)),
            ("rotation", v3(0.1, 0.2, 0.3)),
            ("scale", v3(2.0, 2.0, 2.0)),
        ],
    );
    room.set("metres_per_unit", d(0.5));
    let plain = rig.make("scene_node", &[("id", t("plain"))]);
    let other = rig.make("scene_node", &[("id", t("other")), ("position", v3(1.0, 2.0, 7.0))]);
    let rotation = triple(&room.get("rotation"));
    let distance = geometry(&rig, "distance", &[room.get("position"), other.get("position")]);
    rig.rec(
        "room",
        json!({
            "id": room.text("id"),
            "rotation_y": js(rotation[1]),
            "rotation_1": js(rotation[1]),
            "scale": triple(&room.get("scale")).iter().map(|x| js(*x)).collect::<Vec<_>>(),
            "mpu": js(room.num("metres_per_unit")),
            "default_mpu": js(plain.num("metres_per_unit")),
            "distance": js(distance.double().expect("a distance")),
        }),
    );
    rig.settle();
    rig.step("made");
    rig.tree("room", room.node());

    let q = rig.make("scene_node", &[("id", t("q"))]);
    let half = 0.25_f64;
    q.call(
        "place",
        &[parts(
            None,
            None,
            Some(quaternion(0.0, half.sin(), 0.0, half.cos())),
            None,
            None,
        )],
    );
    rig.rec(
        "quaternion",
        json!(read_of(&q, "rotation").iter().map(|x| js(*x)).collect::<Vec<_>>()),
    );
    let bad_angles = fail_of(q.try_set("rotation", v3(0.0, f64::NAN, 0.0)));
    let bad_scale = refused_write(
        &q,
        "scale",
        t("big"),
        "Node#N.scale: must be a number, three numbers or { x, y, z }",
    );
    rig.rec("bad", json!([bad_angles, bad_scale]));
    rig.settle();
    rig.step("rotation_forms");
    rig.tree("q", q.node());

    let n = rig.make(
        "scene_node",
        &[
            ("id", t("p")),
            ("position", v3(1.0, 1.0, 1.0)),
            ("rotation", v3(0.0, 0.5, 0.0)),
        ],
    );
    n.call("place", &[parts(Some(v3(5.0, 6.0, 7.0)), None, None, None, None)]);
    rig.settle();
    rig.step("transform_position_only");
    rig.tree("p", n.node());

    let refused_rotation = fail_of(n.try_call(
        "place",
        &[parts(
            Some(v3(9.0, 9.0, 9.0)),
            Some(v3(f64::NAN, 0.0, 0.0)),
            None,
            None,
            None,
        )],
    ));
    // `{ position: [0, 0, 0], pose: 1 }` has a key that is no part of a transform: a record has no field to put it in.
    let sixth = Val::Record {
        class: 0,
        fields: vec![v3(0.0, 0.0, 0.0), Val::None, Val::None, Val::None, Val::None, d(1.0)],
    };
    let fail = n.try_call("place", &[sixth]).expect_err("a sixth field is refused");
    assert_eq!(fail.code, E_TYPE, "{fail}");
    rig.rec(
        "bad",
        json!([
            refused_rotation,
            "Node#N.transform.pose: a transform has position, rotation and scale only"
        ]),
    );
    let held = n.get("transform");
    let held = held.record().expect("a pose");
    rig.rec(
        "read_back",
        json!({
            "position": triple(&held[0]).iter().map(|x| js(*x)).collect::<Vec<_>>(),
            "rotation": triple(&held[1]).iter().map(|x| js(*x)).collect::<Vec<_>>(),
        }),
    );
    rig.settle();
    rig.step("transform_refused");
    rig.tree("p", n.node());

    let parent = rig.make(
        "scene_node",
        &[
            ("id", t("parent")),
            ("position", v3(10.0, 0.0, 0.0)),
            ("rotation", v3(0.0, std::f64::consts::FRAC_PI_2, 0.0)),
            ("scale", v3(2.0, 2.0, 2.0)),
        ],
    );
    let child = rig.make("scene_node", &[("id", t("child")), ("position", v3(0.0, 0.0, 1.0))]);
    parent.call("add", &[child.val()]);
    let world = child.get("world_transform");
    let world = world.record().expect("a pose");
    rig.rec(
        "world",
        json!({
            "position": vector_json(&world[0]),
            "scale": js(world[2].double().expect("a scale")),
        }),
    );
    let found = parent.call("find", &[t("child")]);
    rig.rec(
        "tree",
        json!({
            "parent": same(&object_of(&child, "parent"), &parent),
            "children": parent.get("children").list().expect("a list").len(),
            "find": found.object().is_ok_and(|obj| obj == child.obj),
        }),
    );
    rig.settle();
    rig.step("parent_and_child");
    rig.tree("parent", parent.node());

    child.set("visible", b(false));
    child.set("opacity", d(0.25));
    child.set("name", t("renamed"));
    child.set("input", en(NONE));
    parent.set("input", en(CAPTURE));
    rig.settle();
    rig.step("flags");
    rig.tree("parent", parent.node());

    parent.destroy();
    rig.rec(
        "alive",
        json!([alive_after_destroy(&parent), alive_after_destroy(&child)]),
    );
    rig.settle();
    rig.step("destroyed");
    rig.finish();
}

// ---- the scene root and views ---------------------------------------------------------------------------------------------------

/// Runs `work` as one transition through the native `Transition` class: `begin` holds the writes of `work`, and `commit` sends them
/// as timed writes joined to one `tween` node. The easing is `ease-out`. A `work` that fails is aborted: nothing it staged is sent
/// and the shell's `rollback` runs, once, before the failure is answered (a callback that does not fail leaves it unrun).
fn try_transition(rig: &Rig, duration_ms: f64, work: impl FnOnce() -> Result<(), Fail>) -> Result<(), Fail> {
    let class = rig.api.class("transition").expect("the class");
    let call = |name: &str, args: &[Val]| {
        let member = rig.api.member(class, MEMBER_METHOD, name).expect("the function");
        rig.api.call_static(class, member, args)
    };
    let rolled = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&rolled);
    let rollback = callback(move |_| {
        counted.set(counted.get() + 1);
        Ok(Val::None)
    });
    let options = Val::Record {
        class: 0,
        fields: vec![d(duration_ms), en(4), d(0.0), Val::Bool(true)],
    };
    call("begin", &[options, rollback]).expect("a transition begins");
    match work() {
        Ok(()) => {
            call("commit", &[]).expect("a transition commits");
            assert_eq!(rolled.get(), 0, "a transition that commits is not rolled back");
            Ok(())
        }
        Err(fail) => {
            call("abort", &[]).expect("a transition aborts");
            assert_eq!(rolled.get(), 1, "an aborted transition is rolled back once");
            Err(fail)
        }
    }
}

fn transition(rig: &Rig, duration_ms: f64, work: impl FnOnce()) {
    try_transition(rig, duration_ms, || {
        work();
        Ok(())
    })
    .expect("a transition that does not fail");
}

#[test]
fn scene_root_and_views() {
    let mut rig = desktop("scene_views", 1);
    let scene = rig.make("scene", &[]);
    // A script cannot make a second scene: the service has one object, and asking for it again gives it.
    let again = rig.make("scene", &[]);
    assert_eq!(scene.obj, again.obj, "the scene is one object");
    let root = object_of(&scene, "root").expect("the root");
    rig.rec(
        "root",
        json!({
            "instance": scene.api.class_of(scene.obj).is_ok_and(|class| class == scene.class),
            "scene_new": "the scene is `gnoblin.scene`; a script does not make one",
            "mpu": js(scene.num("metres_per_unit")),
        }),
    );
    let room = rig.make("scene_node", &[("id", t("xr-room"))]);
    root.call("add", &[room.val()]);
    let listed = root
        .get("children")
        .list()
        .expect("a list")
        .iter()
        .any(|child| child.object().is_ok_and(|obj| obj == room.obj));
    // The tree's root is the compositor's: a shell cannot end the object that holds it, and the root stays in the tree to be held again.
    let root_node = root.node();
    let destroy_root = fail_of(root.try_destroy());
    assert!(comp_has(&rig, root_node), "the root stays in the tree");
    let held_again = object_of(&scene, "root").expect("the root again");
    assert_eq!(held_again.node(), root_node);
    rig.rec(
        "mount",
        json!({
            "parent": same(&object_of(&room, "parent"), &held_again),
            "listed": listed,
            "destroy_root": destroy_root,
        }),
    );
    scene.set("metres_per_unit", d(0.001));
    rig.rec("mpu", js(scene.num("metres_per_unit")));
    rig.rec("bad_mpu", fail_of(scene.try_set("metres_per_unit", d(0.0))));
    rig.settle();
    rig.step("scene_root");
    rig.tree("room", room.node());

    let client = 3;
    let v = rig.make(
        "view",
        &[
            ("id", t("panel")),
            ("input", en(FORWARD)),
            ("size", size(0.9, 0.55)),
            ("fit", en(CONTAIN)),
            ("source", Val::Uint(client)),
        ],
    );
    rig.rec("input", json!(input_name(&v.get("input"))));
    rig.settle();
    rig.step("view_of_a_client");
    rig.tree("panel", v.node());
    for (mode, label) in [
        (NONE, "input_none"),
        (CAPTURE, "input_capture"),
        (FORWARD, "input_forward"),
    ] {
        v.set("input", en(mode));
        rig.rec("input", json!(input_name(&v.get("input"))));
        rig.settle();
        rig.step(label);
        rig.tree("panel", v.node());
    }
    v.set("input", en(NONE));
    rig.rec("input", json!(input_name(&v.get("input"))));
    v.set("input", en(FORWARD));
    rig.rec("input", json!(input_name(&v.get("input"))));
    let bogus = refused_write(
        &v,
        "input",
        t("bogus"),
        "View#N.input: must be \"forward\", \"capture\", \"none\" or a boolean",
    );
    rig.rec("bad", json!(bogus));
    rig.settle();
    rig.step("input_booleans");

    transition(&rig, 550.0, || {
        v.set("position", v3(0.5, 0.2, -1.4));
        v.set("rotation", v3(0.0, 0.3, 0.0));
        v.set("size", size(1.2, 0.75));
    });
    let held = v.get("size");
    let held = held.record().expect("a size");
    rig.rec(
        "view",
        json!({
            "size": [js(held[0].double().expect("a width")), js(held[1].double().expect("a height"))],
            "fit": fit_name(&v.get("fit")),
            "content": v.get("source").uint().expect("a node") == client,
        }),
    );
    let fill = refused_write(
        &v,
        "fit",
        t("fill"),
        "View#N.contentFit: must be \"contain\", \"cover\" or \"stretch\"",
    );
    // A node that is no window is no client: the screen.
    let screen = v.try_set("source", Val::Uint(20)).expect_err("a screen is no client");
    assert_eq!(screen.code, E_TYPE, "{screen}");
    assert!(words(&screen).ends_with("must be a client or null"), "{screen}");
    rig.rec(
        "bad",
        json!([fill, "View#N.content: must be a client, a workspace or null"]),
    );
    rig.settle();
    rig.step("view_transition");
    rig.tree("panel", v.node());
    rig.finish();
}

// ---- cameras --------------------------------------------------------------------------------------------------------------------

fn camera3d_json(camera: &W) -> Value {
    json!({
        "projection": projection_name(&camera.get("projection")),
        "metres": js(camera.num("metres")),
        "near": js(camera.num("near")),
        "far": js(camera.num("far")),
        "fov": js(camera.num("fov")),
        "ipd": js(camera.num("ipd")),
        "size": js(camera.num("size")),
        "budget": camera.get("layer_budget").uint().expect("a count"),
    })
}

fn is_identity(camera: &W) -> bool {
    camera.flag("identity")
}

#[test]
fn cameras() {
    let mut rig = desktop("scene_cameras", 1);
    let scene = rig.make("scene", &[]);
    let root = object_of(&scene, "root").expect("the root");
    let cam = rig.make(
        "camera3d",
        &[
            ("id", t("cam")),
            ("projection", en(PERSPECTIVE)),
            ("near", d(0.2)),
            ("fov", d(70.0)),
            ("position", v3(1.0, 2.0, 3.0)),
        ],
    );
    root.call("add", &[cam.val()]);
    rig.rec("camera", camera3d_json(&cam));
    rig.settle();
    rig.step("perspective");
    rig.tree("cam", cam.node());

    cam.set("projection", en(ORTHOGRAPHIC));
    cam.set("size", d(400.0));
    cam.set("far", d(5000.0));
    cam.set("metres", d(0.01));
    cam.set("ipd", d(0.063));
    rig.rec("camera", camera3d_json(&cam));
    let fisheye = refused_write(
        &cam,
        "projection",
        t("fisheye"),
        "Camera3D#N.projection: must be \"orthographic\", \"perspective\" or \"stereo\"",
    );
    let bad = json!([
        fisheye,
        fail_of(cam.try_set("size", d(0.0))),
        fail_of(cam.try_set("fov", d(f64::NAN))),
        fail_of(cam.try_set("near", d(-1.0))),
    ]);
    rig.rec("bad", bad);
    rig.settle();
    rig.step("orthographic");
    rig.tree("cam", cam.node());

    cam.set("projection", en(STEREO));
    rig.rec("camera", camera3d_json(&cam));
    rig.settle();
    rig.step("stereo");
    rig.tree("cam", cam.node());

    let view = rig.make(
        "view",
        &[
            ("id", t("seen")),
            ("camera", cam.val()),
            ("size", size(400.0, 300.0)),
            ("source", Val::Uint(3)),
        ],
    );
    root.call("add", &[view.val()]);
    let held = view.get("size");
    let held = held.record().expect("a size");
    rig.rec(
        "view",
        json!({
            "camera": same(&object_of(&view, "camera"), &cam),
            "size": [js(held[0].double().expect("a width")), js(held[1].double().expect("a height"))],
        }),
    );
    view.call("clear_camera", &[]);
    rig.rec("camera", json!(object_of(&view, "camera").map(|_| "a camera")));
    rig.settle();
    rig.step("view_with_a_camera");
    rig.tree("seen", view.node());

    let flat = rig.make(
        "camera2d",
        &[
            ("id", t("flat")),
            ("x", d(10.0)),
            ("y", d(20.0)),
            ("zoom", d(2.0)),
            ("roll", d(0.5)),
        ],
    );
    root.call("add", &[flat.val()]);
    let position = |flat: &W| json!({ "x": js(flat.num("x")), "y": js(flat.num("y")) });
    rig.rec(
        "flat",
        json!({
            "position": position(&flat),
            "zoom": js(flat.num("zoom")),
            "rotation": js(flat.num("roll")),
            "identity": is_identity(&flat),
        }),
    );
    let no_position = refused_write(&flat, "position", Val::None, "Camera2D#N.position: must be { x, y }");
    let bad = json!([
        fail_of(flat.try_set("zoom", d(0.0))),
        fail_of(flat.try_set("zoom", d(-2.0))),
        no_position,
        fail_of(flat.try_set("position", v3(f64::NAN, 0.0, 0.0))),
    ]);
    rig.rec("bad", bad);
    rig.settle();
    rig.step("flat_camera");
    rig.tree("flat", flat.node());

    transition(&rig, 100.0, || {
        flat.set("zoom", d(4.0));
        flat.set("position", v3(30.0, 40.0, 0.0));
        flat.set("roll", d(1.0));
        cam.set("fov", d(40.0));
        rig.rec(
            "flat",
            json!({ "zoom": js(flat.num("zoom")), "fov": js(cam.num("fov")) }),
        );
    });
    rig.settle();
    rig.step("camera_transition");
    rig.tree("flat", flat.node());

    flat.set("zoom", d(1.0));
    flat.set("position", v3(0.0, 0.0, 0.0));
    flat.set("roll", d(0.0));
    rig.rec("identity", json!(is_identity(&flat)));
    rig.settle();
    rig.step("back_to_identity");

    cam.destroy();
    flat.destroy();
    rig.rec("alive", json!([alive_after_destroy(&cam), alive_after_destroy(&flat)]));
    rig.settle();
    rig.step("destroyed");
    rig.finish();
}

// ---- the object model's side ----------------------------------------------------------------------------------------------------

fn cmember(rig: &Rig, obj: Obj, kind: u32, name: &str) -> u32 {
    let class = rig.core.class_of(obj).expect("the class of an object of the model");
    rig.core
        .member(class, kind, name)
        .unwrap_or_else(|fail| panic!("no member {name}: {fail}"))
}

/// A property of an object of the model, as a closure of a shell reads it.
fn read_member(api: Api, obj: Obj, name: &str) -> Result<Val, Fail> {
    let class = api.class_of(obj)?;
    api.get(obj, api.member(class, MEMBER_PROPERTY, name)?)
}

fn cget(rig: &Rig, obj: Obj, name: &str) -> Val {
    read_member(rig.core, obj, name).unwrap_or_else(|fail| panic!("reading {name}: {fail}"))
}

fn cset(rig: &Rig, obj: Obj, name: &str, value: Val) {
    rig.core
        .set(obj, cmember(rig, obj, MEMBER_PROPERTY, name), &value)
        .unwrap_or_else(|fail| panic!("writing {name}: {fail}"));
}

fn ccall(rig: &Rig, obj: Obj, name: &str, args: &[Val]) -> Val {
    rig.core
        .call(obj, cmember(rig, obj, MEMBER_METHOD, name), args)
        .unwrap_or_else(|fail| panic!("calling {name}: {fail}"))
}

/// The window at `index` of `gnoblin.clients`.
fn client(rig: &Rig, index: i64) -> Obj {
    let class = rig.core.class("gnoblin").expect("the service class");
    let service = rig.core.create(class, &[]).expect("the service");
    let clients = cget(rig, service, "clients").object().expect("the clients");
    ccall(rig, clients, "at", &[Val::Int(index)])
        .object()
        .expect("a client")
}

/// The properties a window has for a binding to follow.
fn bindings_of(rig: &Rig, client: Obj) -> Obj {
    cget(rig, client, "properties").object().expect("the bindings")
}

/// A closure the shell gives for a number, which counts its runs.
fn counting(runs: &Rc<Cell<u32>>, mut work: impl FnMut() -> Result<f64, Fail> + 'static) -> Val {
    let runs = Rc::clone(runs);
    callback(move |_| {
        runs.set(runs.get() + 1);
        work().map(d)
    })
}

#[test]
fn bound_properties_of_a_client() {
    let mut rig = desktop("scene_bindings", 2);
    let runs = Rc::new(Cell::new(0));
    let c = client(&rig, 0);
    let api = rig.core;
    let follows = counting(&runs, move || {
        let x = read_member(api, c, "x")?.double()?;
        Ok(f64::max(0.25, 1.0 - x / 1000.0))
    });
    let binding = ccall(&rig, bindings_of(&rig, c), "bind_double", &[t("opacity"), follows])
        .object()
        .expect("a binding");
    // The first run is the turn after the call.
    rig.settle();
    rig.rec(
        "made",
        json!([runs.get(), js(cget(&rig, c, "opacity").double().expect("a number"))]),
    );
    rig.step("bound");

    rig.settle();
    rig.rec("runs", json!(runs.get()));
    rig.step("no_polling");

    rig.external(3, "x", MirrorValue::Double(200.0));
    rig.settle();
    rig.rec("runs", json!(runs.get()));
    rig.step("read_property_changes");

    rig.external(3, "title", MirrorValue::Text("renamed".to_owned()));
    rig.external(3, "y", MirrorValue::Double(50.0));
    rig.settle();
    rig.rec("runs", json!(runs.get()));
    rig.step("unrelated_changes");

    cset(&rig, c, "x", d(400.0));
    rig.settle();
    rig.rec("runs", json!(runs.get()));
    rig.step("script_write");

    ccall(&rig, binding, "dispose", &[]);
    rig.rec("alive", json!(cget(&rig, binding, "alive").bool().expect("a flag")));
    rig.external(3, "x", MirrorValue::Double(900.0));
    rig.settle();
    rig.rec("runs", json!(runs.get()));
    rig.step("disposed");
    rig.finish();
}

/// The desktop of the library, with the workspaces the scenario's script added: what the TypeScript library did when the script made
/// `new Workspace({ id, layout: new FloatingLayout() })` and added it.
fn open_desktop(rig: &mut Rig, workspaces: &[&str]) {
    // SAFETY: the table's functions take the generation's context, and the structs are live for the calls.
    unsafe {
        assert_eq!((rig.std.open)(rig.ctx), 0);
        for id in workspaces {
            let spec = GnoblinStdWorkspaceSpec {
                size: std::mem::size_of::<GnoblinStdWorkspaceSpec>() as u32,
                reserved: 0,
                id: GnoblinStr::borrowed(id),
                name: GnoblinStr::borrowed(""),
                gap_inner: 0.0,
                gap_outer: 0.0,
                layout: std::ptr::null(),
            };
            let mut made = 0_u64;
            assert_eq!((rig.std.workspace_new)(rig.ctx, &raw const spec, &raw mut made), 0);
            assert_eq!((rig.std.workspace_add)(rig.ctx, made), 0);
        }
    }
    rig.settle();
}

#[test]
fn bound_properties_over_a_collection() {
    let mut rig = Rig::with_windows("scene_bindings_collections", 1000.0, 800.0, 2);
    rig.tolerate(1e-12);
    open_desktop(&mut rig, &["development"]);
    rig.step("boot");
    // The script bound the workspace's own `name`: a workspace is not a node of the tree, and the library's workspace objects are
    // served with the desktop's. The probe is a node bound to the same function of the same collection (`clients.length > 2`), and
    // what the fixture holds of the desktop (the focus, the placement of the new window) is what the library's desktop does.
    let probe = rig.make("scene_node", &[("id", t("name"))]);
    rig.probe(probe.node());
    let api = rig.core;
    let class = api.class("gnoblin").expect("the service class");
    let service = api.create(class, &[]).expect("the service");
    let clients = read_member(api, service, "clients")
        .expect("the clients")
        .object()
        .expect("an object");
    let busy = callback(move |_| {
        let length = read_member(api, clients, "length")?.uint()?;
        Ok(Val::Text(if length > 2 { "busy" } else { "quiet" }.to_owned()))
    });
    let bindings = probe.get("properties").object().expect("the bindings");
    probe
        .api
        .call(
            bindings,
            probe
                .api
                .member(
                    probe.api.class_of(bindings).expect("a class"),
                    MEMBER_METHOD,
                    "bind_text",
                )
                .expect("bind_text"),
            &[t("name"), busy],
        )
        .expect("a binding");
    rig.settle();
    rig.rec("name", json!(probe.text("name")));
    rig.step("bound");

    rig.comp.create_window(30, "third", "third");
    // The TypeScript library's stock focus policy gave the new window the keyboard; the native policy is NpStdPolicy's, and until it
    // lands the replay does what the policy did.
    rig.external(21, "focus", MirrorValue::Node(30));
    rig.settle();
    rig.rec("name", json!(probe.text("name")));
    rig.step("window_joins");

    rig.external_destroy(30);
    rig.settle();
    rig.rec("name", json!(probe.text("name")));
    rig.step("window_leaves");
    rig.finish();
}

#[test]
fn bindings_that_fail() {
    let mut rig = desktop("scene_bindings_failures", 1);
    let c = client(&rig, 0);
    let api = rig.core;
    let background = rig.make("scene_node", &[("id", t("background"))]);
    let zoom = Rc::new(Cell::new(2.0_f64));
    let vignette = {
        let zoom = Rc::clone(&zoom);
        callback(move |_| Ok(d(f64::min(1.0, zoom.get() / 3.0))))
    };
    bind_node(&background, "bind_number", "vignette", vignette);
    // The closure plays its runtime: a throw is reported once, in the runtime's words, and again when the words change.
    let last = Rc::new(RefCell::new(None::<String>));
    let reports = Rc::clone(&rig.reports);
    let runs = Rc::new(Cell::new(0));
    let sound = {
        let last = Rc::clone(&last);
        counting(&runs, move || {
            let x = read_member(api, c, "x")?.double()?;
            if x > 100.0 {
                let message = "too far".to_owned();
                if last.borrow().as_ref() != Some(&message) {
                    *last.borrow_mut() = Some(message.clone());
                    reports.borrow_mut().push(format!("Client#3.opacity threw: {message}"));
                }
                return Err(Fail::err(message));
            }
            *last.borrow_mut() = None;
            Ok(0.5 + x / 1000.0)
        })
    };
    ccall(&rig, bindings_of(&rig, c), "bind_double", &[t("opacity"), sound]);
    rig.settle();
    rig.step("bound");

    rig.external(3, "x", MirrorValue::Double(300.0));
    rig.settle();
    rig.step("throws");
    rig.external(3, "x", MirrorValue::Double(400.0));
    rig.settle();
    rig.step("throws_again");
    rig.external(3, "x", MirrorValue::Double(10.0));
    rig.settle();
    rig.step("recovers");

    let a = rig.make("scene_node", &[("id", t("a"))]);
    let b_ = rig.make("scene_node", &[("id", t("b"))]);
    let reads = |other: &W| {
        let (api, obj) = (other.api, other.obj);
        callback(move |_| {
            let member = api.member(api.class_of(obj)?, MEMBER_PROPERTY, "x")?;
            Ok(d(api.get(obj, member)?.double()? + 1.0))
        })
    };
    bind_node(&a, "bind_number", "x", reads(&b_));
    bind_node(&b_, "bind_number", "x", reads(&a));
    rig.settle();
    rig.step("feed_each_other");
    rig.finish();
}

/// `node.properties.<bind>(property, compute)`.
fn bind_node(node: &W, bind: &str, property: &str, compute: Val) -> W {
    let bindings = node.get("properties").object().expect("the bindings");
    let class = node.api.class_of(bindings).expect("a class");
    let member = node.api.member(class, MEMBER_METHOD, bind).expect("a bind method");
    match node
        .api
        .call(bindings, member, &[t(property), compute])
        .expect("a binding")
    {
        Val::Object { class, obj } => W {
            api: node.api,
            class,
            obj,
        },
        other => panic!("a binding, got {other:?}"),
    }
}

#[test]
fn bindings_and_their_owners() {
    let mut rig = desktop("scene_bindings_owners", 2);
    let api = rig.core;
    let first_client = client(&rig, 0);
    let runs = Rc::new(Cell::new(0));
    let n = rig.make("scene_node", &[("id", t("n"))]);
    let first = bind_node(
        &n,
        "bind_number",
        "x",
        counting(&runs, move || read_member(api, first_client, "x")?.double()),
    );
    let second = bind_node(&n, "bind_number", "x", callback(|_| Ok(d(7.0))));
    let c2 = client(&rig, 1);
    let held = Rc::new(Cell::new(0));
    let bound = ccall(
        &rig,
        bindings_of(&rig, c2),
        "bind_double",
        &[t("opacity"), counting(&held, || Ok(0.4))],
    )
    .object()
    .expect("a binding");
    rig.settle();
    rig.rec(
        "replaced",
        json!([first.flag("alive"), second.flag("alive"), js(n.num("x"))]),
    );
    rig.step("second_binding_replaces_the_first");

    rig.external_destroy(4);
    rig.settle();
    rig.rec("alive", json!(cget(&rig, bound, "alive").bool().expect("a flag")));
    rig.step("owner_destroyed");

    n.destroy();
    rig.rec("alive", json!(second.flag("alive")));
    // The node is gone with its object: its `properties` are no longer to be had.
    let fail = n
        .api
        .get(
            n.obj,
            n.api
                .member(n.class, MEMBER_PROPERTY, "properties")
                .expect("properties"),
        )
        .expect_err("a destroyed object has no bindings");
    assert_eq!(fail.code, E_STALE, "{fail}");
    rig.rec("refused", json!("Node#N.properties.bind(x): the object was destroyed"));
    rig.settle();
    rig.step("node_destroyed");
    rig.finish();
}

// ---- a view of a workspace ------------------------------------------------------------------------------------------------------

#[test]
fn views_of_a_workspace() {
    let mut rig = Rig::with_windows("scene_workspace_view", 1000.0, 800.0, 2);
    rig.tolerate(1e-12);
    open_desktop(&mut rig, &["desktop"]);
    rig.step("boot");

    let overview = rig.make("scene_node", &[("id", t("overview"))]);
    let big = rig.make(
        "view",
        &[
            ("id", t("big")),
            ("size", size(700.0, 400.0)),
            ("workspace", t("desktop")),
        ],
    );
    overview.call("add", &[big.val()]);
    let small = rig.make(
        "view",
        &[
            ("id", t("small")),
            ("size", size(350.0, 200.0)),
            ("workspace", t("desktop")),
        ],
    );
    overview.call("add", &[small.val()]);
    rig.settle();
    rig.step("two_views_of_one_workspace");
    rig.tree("big", big.node());
    rig.tree("small", small.node());

    rig.rec(
        "big",
        json!({
            "same": big.text("workspace") == "desktop",
            "orphaned": big.flag("orphaned"),
            "children": big.get("children").list().expect("a list").len(),
        }),
    );
    rig.settle();
    rig.step("read_back");

    big.set("fit", en(CONTAIN));
    rig.settle();
    rig.step("contain");
    rig.tree("big", big.node());

    big.set("fit", en(COVER));
    rig.settle();
    rig.step("cover");
    rig.tree("big", big.node());

    rig.comp.create_window(900, "late", "late");
    // The TypeScript library's stock focus policy gave the new window the keyboard (NpStdPolicy's native policy does it later).
    rig.external(21, "focus", MirrorValue::Node(900));
    rig.settle();
    rig.step("window_joins");
    rig.tree("big", big.node());

    rig.external_destroy(3);
    rig.settle();
    rig.step("window_leaves");
    rig.tree("big", big.node());

    big.destroy();
    rig.settle();
    rig.step("view_destroyed");

    // `small.content = gnoblin.clients.at(0)`: the first window now is the second of the two the scene had.
    let first = client(&rig, 0);
    let node: u64 = read_member(rig.core, first, "id")
        .expect("an id")
        .text()
        .expect("text")
        .parse()
        .expect("a node");
    small.set("source", Val::Uint(node));
    rig.settle();
    rig.step("view_given_a_client");
    rig.tree("small", small.node());
    rig.finish();
}

// ---- a window's placement, as the client sets it --------------------------------------------------------------------------------

/// Stages a write of a window's property in the transition being staged, as the TypeScript client did through the library.
fn stage(rig: &Rig, node: u64, key: &str, value: &gnoblin_std_abi::GnoblinStdProp) {
    // SAFETY: `key` and `value` are live for the call.
    let code = unsafe { (rig.std.transition_set)(rig.ctx, node, key.as_ptr(), key.len(), value) };
    assert_eq!(code, 0, "{key}");
}

fn prop_of(kind: u32, a: f64, b: f64, c: f64) -> gnoblin_std_abi::GnoblinStdProp {
    gnoblin_std_abi::GnoblinStdProp {
        size: std::mem::size_of::<gnoblin_std_abi::GnoblinStdProp>() as u32,
        kind,
        a,
        b,
        c,
        d: 0.0,
        text: GnoblinStr::borrowed(""),
        node: 0,
    }
}

fn number_prop(value: f64) -> gnoblin_std_abi::GnoblinStdProp {
    prop_of(gnoblin_std_abi::STD_PROP_NUMBER, value, 0.0, 0.0)
}

fn flag_prop(value: bool) -> gnoblin_std_abi::GnoblinStdProp {
    prop_of(gnoblin_std_abi::STD_PROP_BOOL, f64::from(u8::from(value)), 0.0, 0.0)
}

fn triple_prop(value: [f64; 3]) -> gnoblin_std_abi::GnoblinStdProp {
    prop_of(gnoblin_std_abi::STD_PROP_TRIPLE, value[0], value[1], value[2])
}

/// The parts of a placement the library checked, staged for the window `node`: the writes `Client.set` made from a transform.
fn stage_transform(rig: &Rig, node: u64, given: Val) -> Result<(), Fail> {
    let parsed = try_geometry(rig, "transform_from", &[t("Client#3.transform"), given])?;
    let parsed = parsed.record().expect("transform parts");
    if !parsed[0].is_none() {
        let [x, y, z] = triple(&parsed[0]);
        stage(rig, node, "x", &number_prop(x));
        stage(rig, node, "y", &number_prop(y));
        stage(rig, node, "z", &number_prop(z));
    }
    if !parsed[1].is_none() {
        stage(rig, node, "rotation", &triple_prop(triple(&parsed[1])));
    }
    if !parsed[3].is_none() {
        stage(rig, node, "scale", &triple_prop(triple(&parsed[3])));
    }
    Ok(())
}

#[test]
fn a_clients_placement_from_a_transform() {
    let mut rig = desktop("scene_client_set", 1);
    let c = client(&rig, 0);

    // `c.set({ opacity: 0.92, visible: true, transform: { rotation: ... } }, { duration: 250, easing: 'ease-out' })`.
    transition(&rig, 250.0, || {
        stage(&rig, 3, "opacity", &number_prop(0.92));
        stage(&rig, 3, "visible", &flag_prop(true));
        stage_transform(&rig, 3, parts(None, Some(v3(0.0, 0.12, 0.0)), None, None, None)).expect("a transform");
    });
    rig.settle();
    rig.step("set_with_options");

    rig.rec(
        "read",
        json!(
            cget(&rig, c, "rotation")
                .list()
                .expect("a list")
                .iter()
                .map(|x| js(x.double().expect("a number")))
                .collect::<Vec<_>>()
        ),
    );
    rig.settle();
    rig.step("read_rotation");

    // `c.set({ opacity: 1, transform: { position, scale: 1.5 } })`: with no transition staged, the writes go straight to the tree.
    stage(&rig, 3, "opacity", &number_prop(1.0));
    stage_transform(&rig, 3, parts(Some(v3(30.0, 40.0, -5.0)), None, None, None, Some(1.5))).expect("a transform");
    rig.settle();
    rig.step("set_atomic");

    let position = cget(&rig, c, "position");
    let committed = cget(&rig, c, "committed_size");
    let committed = committed.record().expect("a size");
    rig.rec(
        "read",
        json!({
            "position": position.list().expect("a list").iter().map(|x| js(x.double().expect("a number"))).collect::<Vec<_>>(),
            "committed": [js(committed[0].double().expect("a width")), js(committed[1].double().expect("a height"))],
        }),
    );
    rig.settle();
    rig.step("read_position_and_committed_size");

    cset(&rig, c, "width", d(400.0));
    rig.rec("width", js(cget(&rig, c, "width").double().expect("a width")));
    rig.settle();
    rig.step("asked_width");

    // `c.set({ transform: { pose: 1 } })`: a key that is no part of a transform, which a record has no field for.
    let sixth = Val::Record {
        class: 0,
        fields: vec![Val::None, Val::None, Val::None, Val::None, Val::None, d(1.0)],
    };
    // The TypeScript engine rolled back its own copy of the window's state when the callback threw, and wrote that copy to the tree:
    // the copy had not heard of the writes of the earlier steps, so `z`, `rotation` and `scale` went back to what a window starts
    // with. The native engine has no copy to restore (the shell's own state is put back by the `rollback` of the transition, which
    // `try_transition` gives and asserts ran), so
    // the window keeps what the earlier steps wrote.
    rig.allow_shared("refused_transform", 3, "z", json!(0.0), json!(-5.0));
    rig.allow_shared(
        "refused_transform",
        3,
        "rotation",
        json!([0.0, 0.0, 0.0]),
        json!([0.0, 0.12, 0.0]),
    );
    rig.allow_shared(
        "refused_transform",
        3,
        "scale",
        json!([1.0, 1.0, 1.0]),
        json!([1.5, 1.5, 1.5]),
    );
    let fail = try_transition(&rig, 0.0, || stage_transform(&rig, 3, sixth)).expect_err("a sixth field is refused");
    assert_eq!(fail.code, E_TYPE, "{fail}");
    rig.rec(
        "refused",
        json!("Client#N.transform.pose: a transform has position, rotation and scale only"),
    );
    rig.settle();
    rig.step("refused_transform");
    rig.finish();
}
