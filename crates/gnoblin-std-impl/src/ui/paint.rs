//! Paint lists: the `paint_op` records a shell gives or the library builds, checked, and the bytes the compositor's `content` holds.
//!
//! The encoding is `gnoblin-paint`'s (`list.rs`), the crate the compositor decodes and draws with, so what is written is what is
//! read. This module reads a `paint_op` record the way the TypeScript toolkit's encoder did: a number that is not finite or a colour
//! that is not `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` refuses the whole list with `draw op N: ...`, and a field left out has the
//! default the catalog names.

use gnoblin_object::Val;
use gnoblin_paint::{PaintList, PaintOp, Rgba8, encode};

use super::widget::{DEFAULT_FONT, DEFAULT_ICON_SIZE, DEFAULT_TEXT_SIZE};

const OPAQUE_WHITE: Rgba8 = Rgba8::new(255, 255, 255, 255);
const DEFAULT_LINE_WIDTH: f64 = 1.0;

/// The field positions of `paint_op` in the catalog.
mod at {
    pub(super) const OP: usize = 0;
    pub(super) const X: usize = 1;
    pub(super) const Y: usize = 2;
    pub(super) const WIDTH: usize = 3;
    pub(super) const HEIGHT: usize = 4;
    pub(super) const RADIUS: usize = 5;
    pub(super) const FILL: usize = 6;
    pub(super) const STROKE: usize = 7;
    pub(super) const STROKE_WIDTH: usize = 8;
    pub(super) const X1: usize = 9;
    pub(super) const Y1: usize = 10;
    pub(super) const X2: usize = 11;
    pub(super) const Y2: usize = 12;
    pub(super) const SIZE: usize = 13;
    pub(super) const COLOR: usize = 14;
    pub(super) const TEXT: usize = 15;
    pub(super) const FONT: usize = 16;
    pub(super) const NAME: usize = 17;
    pub(super) const ALPHA: usize = 18;
    pub(super) const COUNT: usize = 19;
}

/// The values of `paint_kind`.
mod kind {
    pub(super) const RECT: u32 = 1;
    pub(super) const LINE: u32 = 2;
    pub(super) const TEXT: u32 = 3;
    pub(super) const ICON: u32 = 4;
    pub(super) const PUSH_CLIP: u32 = 5;
    pub(super) const POP_CLIP: u32 = 6;
    pub(super) const PUSH_OPACITY: u32 = 7;
    pub(super) const POP_OPACITY: u32 = 8;
}

/// What a number reads as in a message, as the script's own language spells it.
fn spell(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_owned()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned()
    } else {
        value.to_string()
    }
}

/// The colour a word names: `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
///
/// # Errors
///
/// The word is none of those.
pub(crate) fn colour(word: &str) -> Result<Rgba8, String> {
    let bad = || format!("a colour is #rgb, #rgba, #rrggbb or #rrggbbaa, not {word}");
    let digits = word
        .strip_prefix('#')
        .filter(|digits| !digits.is_empty())
        .ok_or_else(bad)?;
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let mut hex: String = if digits.len() == 3 || digits.len() == 4 {
        digits.chars().flat_map(|c| [c, c]).collect()
    } else {
        digits.to_owned()
    };
    if hex.len() == 6 {
        hex.push_str("ff");
    }
    if hex.len() != 8 {
        return Err(bad());
    }
    u32::from_str_radix(&hex, 16).map(Rgba8::from_word).map_err(|_| bad())
}

/// A number as the encoding's `f32`.
///
/// # Errors
///
/// The number is not finite: the compositor cannot read it.
fn finite(value: f64) -> Result<f32, String> {
    if value.is_finite() {
        Ok(value as f32)
    } else {
        Err(format!("expected a finite number, not {}", spell(value)))
    }
}

/// The text of a colour, as `#rrggbbaa`.
fn word(colour: Rgba8) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", colour.r, colour.g, colour.b, colour.a)
}

/// Reads one field of a record: `None` when the caller left it out.
fn field(fields: &[Val], index: usize) -> Option<&Val> {
    fields.get(index).filter(|value| !value.is_none())
}

fn number(fields: &[Val], index: usize, default: f64) -> Result<f32, String> {
    match field(fields, index) {
        Some(value) => finite(value.double().map_err(|fail| fail.message)?),
        None => finite(default),
    }
}

fn colour_of(fields: &[Val], index: usize, default: Rgba8) -> Result<Rgba8, String> {
    match field(fields, index) {
        Some(value) => colour(value.text().map_err(|fail| fail.message)?),
        None => Ok(default),
    }
}

