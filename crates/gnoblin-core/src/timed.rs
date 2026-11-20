//! Timed writes and effects: the two things a node write carries that a plain [`Value`] cannot.
//!
//! [`Node::set_animated`] plays a change over some seconds along a straight line. A [`Timing`] adds what a desktop library needs:
//! the curve (a list of progress samples), a delay, and the `tween` node whose `progress`, `paused` and `reverse` drive the track
//! ([`Node::set_timed`]). [`Effect`] is one entry of a node's `effects` list (a backdrop blur, a shadow, a border), written whole with
//! [`Node::set_effects`]; the list replaces the node's.
#![allow(unsafe_code)]

use core::ffi::c_void;

use gnoblin_core_abi::{self as abi, GnoblinEffect, GnoblinStr, GnoblinTiming, GnoblinValue};

use crate::error::{Result, check};
use crate::owned::Value;
use crate::tree::Node;
use crate::value::{MemberKind, NodeId};

/// How a timed write plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// The duration in seconds, above zero.
    pub seconds: f64,
    /// Seconds the start value is held before the animation begins, from 0 to 3600.
    pub delay: f64,
    /// The `tween` node that drives the track, if any.
    pub tween: Option<NodeId>,
    /// The curve: at least two progress values from 0 to 1, spread evenly over the duration.
    pub samples: Vec<f64>,
}

impl Timing {
    /// A straight line over `seconds`, with no delay and no tween.
    pub fn linear(seconds: f64) -> Timing {
        Timing {
            seconds,
            delay: 0.0,
            tween: None,
            samples: vec![0.0, 1.0],
        }
    }
}

/// The kind of an [`Effect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    /// A blur of what is behind the node.
    Blur,
    /// A shadow behind the node.
    Shadow,
    /// A stroke around the node.
    Border,
    /// A shader the shell wrote, in WGSL.
    Shader,
}

/// One entry of a node's `effects` list, with the schema's field names. A field the kind does not use is ignored by the
/// compositor; [`Effect::new`] gives every field the schema's documented default.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    /// What the effect is.
    pub kind: EffectKind,
    /// Blur: the backdrop is sampled at `1 / downscale` of full resolution.
    pub downscale: u32,
    /// Blur: the number of passes.
    pub passes: u32,
    /// Blur: the kernel name.
    pub algorithm: String,
    /// Blur: grain from 0 to 1.
    pub noise: f64,
    /// Blur: brightness multiplier.
    pub brightness: f64,
    /// Blur: contrast multiplier.
    pub contrast: f64,
    /// Blur: saturation multiplier.
    pub saturation: f64,
    /// Blur: the tint, or empty for none.
    pub tint: String,
    /// Shadow: horizontal offset in logical pixels.
    pub offset_x: f64,
    /// Shadow: vertical offset in logical pixels.
    pub offset_y: f64,
    /// Shadow: blur radius in logical pixels.
    pub blur_radius: f64,
    /// Shadow: how far the shadow grows before blurring.
    pub spread: f64,
    /// Shadow and border: the colour as `#rrggbb` or `#rrggbbaa`.
    pub colour: String,
    /// Border: stroke width in logical pixels.
    pub width: f64,
    /// Border: distance from the silhouette's edge.
    pub offset: f64,
    /// Border: the stroke lies inside the silhouette.
    pub inset: bool,
    /// Shadow: the shadow follows the whole frame.
    pub frame: bool,
    /// Shader: the WGSL source, at most 16 KiB.
    pub source: String,
    /// Shader: the numbers the source reads as `p[0]` to `p[7]`, at most 8.
    pub params: Vec<f64>,
}

impl Effect {
    /// An effect of `kind` with the schema's documented defaults for every parameter.
    pub fn new(kind: EffectKind) -> Effect {
        Effect {
            kind,
            downscale: 2,
            passes: 3,
            algorithm: "kawase".to_owned(),
            noise: 0.0,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            tint: String::new(),
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 12.0,
            spread: 0.0,
            colour: "#00000080".to_owned(),
            width: 0.0,
            offset: 0.0,
            inset: false,
            frame: false,
            source: String::new(),
            params: Vec::new(),
        }
    }

