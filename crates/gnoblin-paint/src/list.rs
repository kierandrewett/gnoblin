//! The paint list: what a `Canvas` node's `content` holds, its binary encoding and its decoder.
//!
//! A shell builds a paint list as a Lua array of drawing operations (`data/lua/gnoblin/shell/ui.lua`) and
//! `data/lua/gnoblin/node.lua` encodes it into the `bytes` value the tree carries. The compositor decodes the
//! same bytes here, both to refuse a malformed value at write time (`bad-paint`, design/node-core.md F42) and to
//! draw it. One decoder serves both, so a value the tree accepts is a value the painter can draw.
//!
//! # Encoding
//!
//! Little-endian throughout. Empty `content` (the property's default) is the empty list. Otherwise:
//!
//! ```text
//! "GNPL"  u8 version (1)  u32 op count  op...
//! ```
//!
//! Each op is a `u8` tag followed by its fields. `f32` is IEEE 754 binary32; a colour is a `u32` holding
//! `0xRRGGBBAA` (straight, not premultiplied); `str2` is a `u16` byte length then UTF-8, `str4` a `u32` length.
//!
//! | Tag | Op | Fields |
//! |---|---|---|
//! | 1 | rect | `f32` x, y, width, height, radius; `u32` fill; `u32` stroke; `f32` stroke width |
//! | 2 | line | `f32` x1, y1, x2, y2, width; `u32` colour |
//! | 3 | text | `f32` x, y, size; `u32` colour; `str2` font; `str4` text |
//! | 4 | icon | `f32` x, y, size; `u32` colour; `str2` name |
//! | 5 | push clip | `f32` x, y, width, height, radius |
//! | 6 | pop clip | none |
//! | 7 | push opacity | `f32` alpha, 0 to 1 |
//! | 8 | pop opacity | none |
//!
//! Ops draw in order, later over earlier. A clip or opacity group applies to every op between its push and its
//! pop; groups nest. The pushes and pops must balance.
//!
//! # Refusals
//!
//! A value that does not decode is [`DecodeError`]: a wrong magic or version, a truncated or over-long buffer, an
//! unknown tag, a float that is not finite or is out of range, a string that is not UTF-8 or is over its cap, a
//! pop with no push, a push never popped, or more than [`MAX_OPS`] ops or [`MAX_DEPTH`] nested groups.

use std::fmt;

/// The magic every encoded list starts with.
pub const MAGIC: [u8; 4] = *b"GNPL";

/// The encoding's version byte.
pub const VERSION: u8 = 1;

/// The most ops one list may carry.
pub const MAX_OPS: usize = 4096;

/// The deepest nesting of clip and opacity groups.
pub const MAX_DEPTH: usize = 32;

/// The longest text of one text op, in bytes.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

/// The longest font or icon name, in bytes.
pub const MAX_NAME_BYTES: usize = 128;

/// The largest coordinate or extent magnitude an op may carry, in pixels.
pub const MAX_COORD: f32 = 65_536.0;

/// The largest text or icon size, in pixels.
pub const MAX_SIZE: f32 = 1024.0;

const TAG_RECT: u8 = 1;
const TAG_LINE: u8 = 2;
const TAG_TEXT: u8 = 3;
const TAG_ICON: u8 = 4;
const TAG_PUSH_CLIP: u8 = 5;
const TAG_POP_CLIP: u8 = 6;
const TAG_PUSH_OPACITY: u8 = 7;
const TAG_POP_OPACITY: u8 = 8;

/// A straight (not premultiplied) colour with eight bits per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba8 {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha: 0 is transparent.
    pub a: u8,
}

impl Rgba8 {
    /// The fully transparent colour.
    pub const TRANSPARENT: Self = Self::new(0, 0, 0, 0);

    /// A colour from its channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// The colour a `0xRRGGBBAA` word names.
    #[must_use]
    pub const fn from_word(word: u32) -> Self {
        Self::new((word >> 24) as u8, (word >> 16) as u8, (word >> 8) as u8, word as u8)
    }

    /// The `0xRRGGBBAA` word of this colour.
    #[must_use]
    pub const fn to_word(self) -> u32 {
        ((self.r as u32) << 24) | ((self.g as u32) << 16) | ((self.b as u32) << 8) | self.a as u32
    }

    /// Whether drawing this colour changes nothing.
    #[must_use]
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }
}