fn text_of(fields: &[Val], index: usize, what: &str) -> Result<String, String> {
    match field(fields, index) {
        Some(value) => value.text().map(str::to_owned).map_err(|fail| fail.message),
        None => Err(format!("a {what} needs its {what} field")),
    }
}

/// One operation read from a `paint_op` record.
fn read_op(record: &Val) -> Result<PaintOp, String> {
    let fields = record.record().map_err(|fail| fail.message)?;
    let op = field(fields, at::OP)
        .ok_or_else(|| {
            "an op is a rect, line, text, icon, push-clip, pop-clip, push-opacity or pop-opacity".to_owned()
        })?
        .enum_value()
        .map_err(|fail| fail.message)?;
    Ok(match op {
        kind::RECT => PaintOp::Rect {
            x: number(fields, at::X, 0.0)?,
            y: number(fields, at::Y, 0.0)?,
            width: number(fields, at::WIDTH, 0.0)?,
            height: number(fields, at::HEIGHT, 0.0)?,
            radius: number(fields, at::RADIUS, 0.0)?,
            fill: colour_of(fields, at::FILL, Rgba8::TRANSPARENT)?,
            stroke: colour_of(fields, at::STROKE, Rgba8::TRANSPARENT)?,
            stroke_width: number(fields, at::STROKE_WIDTH, 0.0)?,
        },
        kind::LINE => PaintOp::TextLine {
            x1: number(fields, at::X1, 0.0)?,
            y1: number(fields, at::Y1, 0.0)?,
            x2: number(fields, at::X2, 0.0)?,
            y2: number(fields, at::Y2, 0.0)?,
            width: number(fields, at::SIZE, DEFAULT_LINE_WIDTH)?,
            colour: colour_of(fields, at::COLOR, OPAQUE_WHITE)?,
        },
        kind::TEXT => PaintOp::Text {
            x: number(fields, at::X, 0.0)?,
            y: number(fields, at::Y, 0.0)?,
            size: number(fields, at::SIZE, DEFAULT_TEXT_SIZE)?,
            colour: colour_of(fields, at::COLOR, OPAQUE_WHITE)?,
            font: match field(fields, at::FONT) {
                Some(value) => value.text().map_err(|fail| fail.message)?.to_owned(),
                None => DEFAULT_FONT.to_owned(),
            },
            text: text_of(fields, at::TEXT, "text")?,
        },
        kind::ICON => PaintOp::Icon {
            x: number(fields, at::X, 0.0)?,
            y: number(fields, at::Y, 0.0)?,
            size: number(fields, at::SIZE, DEFAULT_ICON_SIZE)?,
            colour: colour_of(fields, at::COLOR, OPAQUE_WHITE)?,
            name: text_of(fields, at::NAME, "name")?,
        },
        kind::PUSH_CLIP => PaintOp::PushClip {
            x: number(fields, at::X, 0.0)?,
            y: number(fields, at::Y, 0.0)?,
            width: number(fields, at::WIDTH, 0.0)?,
            height: number(fields, at::HEIGHT, 0.0)?,
            radius: number(fields, at::RADIUS, 0.0)?,
        },
        kind::POP_CLIP => PaintOp::PopClip,
        kind::PUSH_OPACITY => PaintOp::PushOpacity(number(fields, at::ALPHA, 1.0)?),
        kind::POP_OPACITY => PaintOp::PopOpacity,
        _ => {
            return Err(
                "an op is a rect, line, text, icon, push-clip, pop-clip, push-opacity or pop-opacity".to_owned(),
            );
        }
    })
}

/// The operations of a list of `paint_op` records.
///
/// # Errors
///
/// An operation is malformed: the message names its place (`draw op 2: ...`).
pub(crate) fn read(list: &Val) -> Result<Vec<PaintOp>, String> {
    let items = list.list().map_err(|fail| fail.message)?;
    items
        .iter()
        .enumerate()
        .map(|(index, record)| read_op(record).map_err(|message| format!("draw op {}: {message}", index + 1)))
        .collect()
}

/// A `paint_op` record of a field set: `fields` are the ones given, by position, and the rest are left out.
fn record(op: u32, given: &[(usize, Val)]) -> Val {
    let mut fields = vec![Val::None; at::COUNT];
    fields[at::OP] = Val::Enum { class: 0, value: op };
    for (index, value) in given {
        fields[*index] = value.clone();
    }
    Val::Record { class: 0, fields }
}

fn double(value: f32) -> Val {
    Val::Double(f64::from(value))
}