    /// A shader effect: `source` is WGSL that defines `fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) ->
    /// vec4<f32>`, and `params` (at most 8) is what it reads as `p`. The compositor refuses it, and the whole list with it,
    /// when `compositor.user-shaders` is switched off (the seat's `user_shaders` says) or the source does not validate.
    pub fn shader(source: &str, params: &[f64]) -> Effect {
        Effect {
            source: source.to_owned(),
            params: params.to_vec(),
            ..Effect::new(EffectKind::Shader)
        }
    }

    /// The ABI record, borrowing this effect's text for as long as the record is read.
    fn raw(&self) -> GnoblinEffect {
        let text = |text: &str| GnoblinStr {
            ptr: if text.is_empty() {
                core::ptr::null()
            } else {
                text.as_ptr()
            },
            len: text.len(),
        };
        GnoblinEffect {
            size: size_of::<GnoblinEffect>() as u32,
            kind: match self.kind {
                EffectKind::Blur => abi::EFFECT_BLUR,
                EffectKind::Shadow => abi::EFFECT_SHADOW,
                EffectKind::Border => abi::EFFECT_BORDER,
                EffectKind::Shader => abi::EFFECT_SHADER,
            },
            downscale: self.downscale,
            passes: self.passes,
            inset: u32::from(self.inset),
            frame: u32::from(self.frame),
            noise: self.noise,
            brightness: self.brightness,
            contrast: self.contrast,
            saturation: self.saturation,
            offset_x: self.offset_x,
            offset_y: self.offset_y,
            blur_radius: self.blur_radius,
            spread: self.spread,
            width: self.width,
            offset: self.offset,
            algorithm: text(&self.algorithm),
            tint: text(&self.tint),
            colour: text(&self.colour),
            source: text(&self.source),
            params: if self.params.is_empty() {
                core::ptr::null()
            } else {
                self.params.as_ptr()
            },
            n_params: self.params.len(),
        }
    }
}

impl Node {
    /// Writes a property as a timed write: the compositor plays the change over `timing.seconds` along `timing.samples`, after
    /// `timing.delay`, as part of `timing.tween` when one is given. Only a property the schema marks animated takes one.
    pub fn set_timed(&self, property: &str, value: impl Into<Value>, timing: &Timing) -> Result<()> {
        let class = self.class()?;
        let member = self.gnoblin().member(class, MemberKind::Property, property)?;
        let value = value.into();
        let raw_timing = GnoblinTiming {
            size: size_of::<GnoblinTiming>() as u32,
            reserved: 0,
            seconds: timing.seconds,
            delay: timing.delay,
            tween: timing.tween.map_or(0, |tween| tween.0),
            samples: timing.samples.as_ptr(),
            n_samples: timing.samples.len(),
        };
        let g = self.gnoblin();
        // SAFETY: `write_set_timed` reads the value and the timing for the call; both outlive it.
        value.with_raw(|raw| {
            check(unsafe { (g.api().write_set_timed)(g.ctx(), self.id().0, member.id, raw, &raw_timing) })
        })
    }

    /// Replaces the node's `effects` list with `effects`, in paint order.
    pub fn set_effects(&self, effects: &[Effect]) -> Result<()> {
        let class = self.class()?;
        let member = self.gnoblin().member(class, MemberKind::Property, "effects")?;
        let records: Vec<GnoblinEffect> = effects.iter().map(Effect::raw).collect();
        let raw = GnoblinValue {
            size: size_of::<GnoblinValue>() as u32,
            kind: abi::VALUE_EFFECTS,
            flags: 0,
            reserved: 0,
            bits: 0,
            ptr: if records.is_empty() {
                core::ptr::null::<c_void>()
            } else {
                records.as_ptr().cast()
            },
            len: records.len(),
        };
        let g = self.gnoblin();
        // SAFETY: `write_set` reads the effects and the text they point at for the call, and `effects` outlives it.
        check(unsafe { (g.api().write_set)(g.ctx(), self.id().0, member.id, &raw, 0.0) })
    }
}
