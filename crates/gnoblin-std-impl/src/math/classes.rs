//! The functions of `geometry` and the values they take, as the object protocol serves them (`math.toml`).
//!
//! The records are positional: a `vec3` is `[x, y, z]`, a `pose` is `[position, rotation, scale]`, a `ray` is `[origin, direction]`,
//! and a field a caller left out reads as none, which each reader turns into the default the catalog names.

use gnoblin_object::server::Builder;
use gnoblin_object::{Fail, Res, Val};

use super::spatial::{self, Given};
use super::transform::Placement;
use super::vector::{Flat, Point3, RayLine};

/// A record with the fields given.
fn record(fields: Vec<Val>) -> Val {
    Val::Record { class: 0, fields }
}

/// A `vec3` as a value.
pub(crate) fn vec3_val(vector: Point3) -> Val {
    record(vec![
        Val::Double(vector.x),
        Val::Double(vector.y),
        Val::Double(vector.z),
    ])
}

/// A list of three numbers as a `vec3` value.
pub(crate) fn triple_val(parts: [f64; 3]) -> Val {
    vec3_val(Point3::new(parts[0], parts[1], parts[2]))
}

/// A field of a record, or `default` where it was left out.
fn field(fields: &[Val], index: usize, default: f64) -> Res<f64> {
    match fields.get(index) {
        None | Some(Val::None) => Ok(default),
        Some(value) => value.double(),
    }
}

/// The `vec3` a value holds.
pub(crate) fn vec3_of(value: &Val) -> Res<Point3> {
    let fields = value.record()?;
    Ok(Point3::new(
        field(fields, 0, 0.0)?,
        field(fields, 1, 0.0)?,
        field(fields, 2, 0.0)?,
    ))
}

/// The `vec3` a value holds, or none where it is left out.
fn optional_vec3(value: Option<&Val>) -> Res<Option<Point3>> {
    match value {
        None | Some(Val::None) => Ok(None),
        Some(value) => vec3_of(value).map(Some),
    }
}

/// The four numbers of a `quaternion` value, `[x, y, z, w]`.
fn quaternion_of(value: &Val) -> Res<[f64; 4]> {
    let fields = value.record()?;
    Ok([
        field(fields, 0, 0.0)?,
        field(fields, 1, 0.0)?,
        field(fields, 2, 0.0)?,
        field(fields, 3, 1.0)?,
    ])
}

/// A `pose` as a value.
pub(crate) fn pose_val(pose: &Placement) -> Val {
    record(vec![
        vec3_val(pose.position),
        triple_val(pose.rotation()),
        Val::Double(pose.scale),
    ])
}

/// The pose a value holds.
pub(crate) fn pose_of(value: &Val) -> Res<Placement> {
    let fields = value.record()?;
    let position = optional_vec3(fields.first())?.unwrap_or_default();
    let rotation = optional_vec3(fields.get(1))?.unwrap_or_default();
    Ok(Placement::new(position, rotation.array(), field(fields, 2, 1.0)?))
}

fn line_of(value: &Val) -> Res<RayLine> {
    let fields = value.record()?;
    Ok(RayLine {
        origin: optional_vec3(fields.first())?.unwrap_or_default(),
        direction: optional_vec3(fields.get(1))?.unwrap_or_default(),
    })
}

fn flat_of(value: &Val) -> Res<Flat> {
    let fields = value.record()?;
    Ok(Flat {
        point: optional_vec3(fields.first())?.unwrap_or_default(),
        normal: optional_vec3(fields.get(1))?.unwrap_or_default(),
    })
}

/// The parts of a `transform_parts` value.
pub(crate) fn given_of(value: &Val) -> Res<Given> {
    let fields = value.record()?;
    let orientation = match fields.get(2) {
        None | Some(Val::None) => None,
        Some(value) => Some(quaternion_of(value)?),
    };
    let uniform_scale = match fields.get(4) {
        None | Some(Val::None) => None,
        Some(value) => Some(value.double()?),
    };
    Ok(Given {
        position: optional_vec3(fields.first())?,
        rotation: optional_vec3(fields.get(1))?,
        orientation,
        scale: optional_vec3(fields.get(3))?,
        uniform_scale,
    })
}

/// What `transform_from` answers: a `transform_parts` with the parts that were given, each as angles or an amount per axis.
pub(crate) fn parsed_val(parsed: &spatial::ParsedTransform) -> Val {
    let part = |value: Option<[f64; 3]>| value.map_or(Val::None, triple_val);
    record(vec![
        part(parsed.position),
        part(parsed.rotation),
        Val::None,
        part(parsed.scale),
        Val::None,
    ])
}

/// A list of numbers.
fn numbers(value: &Val) -> Res<Vec<f64>> {
    value.list()?.iter().map(Val::double).collect()
}

fn convert<T>(result: Result<T, crate::error::Fail>) -> Res<T> {
    result.map_err(Fail::from)
}