/// The `paint_op` record of an operation.
fn op_record(op: &PaintOp) -> Val {
    match op {
        PaintOp::Rect {
            x,
            y,
            width,
            height,
            radius,
            fill,
            stroke,
            stroke_width,
        } => record(
            kind::RECT,
            &[
                (at::X, double(*x)),
                (at::Y, double(*y)),
                (at::WIDTH, double(*width)),
                (at::HEIGHT, double(*height)),
                (at::RADIUS, double(*radius)),
                (at::FILL, Val::Text(word(*fill))),
                (at::STROKE, Val::Text(word(*stroke))),
                (at::STROKE_WIDTH, double(*stroke_width)),
            ],
        ),
        PaintOp::TextLine {
            x1,
            y1,
            x2,
            y2,
            width,
            colour,
        } => record(
            kind::LINE,
            &[
                (at::X1, double(*x1)),
                (at::Y1, double(*y1)),
                (at::X2, double(*x2)),
                (at::Y2, double(*y2)),
                (at::SIZE, double(*width)),
                (at::COLOR, Val::Text(word(*colour))),
            ],
        ),
        PaintOp::Text {
            x,
            y,
            size,
            colour,
            font,
            text,
        } => record(
            kind::TEXT,
            &[
                (at::X, double(*x)),
                (at::Y, double(*y)),
                (at::SIZE, double(*size)),
                (at::COLOR, Val::Text(word(*colour))),
                (at::FONT, Val::Text(font.clone())),
                (at::TEXT, Val::Text(text.clone())),
            ],
        ),
        PaintOp::Icon {
            x,
            y,
            size,
            colour,
            name,
        } => record(
            kind::ICON,
            &[
                (at::X, double(*x)),
                (at::Y, double(*y)),
                (at::SIZE, double(*size)),
                (at::COLOR, Val::Text(word(*colour))),
                (at::NAME, Val::Text(name.clone())),
            ],
        ),
        PaintOp::PushClip {
            x,
            y,
            width,
            height,
            radius,
        } => record(
            kind::PUSH_CLIP,
            &[
                (at::X, double(*x)),
                (at::Y, double(*y)),
                (at::WIDTH, double(*width)),
                (at::HEIGHT, double(*height)),
                (at::RADIUS, double(*radius)),
            ],
        ),
        PaintOp::PopClip => record(kind::POP_CLIP, &[]),
        PaintOp::PushOpacity(alpha) => record(kind::PUSH_OPACITY, &[(at::ALPHA, double(*alpha))]),
        PaintOp::PopOpacity => record(kind::POP_OPACITY, &[]),
    }
}

/// The list of `paint_op` records of some operations.
pub(crate) fn to_val(ops: &[PaintOp]) -> Val {
    Val::List(ops.iter().map(op_record).collect())
}

/// The bytes of a paint list: what the canvas's `content` holds.
pub(crate) fn bytes(ops: Vec<PaintOp>) -> Vec<u8> {
    encode(&PaintList { ops })
}

/// A rectangle op from the numbers and colour words of a canvas's own fields (a box's background).
///
/// # Errors
///
/// A number is not finite or the fill is not a colour.
pub(crate) fn rect(x: f64, y: f64, width: f64, height: f64, radius: f64, fill: &str) -> Result<PaintOp, String> {
    Ok(PaintOp::Rect {
        x: finite(x)?,
        y: finite(y)?,
        width: finite(width)?,
        height: finite(height)?,
        radius: finite(radius)?,
        fill: colour(fill)?,
        stroke: Rgba8::TRANSPARENT,
        stroke_width: 0.0,
    })
}

/// A text op.
///
/// # Errors
///
/// A number is not finite or the colour is not one.
pub(crate) fn text(x: f64, y: f64, size: f64, color: &str, font: &str, text: &str) -> Result<PaintOp, String> {
    Ok(PaintOp::Text {
        x: finite(x)?,
        y: finite(y)?,
        size: finite(size)?,
        colour: colour(color)?,
        font: font.to_owned(),
        text: text.to_owned(),
    })
}

/// An icon op.
///
/// # Errors
///
/// A number is not finite or the colour is not one.
pub(crate) fn icon(size: f64, color: &str, name: &str) -> Result<PaintOp, String> {
    Ok(PaintOp::Icon {
        x: 0.0,
        y: 0.0,
        size: finite(size)?,
        colour: colour(color)?,
        name: name.to_owned(),
    })
}