/// One drawing operation of a paint list.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintOp {
    /// A rectangle with optionally rounded corners, filled and outlined.
    Rect {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Width.
        width: f32,
        /// Height.
        height: f32,
        /// Corner radius; clamped to half the shorter side when drawn.
        radius: f32,
        /// The fill; transparent for none.
        fill: Rgba8,
        /// The outline, drawn inside the rectangle's edge; transparent for none.
        stroke: Rgba8,
        /// The outline's width.
        stroke_width: f32,
    },
    /// A straight line with butt caps.
    TextLine {
        /// Start x.
        x1: f32,
        /// Start y.
        y1: f32,
        /// End x.
        x2: f32,
        /// End y.
        y2: f32,
        /// Stroke width.
        width: f32,
        /// Colour.
        colour: Rgba8,
    },
    /// A single line of text (a `\n` starts another). `(x, y)` is the top left of the first line's box, which is
    /// [`crate::line_height`] tall.
    Text {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Font size in pixels (the em).
        size: f32,
        /// Colour.
        colour: Rgba8,
        /// The font family or generic name (`sans`, `serif`, `monospace`).
        font: String,
        /// The text.
        text: String,
    },
    /// A named symbolic icon in a square of `size` at `(x, y)`.
    Icon {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Side of the icon's square.
        size: f32,
        /// Colour.
        colour: Rgba8,
        /// The icon's name, such as `window-close-symbolic`.
        name: String,
    },
    /// Restrict the ops until the matching [`PaintOp::PopClip`] to a rectangle, with optionally rounded corners.
    PushClip {
        /// Left edge.
        x: f32,
        /// Top edge.
        y: f32,
        /// Width.
        width: f32,
        /// Height.
        height: f32,
        /// Corner radius.
        radius: f32,
    },
    /// End the innermost clip.
    PopClip,
    /// Draw the ops until the matching [`PaintOp::PopOpacity`] as a group at this alpha.
    PushOpacity(f32),
    /// End the innermost opacity group.
    PopOpacity,
}

/// A decoded paint list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaintList {
    /// The ops, in drawing order.
    pub ops: Vec<PaintOp>,
}

