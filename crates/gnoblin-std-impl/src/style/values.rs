//! The values a style holds, their limits, and the checkers that accept one or fail with the property path.
//!
//! `data/js/gnoblin-std/style/values.ts`, `border_shadow.ts`, `filters.ts` and `lists.ts`: the same limits and the same texts, so a
//! bad value is refused here as it is there (`Client#42.style.opacity: must be finite and between 0 and 1`).
#![allow(unsafe_code)]

use gnoblin_std_abi as abi;

use crate::error::{Fail, Res};

/// How many entries a style list holds, and how many effects a window's node carries.
pub(crate) const MAX_STYLE_LIST: usize = 8;

/// The renderer's own limits: a blur or shadow radius, a spread.
const MAX_RADIUS: f64 = 100.0;

/// The largest `amount` a saturate, brightness or contrast filter takes.
pub(crate) const MAX_GRADE: f64 = 2.0;

/// The renderer's limits on a shader (`gnoblin-render`'s `MAX_SHADER_SOURCE_BYTES`, `MAX_SHADER_PARAMS` and `MAX_SHADERS`): the
/// source's size in bytes, the numbers it reads as `p`, and the shader passes one window's node carries.
pub(crate) const MAX_SHADER_SOURCE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_SHADER_PARAMS: usize = 8;
pub(crate) const MAX_SHADERS: usize = 4;

/// A colour as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
pub(crate) type Color = String;

/// Fails with `path: message`.
pub(crate) fn fail(path: &str, message: &str) -> Fail {
    Fail::type_error(format!("{path}: {message}"))
}

/// A finite number from `min` to `max`, both included.
pub(crate) fn finite_in(path: &str, value: f64, min: f64, max: f64) -> Res<f64> {
    if value.is_finite() && value >= min && value <= max {
        return Ok(value);
    }
    Err(if min == f64::NEG_INFINITY && max == f64::INFINITY {
        fail(path, "must be a finite number")
    } else if max == f64::INFINITY {
        fail(path, &format!("must be finite and >= {min}"))
    } else {
        fail(path, &format!("must be finite and between {min} and {max}"))
    })
}

/// Any finite number.
pub(crate) fn finite(path: &str, value: f64) -> Res<f64> {
    finite_in(path, value, f64::NEG_INFINITY, f64::INFINITY)
}

/// A finite number that is not below zero.
pub(crate) fn non_negative(path: &str, value: f64) -> Res<f64> {
    finite_in(path, value, 0.0, f64::INFINITY)
}

/// A finite radius from zero to the largest one.
pub(crate) fn radius(path: &str, value: f64) -> Res<f64> {
    finite_in(path, value, 0.0, MAX_RADIUS)
}

/// A finite spread, within the largest radius either way.
pub(crate) fn spread(path: &str, value: f64) -> Res<f64> {
    finite_in(path, value, -MAX_RADIUS, MAX_RADIUS)
}

/// A finite filter amount from zero to the largest.
pub(crate) fn grade(path: &str, value: f64) -> Res<f64> {
    finite_in(path, value, 0.0, MAX_GRADE)
}

/// A finite number from zero to one.
pub(crate) fn unit(path: &str, value: f64) -> Res<f64> {
    finite_in(path, value, 0.0, 1.0)
}

/// A colour: a CSS hex string of 3, 4, 6 or 8 digits after the `#`.
pub(crate) fn colour(path: &str, value: &str) -> Res<Color> {
    let digits = value.strip_prefix('#').unwrap_or("");
    if matches!(digits.len(), 3 | 4 | 6 | 8) && digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value.to_owned())
    } else {
        Err(fail(path, "must be a #rgb, #rgba, #rrggbb or #rrggbbaa colour"))
    }
}

/// One corner radius per corner, or one for all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Radii {
    All(f64),
    /// Top left, top right, bottom right, bottom left.
    Corners([f64; 4]),
}

/// One element of a style list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Entry {
    Border {
        width: f64,
        color: Color,
        offset: f64,
        inset: bool,
    },
    Shadow {
        x: f64,
        y: f64,
        blur: f64,
        spread: f64,
        color: Color,
        inset: bool,
    },
    Blur(f64),
    Saturate(f64),
    Brightness(f64),
    Contrast(f64),
    /// A shader the shell wrote: `source` defines `fn effect(c, uv, p, t)` and `params` is what it reads as `p`.
    Shader {
        source: String,
        params: Vec<f64>,
    },
}

