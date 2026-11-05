//! Reads a layout's return value into a [`gnoblin_wm::layout::Reply`].
//!
//! The shape is checked here: a return of `nil` declines; a table is a handled reply
//! with `placements`, an optional `camera` and an optional `effects` list. Numeric
//! and id checks are structural only. Geometry (NaN, negative sizes, unknown ids,
//! duplicates, singular transforms) is judged by `gnoblin_wm::layout::settle`, so the
//! validator stays the single source of those rules.

use gnoblin_types::{Vec3, WindowId};
use gnoblin_wm::layout::{Camera, Handled, LayoutEffect, Placement, Reply, Transform3};
use mlua::{Table, Value as LuaValue};

use super::require::runtime;

/// Read a layout's return value.
pub(crate) fn read(value: &LuaValue) -> mlua::Result<Reply> {
    match value {
        LuaValue::Nil => Ok(Reply::Declined),
        LuaValue::Table(table) => handled(table).map(Reply::Handled),
        other => Err(runtime(format!(
            "a layout must return nil (decline) or a reply table, not {}",
            other.type_name()
        ))),
    }
}

fn handled(table: &Table) -> mlua::Result<Handled> {
    let placements = list(table, "placements")?
        .ok_or_else(|| runtime("a reply needs a `placements` list"))?
        .iter()
        .enumerate()
        .map(|(index, entry)| placement(entry).map_err(|error| context(error, &format!("placements[{}]", index + 1))))
        .collect::<mlua::Result<Vec<_>>>()?;
    let camera = match table.get::<LuaValue>("camera")? {
        LuaValue::Nil => Camera::Orthographic,
        LuaValue::Table(camera) => camera_from(&camera)?,
        other => return Err(runtime(format!("`camera` must be a table, not {}", other.type_name()))),
    };
    let effects = match list(table, "effects")? {
        None => Vec::new(),
        Some(entries) => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| effect(entry).map_err(|error| context(error, &format!("effects[{}]", index + 1))))
            .collect::<mlua::Result<Vec<_>>>()?,
    };
    Ok(Handled {
        placements,
        camera,
        effects,
    })
}

/// The list at `key`, or `None` when the field is absent.
fn list(table: &Table, key: &str) -> mlua::Result<Option<Vec<Table>>> {
    match table.get::<LuaValue>(key)? {
        LuaValue::Nil => Ok(None),
        LuaValue::Table(list) => {
            let mut entries = Vec::new();
            for index in 1..=list.raw_len() {
                match list.raw_get::<LuaValue>(index)? {
                    LuaValue::Table(entry) => entries.push(entry),
                    other => {
                        return Err(runtime(format!(
                            "`{key}[{index}]` must be a table, not {}",
                            other.type_name()
                        )));
                    }
                }
            }
            Ok(Some(entries))
        }
        other => Err(runtime(format!("`{key}` must be a list, not {}", other.type_name()))),
    }
}

fn placement(table: &Table) -> mlua::Result<Placement> {
    let window = window_id(table, "window")?;
    let width = number(table, "width")?;
    let height = number(table, "height")?;
    let transform = match table.get::<LuaValue>("transform")? {
        LuaValue::Nil => {
            let x = optional(table, "x")?.unwrap_or(0.0);
            let y = optional(table, "y")?.unwrap_or(0.0);
            Transform3::at(x, y)
        }
        LuaValue::Table(transform) => transform_from(&transform)?,
        other => {
            return Err(runtime(format!(
                "`transform` must be a table, not {}",
                other.type_name()
            )));
        }
    };
    Ok(Placement {
        window,
        width,
        height,
        transform,
        visible: flag(table, "visible", true)?,
        opacity: optional(table, "opacity")?.unwrap_or(1.0),
        decorated: flag(table, "decorated", true)?,
    })
}

fn effect(table: &Table) -> mlua::Result<LayoutEffect> {
    match text(table, "kind")?.as_str() {
        "float" => Ok(LayoutEffect::Float {
            window: window_id(table, "window")?,
        }),
        other => Err(runtime(format!("unknown effect kind `{other}`"))),
    }
}