/// Why a paint list did not decode. The tree refuses such a value `bad-paint`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer does not start with the magic or carries another version.
    Header,
    /// The buffer ends inside an op, or has bytes after the last one.
    Length,
    /// An op tag this version does not define.
    Tag(u8),
    /// A number that is not finite, or is outside the range the op allows.
    Number(&'static str),
    /// A string that is not UTF-8 or is over its cap.
    Text(&'static str),
    /// A pop with no push, a push never popped, or a pop of the other kind.
    Unbalanced,
    /// More than [`MAX_OPS`] ops, or groups nested deeper than [`MAX_DEPTH`].
    TooLarge,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header => f.write_str("not a paint list: wrong magic or version"),
            Self::Length => f.write_str("the paint list is truncated or has trailing bytes"),
            Self::Tag(tag) => write!(f, "unknown paint op {tag}"),
            Self::Number(what) => write!(f, "{what} is not a finite number in range"),
            Self::Text(what) => write!(f, "{what} is not valid UTF-8 or is too long"),
            Self::Unbalanced => f.write_str("clip or opacity pushes and pops do not balance"),
            Self::TooLarge => f.write_str("the paint list has too many ops or groups nested too deep"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Which kind of group a pushed op opened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    Clip,
    Opacity,
}

/// A cursor over the bytes being decoded.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
        if self.bytes.len() < count {
            return Err(DecodeError::Length);
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A finite `f32` whose magnitude is at most `limit`.
    fn float(&mut self, what: &'static str, limit: f32) -> Result<f32, DecodeError> {
        let bytes = self.take(4)?;
        let value = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if value.is_finite() && value.abs() <= limit {
            Ok(value)
        } else {
            Err(DecodeError::Number(what))
        }
    }

    /// A finite, non-negative `f32` of at most `limit`.
    fn extent(&mut self, what: &'static str, limit: f32) -> Result<f32, DecodeError> {
        let value = self.float(what, limit)?;
        if value < 0.0 {
            Err(DecodeError::Number(what))
        } else {
            Ok(value)
        }
    }

    fn colour(&mut self) -> Result<Rgba8, DecodeError> {
        Ok(Rgba8::from_word(self.u32()?))
    }

    fn string(&mut self, what: &'static str, length: usize, cap: usize) -> Result<String, DecodeError> {
        if length > cap {
            return Err(DecodeError::Text(what));
        }
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::Text(what))
    }
}

/// Decode a paint list. Empty bytes are the empty list.
///
/// # Errors
///
/// Returns why the bytes are not a paint list, which the tree reports as `bad-paint`.
pub fn decode(bytes: &[u8]) -> Result<PaintList, DecodeError> {
    if bytes.is_empty() {
        return Ok(PaintList::default());
    }
    let mut reader = Reader { bytes };
    if reader.take(4)? != MAGIC || reader.u8()? != VERSION {
        return Err(DecodeError::Header);
    }
    let count = reader.u32()? as usize;
    if count > MAX_OPS {
        return Err(DecodeError::TooLarge);
    }
    let mut ops = Vec::with_capacity(count);
    let mut groups: Vec<Group> = Vec::new();
    for _ in 0..count {
        let op = decode_op(&mut reader)?;
        match op {
            PaintOp::PushClip { .. } => push_group(&mut groups, Group::Clip)?,
            PaintOp::PushOpacity(_) => push_group(&mut groups, Group::Opacity)?,
            PaintOp::PopClip if groups.pop() != Some(Group::Clip) => return Err(DecodeError::Unbalanced),
            PaintOp::PopOpacity if groups.pop() != Some(Group::Opacity) => return Err(DecodeError::Unbalanced),
            _ => {}
        }
        ops.push(op);
    }
    if !reader.bytes.is_empty() {
        return Err(DecodeError::Length);
    }
    if !groups.is_empty() {
        return Err(DecodeError::Unbalanced);
    }
    Ok(PaintList { ops })
}

fn push_group(groups: &mut Vec<Group>, group: Group) -> Result<(), DecodeError> {
    if groups.len() >= MAX_DEPTH {
        return Err(DecodeError::TooLarge);
    }
    groups.push(group);
    Ok(())
}

fn decode_op(reader: &mut Reader<'_>) -> Result<PaintOp, DecodeError> {
    Ok(match reader.u8()? {
        TAG_RECT => PaintOp::Rect {
            x: reader.float("rect x", MAX_COORD)?,
            y: reader.float("rect y", MAX_COORD)?,
            width: reader.extent("rect width", MAX_COORD)?,
            height: reader.extent("rect height", MAX_COORD)?,
            radius: reader.extent("rect radius", MAX_COORD)?,
            fill: reader.colour()?,
            stroke: reader.colour()?,
            stroke_width: reader.extent("rect stroke width", MAX_COORD)?,
        },
        TAG_LINE => PaintOp::TextLine {
            x1: reader.float("line x1", MAX_COORD)?,
            y1: reader.float("line y1", MAX_COORD)?,
            x2: reader.float("line x2", MAX_COORD)?,
            y2: reader.float("line y2", MAX_COORD)?,
            width: reader.extent("line width", MAX_COORD)?,
            colour: reader.colour()?,
        },
        TAG_TEXT => {
            let x = reader.float("text x", MAX_COORD)?;
            let y = reader.float("text y", MAX_COORD)?;
            let size = reader.extent("text size", MAX_SIZE)?;
            let colour = reader.colour()?;
            let font_length = usize::from(reader.u16()?);
            let font = reader.string("text font", font_length, MAX_NAME_BYTES)?;
            let text_length = reader.u32()? as usize;
            let text = reader.string("text", text_length, MAX_TEXT_BYTES)?;
            PaintOp::Text {
                x,
                y,
                size,
                colour,
                font,
                text,
            }
        }
        TAG_ICON => {
            let x = reader.float("icon x", MAX_COORD)?;
            let y = reader.float("icon y", MAX_COORD)?;
            let size = reader.extent("icon size", MAX_SIZE)?;
            let colour = reader.colour()?;
            let name_length = usize::from(reader.u16()?);
            let name = reader.string("icon name", name_length, MAX_NAME_BYTES)?;
            PaintOp::Icon {
                x,
                y,
                size,
                colour,
                name,
            }
        }
        TAG_PUSH_CLIP => PaintOp::PushClip {
            x: reader.float("clip x", MAX_COORD)?,
            y: reader.float("clip y", MAX_COORD)?,
            width: reader.extent("clip width", MAX_COORD)?,
            height: reader.extent("clip height", MAX_COORD)?,
            radius: reader.extent("clip radius", MAX_COORD)?,
        },
        TAG_POP_CLIP => PaintOp::PopClip,
        TAG_PUSH_OPACITY => {
            let alpha = reader.float("opacity", 1.0)?;
            if alpha < 0.0 {
                return Err(DecodeError::Number("opacity"));
            }
            PaintOp::PushOpacity(alpha)
        }
        TAG_POP_OPACITY => PaintOp::PopOpacity,
        tag => return Err(DecodeError::Tag(tag)),
    })
}

/// Encode a paint list. [`decode`] of the result gives the list back, unless an op carries a value the decoder
/// refuses.
#[must_use]
pub fn encode(list: &PaintList) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&u32::try_from(list.ops.len()).unwrap_or(u32::MAX).to_le_bytes());
    for op in &list.ops {
        encode_op(&mut out, op);
    }
    out
}