/// The lists a style has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ListProp {
    Borders,
    Shadows,
    Filter,
    BackdropFilter,
}

/// The scalars a style has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ScalarProp {
    Opacity,
    CornerRadius,
    Background,
}

/// A style property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prop {
    Scalar(ScalarProp),
    List(ListProp),
}

impl Prop {
    pub(crate) fn from_abi(value: u32) -> Option<Prop> {
        Some(match value {
            abi::STD_STYLE_OPACITY => Prop::Scalar(ScalarProp::Opacity),
            abi::STD_STYLE_CORNER_RADIUS => Prop::Scalar(ScalarProp::CornerRadius),
            abi::STD_STYLE_BACKGROUND => Prop::Scalar(ScalarProp::Background),
            abi::STD_STYLE_BORDERS => Prop::List(ListProp::Borders),
            abi::STD_STYLE_SHADOWS => Prop::List(ListProp::Shadows),
            abi::STD_STYLE_FILTER => Prop::List(ListProp::Filter),
            abi::STD_STYLE_BACKDROP_FILTER => Prop::List(ListProp::BackdropFilter),
            _ => return None,
        })
    }

    /// Every property, in the order the TypeScript style lists them.
    pub(crate) const ALL: [Prop; 7] = [
        Prop::Scalar(ScalarProp::Opacity),
        Prop::Scalar(ScalarProp::CornerRadius),
        Prop::Scalar(ScalarProp::Background),
        Prop::List(ListProp::Borders),
        Prop::List(ListProp::Shadows),
        Prop::List(ListProp::Filter),
        Prop::List(ListProp::BackdropFilter),
    ];

    /// The name a script and an error path call it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Prop::Scalar(ScalarProp::Opacity) => "opacity",
            Prop::Scalar(ScalarProp::CornerRadius) => "cornerRadius",
            Prop::Scalar(ScalarProp::Background) => "background",
            Prop::List(ListProp::Borders) => "borders",
            Prop::List(ListProp::Shadows) => "shadows",
            Prop::List(ListProp::Filter) => "filter",
            Prop::List(ListProp::BackdropFilter) => "backdropFilter",
        }
    }
}

/// The element a raw list entry stands for, checked, with `path` at the front of every failure.
pub(crate) fn entry_in(list: ListProp, raw: &abi::GnoblinStdStyleEntry, path: &str) -> Res<Entry> {
    let color = |what: &str| -> Res<Color> {
        let text = if raw.color.ptr.is_null() || raw.color.len == 0 {
            ""
        } else {
            // SAFETY: the caller of the table function guarantees the colour is `len` readable bytes.
            std::str::from_utf8(unsafe { std::slice::from_raw_parts(raw.color.ptr, raw.color.len) })
                .map_err(|_| fail(&format!("{path}.{what}"), "must be UTF-8 text"))?
        };
        colour(&format!("{path}.{what}"), text)
    };
    let field = |name: &str| format!("{path}.{name}");
    match (list, raw.kind) {
        (ListProp::Borders, abi::STD_ENTRY_BORDER) => Ok(Entry::Border {
            width: non_negative(&field("width"), raw.a)?,
            color: color("color")?,
            offset: finite(&field("offset"), raw.b)?,
            inset: raw.inset != 0,
        }),
        (ListProp::Shadows, abi::STD_ENTRY_SHADOW) => Ok(Entry::Shadow {
            x: finite(&field("x"), raw.a)?,
            y: finite(&field("y"), raw.b)?,
            blur: radius(&field("blur"), raw.c)?,
            spread: spread(&field("spread"), raw.d)?,
            color: color("color")?,
            inset: raw.inset != 0,
        }),
        (ListProp::Filter | ListProp::BackdropFilter, abi::STD_ENTRY_BLUR) => {
            Ok(Entry::Blur(radius(&field("radius"), raw.a)?))
        }
        (ListProp::Filter | ListProp::BackdropFilter, abi::STD_ENTRY_SATURATE) => {
            Ok(Entry::Saturate(grade(&field("amount"), raw.a)?))
        }
        (ListProp::Filter | ListProp::BackdropFilter, abi::STD_ENTRY_BRIGHTNESS) => {
            Ok(Entry::Brightness(grade(&field("amount"), raw.a)?))
        }
        (ListProp::Filter | ListProp::BackdropFilter, abi::STD_ENTRY_CONTRAST) => {
            Ok(Entry::Contrast(grade(&field("amount"), raw.a)?))
        }
        (ListProp::Filter | ListProp::BackdropFilter, abi::STD_ENTRY_SHADER) => shader_in(raw, path),
        (ListProp::Borders, _) => Err(fail(path, "must be a Border")),
        (ListProp::Shadows, _) => Err(fail(path, "must be a Shadow")),
        _ => Err(fail(
            path,
            "must be a filter: a blur, saturate, brightness, contrast or shader filter",
        )),
    }
}