pub(crate) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("geometry")
        .ctor(|_cx, _inits| Ok(()))
        .function("add", |_cx, args| {
            Ok(vec3_val(vec3_of(&args[0])?.add(vec3_of(&args[1])?)))
        })
        .function("subtract", |_cx, args| {
            Ok(vec3_val(vec3_of(&args[0])?.subtract(vec3_of(&args[1])?)))
        })
        .function("scale", |_cx, args| {
            Ok(vec3_val(vec3_of(&args[0])?.scale(args[1].double()?)))
        })
        .function("dot", |_cx, args| {
            Ok(Val::Double(vec3_of(&args[0])?.dot(vec3_of(&args[1])?)))
        })
        .function("cross", |_cx, args| {
            Ok(vec3_val(vec3_of(&args[0])?.cross(vec3_of(&args[1])?)))
        })
        .function("length", |_cx, args| Ok(Val::Double(vec3_of(&args[0])?.length())))
        .function("normalize", |_cx, args| Ok(vec3_val(vec3_of(&args[0])?.normalize())))
        .function("distance", |_cx, args| {
            Ok(Val::Double(vec3_of(&args[0])?.distance(vec3_of(&args[1])?)))
        })
        .function("vec3_of", |_cx, args| Ok(vec3_val(Point3::of(&numbers(&args[0])?))))
        .function("ray_at", |_cx, args| {
            Ok(vec3_val(line_of(&args[0])?.at(args[1].double()?)))
        })
        .function("intersect", |_cx, args| {
            let plane = flat_of(&args[0])?;
            let line = line_of(&args[1])?;
            Ok(match plane.intersect(line) {
                Some((point, distance)) => record(vec![Val::Bool(true), Val::Double(distance), vec3_val(point)]),
                None => record(vec![Val::Bool(false), Val::Double(0.0), Val::None]),
            })
        })
        .function("pose_identity", |_cx, _args| Ok(pose_val(&Placement::identity())))
        .function("pose_translation", |_cx, args| {
            let position = Point3::new(args[0].double()?, args[1].double()?, args[2].double()?);
            Ok(pose_val(&Placement::new(position, [0.0; 3], 1.0)))
        })
        .function("pose_multiply", |_cx, args| {
            Ok(pose_val(&pose_of(&args[0])?.multiply(&pose_of(&args[1])?)))
        })
        .function("pose_inverse", |_cx, args| Ok(pose_val(&pose_of(&args[0])?.inverse())))
        .function("pose_normalise", |_cx, args| Ok(pose_val(&pose_of(&args[0])?)))
        .function("transform_direction", |_cx, args| {
            Ok(vec3_val(pose_of(&args[0])?.transform_direction(vec3_of(&args[1])?)))
        })
        .function("transform_point", |_cx, args| {
            Ok(vec3_val(pose_of(&args[0])?.transform_point(vec3_of(&args[1])?)))
        })
        .function("pose_plane", |_cx, args| {
            let plane = pose_of(&args[0])?.plane();
            Ok(record(vec![vec3_val(plane.point), vec3_val(plane.normal)]))
        })
        .function("euler_of_quaternion", |_cx, args| {
            let [x, y, z, w] = quaternion_of(&args[0])?;
            Ok(triple_val(convert(spatial::euler_of_quaternion(x, y, z, w))?))
        })
        .function("vector_from", |_cx, args| {
            Ok(triple_val(convert(spatial::vector_from(
                args[0].text()?,
                &numbers(&args[1])?,
            ))?))
        })
        .function("vector_checked", |_cx, args| {
            Ok(triple_val(convert(spatial::vector_checked(
                args[0].text()?,
                vec3_of(&args[1])?,
            ))?))
        })
        .function("rotation_from_list", |_cx, args| {
            Ok(triple_val(convert(spatial::rotation_from_list(
                args[0].text()?,
                &numbers(&args[1])?,
            ))?))
        })
        .function("rotation_from_angles", |_cx, args| {
            Ok(triple_val(convert(spatial::rotation_from_angles(
                args[0].text()?,
                vec3_of(&args[1])?,
            ))?))
        })
        .function("rotation_from_quaternion", |_cx, args| {
            Ok(triple_val(convert(spatial::rotation_from_quaternion(
                args[0].text()?,
                quaternion_of(&args[1])?,
            ))?))
        })
        .function("scale_from_number", |_cx, args| {
            Ok(triple_val(convert(spatial::scale_from_number(
                args[0].text()?,
                args[1].double()?,
            ))?))
        })
        .function("scale_from_list", |_cx, args| {
            Ok(triple_val(convert(spatial::scale_from_list(
                args[0].text()?,
                &numbers(&args[1])?,
            ))?))
        })
        .function("scale_from_vector", |_cx, args| {
            Ok(triple_val(convert(spatial::scale_from_vector(
                args[0].text()?,
                vec3_of(&args[1])?,
            ))?))
        })
        .function("transform_from", |_cx, args| {
            let parsed = convert(spatial::transform_from(args[0].text()?, &given_of(&args[1])?))?;
            Ok(parsed_val(&parsed))
        });
}