fn put_f32(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn put_colour(out: &mut Vec<u8>, colour: Rgba8) {
    out.extend_from_slice(&colour.to_word().to_le_bytes());
}

fn put_str2(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&u16::try_from(text.len()).unwrap_or(u16::MAX).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn encode_op(out: &mut Vec<u8>, op: &PaintOp) {
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
        } => {
            out.push(TAG_RECT);
            put_f32(out, &[*x, *y, *width, *height, *radius]);
            put_colour(out, *fill);
            put_colour(out, *stroke);
            put_f32(out, &[*stroke_width]);
        }
        PaintOp::TextLine {
            x1,
            y1,
            x2,
            y2,
            width,
            colour,
        } => {
            out.push(TAG_LINE);
            put_f32(out, &[*x1, *y1, *x2, *y2, *width]);
            put_colour(out, *colour);
        }
        PaintOp::Text {
            x,
            y,
            size,
            colour,
            font,
            text,
        } => {
            out.push(TAG_TEXT);
            put_f32(out, &[*x, *y, *size]);
            put_colour(out, *colour);
            put_str2(out, font);
            out.extend_from_slice(&u32::try_from(text.len()).unwrap_or(u32::MAX).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
        }
        PaintOp::Icon {
            x,
            y,
            size,
            colour,
            name,
        } => {
            out.push(TAG_ICON);
            put_f32(out, &[*x, *y, *size]);
            put_colour(out, *colour);
            put_str2(out, name);
        }
        PaintOp::PushClip {
            x,
            y,
            width,
            height,
            radius,
        } => {
            out.push(TAG_PUSH_CLIP);
            put_f32(out, &[*x, *y, *width, *height, *radius]);
        }
        PaintOp::PopClip => out.push(TAG_POP_CLIP),
        PaintOp::PushOpacity(alpha) => {
            out.push(TAG_PUSH_OPACITY);
            put_f32(out, &[*alpha]);
        }
        PaintOp::PopOpacity => out.push(TAG_POP_OPACITY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PaintList {
        PaintList {
            ops: vec![
                PaintOp::Rect {
                    x: 0.0,
                    y: 1.5,
                    width: 20.0,
                    height: 10.0,
                    radius: 3.0,
                    fill: Rgba8::new(1, 2, 3, 4),
                    stroke: Rgba8::TRANSPARENT,
                    stroke_width: 0.0,
                },
                PaintOp::PushClip {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                    radius: 0.0,
                },
                PaintOp::PushOpacity(0.5),
                PaintOp::Text {
                    x: 2.0,
                    y: 3.0,
                    size: 13.0,
                    colour: Rgba8::new(255, 255, 255, 255),
                    font: "sans".to_owned(),
                    text: "héllo".to_owned(),
                },
                PaintOp::PopOpacity,
                PaintOp::Icon {
                    x: 0.0,
                    y: 0.0,
                    size: 16.0,
                    colour: Rgba8::new(9, 9, 9, 255),
                    name: "window-close-symbolic".to_owned(),
                },
                PaintOp::PopClip,
                PaintOp::TextLine {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 4.0,
                    y2: 4.0,
                    width: 1.0,
                    colour: Rgba8::new(0, 0, 0, 255),
                },
            ],
        }
    }

    #[test]
    fn a_list_round_trips_through_its_encoding() {
        let list = sample();
        assert_eq!(decode(&encode(&list)), Ok(list));
    }

    #[test]
    fn empty_content_is_the_empty_list() {
        assert_eq!(decode(&[]), Ok(PaintList::default()));
    }

    #[test]
    fn a_colour_word_is_red_green_blue_alpha_from_the_top_byte() {
        let colour = Rgba8::from_word(0x1122_3344);
        assert_eq!(colour, Rgba8::new(0x11, 0x22, 0x33, 0x44));
        assert_eq!(colour.to_word(), 0x1122_3344);
    }

    #[test]
    fn a_wrong_magic_or_version_is_refused() {
        let mut bytes = encode(&sample());
        bytes[0] = b'X';
        assert_eq!(decode(&bytes), Err(DecodeError::Header));
        let mut bytes = encode(&sample());
        bytes[4] = 2;
        assert_eq!(decode(&bytes), Err(DecodeError::Header));
    }

    #[test]
    fn every_truncation_is_refused_without_panicking() {
        let bytes = encode(&sample());
        for cut in 1..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "a list cut at {cut} decoded");
        }
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut bytes = encode(&sample());
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(DecodeError::Length));
    }

    #[test]
    fn an_unknown_tag_is_refused() {
        let mut bytes = Vec::from(MAGIC);
        bytes.push(VERSION);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.push(99);
        assert_eq!(decode(&bytes), Err(DecodeError::Tag(99)));
    }

    #[test]
    fn a_number_that_is_not_finite_or_is_negative_is_refused() {
        for (width, expected) in [
            (f32::NAN, "rect width"),
            (f32::INFINITY, "rect width"),
            (-1.0, "rect width"),
            (1.0e9, "rect width"),
        ] {
            let mut list = sample();
            list.ops[0] = PaintOp::Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: 1.0,
                radius: 0.0,
                fill: Rgba8::TRANSPARENT,
                stroke: Rgba8::TRANSPARENT,
                stroke_width: 0.0,
            };
            assert_eq!(decode(&encode(&list)), Err(DecodeError::Number(expected)), "{width}");
        }
    }

    #[test]
    fn an_opacity_outside_zero_to_one_is_refused() {
        for alpha in [-0.1, 1.5, f32::NAN] {
            let list = PaintList {
                ops: vec![PaintOp::PushOpacity(alpha), PaintOp::PopOpacity],
            };
            assert_eq!(decode(&encode(&list)), Err(DecodeError::Number("opacity")));
        }
    }

    #[test]
    fn text_that_is_not_utf8_or_is_too_long_is_refused() {
        let mut bytes = encode(&PaintList {
            ops: vec![PaintOp::Text {
                x: 0.0,
                y: 0.0,
                size: 10.0,
                colour: Rgba8::TRANSPARENT,
                font: "sans".to_owned(),
                text: "ab".to_owned(),
            }],
        });
        let last = bytes.len() - 1;
        bytes[last] = 0xff;
        assert_eq!(decode(&bytes), Err(DecodeError::Text("text")));

        let long = PaintList {
            ops: vec![PaintOp::Text {
                x: 0.0,
                y: 0.0,
                size: 10.0,
                colour: Rgba8::TRANSPARENT,
                font: "sans".to_owned(),
                text: "a".repeat(MAX_TEXT_BYTES + 1),
            }],
        };
        assert_eq!(decode(&encode(&long)), Err(DecodeError::Text("text")));
    }

    #[test]
    fn pushes_and_pops_must_balance_and_match_in_kind() {
        let clip = PaintOp::PushClip {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            radius: 0.0,
        };
        for ops in [
            vec![PaintOp::PopClip],
            vec![clip.clone()],
            vec![clip.clone(), PaintOp::PopOpacity],
            vec![PaintOp::PushOpacity(1.0), PaintOp::PopClip],
        ] {
            assert_eq!(decode(&encode(&PaintList { ops })), Err(DecodeError::Unbalanced));
        }
    }

    #[test]
    fn too_many_ops_or_too_deep_a_nest_is_refused() {
        let many = PaintList {
            ops: vec![PaintOp::PopClip; MAX_OPS + 1],
        };
        assert_eq!(decode(&encode(&many)), Err(DecodeError::TooLarge));
        let mut ops = vec![PaintOp::PushOpacity(1.0); MAX_DEPTH + 1];
        ops.extend(vec![PaintOp::PopOpacity; MAX_DEPTH + 1]);
        assert_eq!(decode(&encode(&PaintList { ops })), Err(DecodeError::TooLarge));
    }
}