/// A line op of width `width`.
///
/// # Errors
///
/// A number is not finite or the colour is not one.
pub(crate) fn line(x1: f64, y1: f64, x2: f64, y2: f64, width: f64, color: &str) -> Result<PaintOp, String> {
    Ok(PaintOp::TextLine {
        x1: finite(x1)?,
        y1: finite(y1)?,
        x2: finite(x2)?,
        y2: finite(y2)?,
        width: finite(width)?,
        colour: colour(color)?,
    })
}

/// A rectangle op that is a ring: no fill, an outline of `stroke_width` in `stroke`.
///
/// # Errors
///
/// A number is not finite or the colour is not one.
pub(crate) fn outline(
    (x, y, width, height): (f64, f64, f64, f64),
    radius: f64,
    stroke: &str,
    stroke_width: f64,
) -> Result<PaintOp, String> {
    Ok(PaintOp::Rect {
        x: finite(x)?,
        y: finite(y)?,
        width: finite(width)?,
        height: finite(height)?,
        radius: finite(radius)?,
        fill: Rgba8::TRANSPARENT,
        stroke: colour(stroke)?,
        stroke_width: finite(stroke_width)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_read_in_its_four_forms() {
        assert_eq!(colour("#f00").unwrap(), Rgba8::new(255, 0, 0, 255));
        assert_eq!(colour("#f008").unwrap(), Rgba8::new(255, 0, 0, 136));
        assert_eq!(colour("#ff8800").unwrap(), Rgba8::new(255, 136, 0, 255));
        assert_eq!(colour("#ff880044").unwrap(), Rgba8::new(255, 136, 0, 68));
        assert_eq!(colour("#ABC").unwrap(), Rgba8::new(170, 187, 204, 255));
    }

    #[test]
    fn anything_else_is_refused_in_the_words_the_toolkit_always_used() {
        for bad in ["red", "#", "#12", "#12345", "#1234567", "ff0000", "#gg0000", ""] {
            assert_eq!(
                colour(bad).unwrap_err(),
                format!("a colour is #rgb, #rgba, #rrggbb or #rrggbbaa, not {bad}")
            );
        }
    }

    #[test]
    fn a_number_that_is_not_finite_is_refused() {
        assert_eq!(finite(f64::NAN).unwrap_err(), "expected a finite number, not NaN");
        assert_eq!(
            finite(f64::NEG_INFINITY).unwrap_err(),
            "expected a finite number, not -Infinity"
        );
        assert_eq!(finite(2.5).unwrap(), 2.5);
    }

    #[test]
    fn an_operation_that_leaves_fields_out_has_the_catalogs_defaults() {
        let list = Val::List(vec![
            record(kind::LINE, &[(at::X2, Val::Double(1.0)), (at::Y2, Val::Double(1.0))]),
            record(kind::TEXT, &[(at::TEXT, Val::Text("plain".into()))]),
            record(kind::ICON, &[(at::NAME, Val::Text("user-home".into()))]),
            record(kind::PUSH_OPACITY, &[]),
        ]);
        let ops = read(&list).unwrap();
        assert_eq!(
            ops[0],
            PaintOp::TextLine {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
                width: 1.0,
                colour: OPAQUE_WHITE
            }
        );
        assert_eq!(
            ops[1],
            PaintOp::Text {
                x: 0.0,
                y: 0.0,
                size: 14.0,
                colour: OPAQUE_WHITE,
                font: "sans".into(),
                text: "plain".into()
            }
        );
        assert_eq!(
            ops[2],
            PaintOp::Icon {
                x: 0.0,
                y: 0.0,
                size: 16.0,
                colour: OPAQUE_WHITE,
                name: "user-home".into()
            }
        );
        assert_eq!(ops[3], PaintOp::PushOpacity(1.0));
    }

    #[test]
    fn a_bad_operation_names_its_place() {
        let list = Val::List(vec![
            record(kind::POP_CLIP, &[]),
            record(kind::RECT, &[(at::FILL, Val::Text("red".into()))]),
        ]);
        assert_eq!(
            read(&list).unwrap_err(),
            "draw op 2: a colour is #rgb, #rgba, #rrggbb or #rrggbbaa, not red"
        );
    }

    #[test]
    fn what_the_library_draws_reads_back_as_the_records_it_would_be_given() {
        let ops = vec![
            rect(1.0, 2.0, 30.0, 20.0, 4.0, "#ff000080").unwrap(),
            text(3.0, 4.0, 12.0, "#abc", "mono", "hi").unwrap(),
            PaintOp::PopClip,
        ];
        assert_eq!(read(&to_val(&ops)).unwrap(), ops);
    }
}