/// A shader filter: WGSL source of at most [`MAX_SHADER_SOURCE_BYTES`] and at most [`MAX_SHADER_PARAMS`] finite numbers. Whether the
/// source is valid WGSL is the compositor's to say (it checks the source before it accepts it); the library cannot, for it has no
/// shader compiler.
fn shader_in(raw: &abi::GnoblinStdStyleEntry, path: &str) -> Res<Entry> {
    let source = if raw.source.ptr.is_null() || raw.source.len == 0 {
        ""
    } else {
        // SAFETY: the caller of the table function guarantees the source is `len` readable bytes.
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(raw.source.ptr, raw.source.len) })
            .map_err(|_| fail(&format!("{path}.source"), "must be UTF-8 text"))?
    };
    if source.trim().is_empty() || source.len() > MAX_SHADER_SOURCE_BYTES {
        return Err(fail(
            &format!("{path}.source"),
            &format!("must be WGSL of 1 to {MAX_SHADER_SOURCE_BYTES} bytes"),
        ));
    }
    if raw.n_params > MAX_SHADER_PARAMS || (raw.n_params > 0 && raw.params.is_null()) {
        return Err(fail(
            &format!("{path}.params"),
            &format!("holds at most {MAX_SHADER_PARAMS} numbers, got {}", raw.n_params),
        ));
    }
    let numbers: &[f64] = if raw.n_params == 0 {
        &[]
    } else {
        // SAFETY: the caller of the table function guarantees `n_params` readable numbers at `params`.
        unsafe { std::slice::from_raw_parts(raw.params, raw.n_params) }
    };
    let params = numbers
        .iter()
        .enumerate()
        .map(|(at, value)| finite(&format!("{path}.params[{at}]"), *value))
        .collect::<Res<Vec<f64>>>()?;
    Ok(Entry::Shader {
        source: source.to_owned(),
        params,
    })
}

/// The source and numbers of a shader entry as the wire carries them; empty for any other kind.
pub(crate) fn entry_shader(entry: &Entry) -> (&str, &[f64]) {
    match entry {
        Entry::Shader { source, params } => (source, params),
        _ => ("", &[]),
    }
}

/// The wire form of a list element, for `style_list`: `(kind, inset, a, b, c, d, colour)`.
pub(crate) fn entry_out(entry: &Entry) -> (u32, bool, [f64; 4], Option<&str>) {
    match entry {
        Entry::Border {
            width,
            color,
            offset,
            inset,
        } => (abi::STD_ENTRY_BORDER, *inset, [*width, *offset, 0.0, 0.0], Some(color)),
        Entry::Shadow {
            x,
            y,
            blur,
            spread,
            color,
            inset,
        } => (abi::STD_ENTRY_SHADOW, *inset, [*x, *y, *blur, *spread], Some(color)),
        Entry::Blur(radius) => (abi::STD_ENTRY_BLUR, false, [*radius, 0.0, 0.0, 0.0], None),
        Entry::Saturate(amount) => (abi::STD_ENTRY_SATURATE, false, [*amount, 0.0, 0.0, 0.0], None),
        Entry::Brightness(amount) => (abi::STD_ENTRY_BRIGHTNESS, false, [*amount, 0.0, 0.0, 0.0], None),
        Entry::Contrast(amount) => (abi::STD_ENTRY_CONTRAST, false, [*amount, 0.0, 0.0, 0.0], None),
        Entry::Shader { .. } => (abi::STD_ENTRY_SHADER, false, [0.0; 4], None),
    }
}

/// A corner radius: one number for every corner, or a value per corner.
pub(crate) fn corner_radius(path: &str, value: Radii) -> Res<Radii> {
    match value {
        Radii::All(radius) => non_negative(path, radius).map(Radii::All),
        Radii::Corners(corners) => {
            let names = ["topLeft", "topRight", "bottomRight", "bottomLeft"];
            let mut checked = [0.0; 4];
            for (at, corner) in corners.iter().enumerate() {
                checked[at] = non_negative(&format!("{path}.{}", names[at]), *corner)?;
            }
            Ok(Radii::Corners(checked))
        }
    }
}