fn camera_from(table: &Table) -> mlua::Result<Camera> {
    match text(table, "kind")?.as_str() {
        "orthographic" => Ok(Camera::Orthographic),
        "perspective" => {
            let pose = match table.get::<LuaValue>("pose")? {
                LuaValue::Nil => Transform3::IDENTITY,
                LuaValue::Table(pose) => transform_from(&pose)?,
                other => return Err(runtime(format!("`pose` must be a table, not {}", other.type_name()))),
            };
            Ok(Camera::Perspective {
                fov_degrees: number(table, "fov_degrees")?,
                near: number(table, "near")?,
                far: number(table, "far")?,
                pose,
            })
        }
        other => Err(runtime(format!("unknown camera kind `{other}`"))),
    }
}

/// A transform: either `matrix` (16 column-major numbers) or `position`, `rotation` and
/// `scale` as `{x, y, z}` tables, each optional with the identity as its default.
fn transform_from(table: &Table) -> mlua::Result<Transform3> {
    if let LuaValue::Table(matrix) = table.get::<LuaValue>("matrix")? {
        let mut values = [0.0; 16];
        if matrix.raw_len() != values.len() {
            return Err(runtime(format!("`matrix` needs 16 numbers, not {}", matrix.raw_len())));
        }
        for (index, slot) in values.iter_mut().enumerate() {
            *slot = number_at(&matrix, index + 1)?;
        }
        return Ok(Transform3::Matrix(values));
    }
    let position = vec3(table, "position", Vec3::new(0.0, 0.0, 0.0))?;
    let rotation = vec3(table, "rotation", Vec3::new(0.0, 0.0, 0.0))?;
    let scale = vec3(table, "scale", Vec3::new(1.0, 1.0, 1.0))?;
    Ok(Transform3::Trs {
        position,
        rotation_degrees: rotation,
        scale,
    })
}

fn vec3(table: &Table, key: &str, default: Vec3) -> mlua::Result<Vec3> {
    match table.get::<LuaValue>(key)? {
        LuaValue::Nil => Ok(default),
        LuaValue::Table(vector) => Ok(Vec3::new(
            number(&vector, "x")?,
            number(&vector, "y")?,
            optional(&vector, "z")?.unwrap_or(0.0),
        )),
        other => Err(runtime(format!("`{key}` must be a table, not {}", other.type_name()))),
    }
}

fn window_id(table: &Table, key: &str) -> mlua::Result<WindowId> {
    match table.get::<LuaValue>(key)? {
        LuaValue::Integer(id) => u64::try_from(id)
            .map(WindowId::new)
            .map_err(|_| runtime(format!("`{key}` must be a non-negative window id"))),
        LuaValue::Nil => Err(runtime(format!("`{key}` is required"))),
        other => Err(runtime(format!(
            "`{key}` must be a window id, not {}",
            other.type_name()
        ))),
    }
}

fn number(table: &Table, key: &str) -> mlua::Result<f64> {
    optional(table, key)?.ok_or_else(|| runtime(format!("`{key}` is required")))
}

fn optional(table: &Table, key: &str) -> mlua::Result<Option<f64>> {
    match table.get::<LuaValue>(key)? {
        LuaValue::Nil => Ok(None),
        value => as_number(&value, key).map(Some),
    }
}

fn number_at(table: &Table, index: usize) -> mlua::Result<f64> {
    as_number(&table.raw_get::<LuaValue>(index)?, &format!("matrix[{index}]"))
}

fn as_number(value: &LuaValue, key: &str) -> mlua::Result<f64> {
    match value {
        // Floats include NaN and the infinities; the validator refuses them with a
        // named reason, so they are passed through rather than dropped here.
        LuaValue::Number(number) => Ok(*number),
        LuaValue::Integer(number) => Ok(*number as f64),
        other => Err(runtime(format!("`{key}` must be a number, not {}", other.type_name()))),
    }
}

fn flag(table: &Table, key: &str, default: bool) -> mlua::Result<bool> {
    match table.get::<LuaValue>(key)? {
        LuaValue::Nil => Ok(default),
        LuaValue::Boolean(value) => Ok(value),
        other => Err(runtime(format!("`{key}` must be a boolean, not {}", other.type_name()))),
    }
}

fn text(table: &Table, key: &str) -> mlua::Result<String> {
    match table.get::<LuaValue>(key)? {
        LuaValue::String(text) => Ok(text.to_string_lossy()),
        LuaValue::Nil => Err(runtime(format!("`{key}` is required"))),
        other => Err(runtime(format!("`{key}` must be a string, not {}", other.type_name()))),
    }
}

fn context(error: mlua::Error, at: &str) -> mlua::Error {
    runtime(format!("{at}: {}", super::require::message(error)))
}
