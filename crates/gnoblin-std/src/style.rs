//! Styles: borders, shadows, backdrop filters, opacity and corner radius.
//!
//! A [`Style`] holds only what was set on its scope, and what was never set is inherited. The scopes of a client, lowest first, are the
//! global one (`std.clients().style()`), the one of its workspace (`workspace.clients().style()`), the `where` selections that hold it
//! (`clients.where_(&Query::new().app_id("foo")).style()`, in the order they first got a style) and its own (`client.style()`). Per
//! property the highest scope that sets it wins, and a list a scope sets replaces the inherited list whole.
//!
//! The library applies the resolved style to each window's node in the turn after the layout pass: shadows, borders and a backdrop blur
//! become the node's `effects` (in paint order: the backdrop blur, the shadows from the last listed to the first, then the borders), the
//! opacity and the corner radius are written to the node, and [`Client::style_warnings`] names what the tree cannot express (an inset
//! shadow, a window background, a content filter, per-corner radii) instead of faking it.
//!
//! This is `data/js/gnoblin-std/style/`. What differs: a list is a `Vec` read, changed and set whole (there is no array to edit in
//! place), so the copy-on-write of an inherited list has nothing to do.
#![allow(unsafe_code)]

use gnoblin_core_abi as core_abi;
use gnoblin_std_abi as abi;
use gnoblin_std_abi::{GnoblinStdProp, GnoblinStdStyle, GnoblinStdStyleEntries, GnoblinStdStyleEntry};

use crate::animation::{PropValue, lent_text};
use crate::api::Std;
use crate::client::Client;
use crate::collection::Collection;
use crate::error::Result;

/// How many entries a style list holds.
pub const MAX_STYLE_LIST: usize = 8;

/// One stroke around a window. Several make concentric rings, `offset` apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    /// Stroke width in pixels. Default 1.
    pub width: f64,
    /// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. Default white.
    pub color: String,
    /// Distance from the frame in pixels; negative moves the stroke inwards. Default 0.
    pub offset: f64,
    /// Draws the stroke inside the frame. Default false.
    pub inset: bool,
}

impl Default for Border {
    fn default() -> Border {
        Border {
            width: 1.0,
            color: "#ffffff".to_owned(),
            offset: 0.0,
            inset: false,
        }
    }
}

/// A shadow behind a window (or inside it with `inset`, which the tree cannot draw).
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    /// Horizontal offset in pixels. Default 0.
    pub x: f64,
    /// Vertical offset in pixels. Default 0.
    pub y: f64,
    /// Blur radius in pixels, 0 to 100. Default 12.
    pub blur: f64,
    /// How far it grows before blurring, -100 to 100. Default 0.
    pub spread: f64,
    /// Default translucent black.
    pub color: String,
    /// Draws the shadow inside the window. Default false.
    pub inset: bool,
}

impl Default for Shadow {
    fn default() -> Shadow {
        Shadow {
            x: 0.0,
            y: 0.0,
            blur: 12.0,
            spread: 0.0,
            color: "#00000080".to_owned(),
            inset: false,
        }
    }
}

/// One filter of a `filter` or `backdrop_filter` chain.
#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    /// A blur of `radius` pixels (0 to 100). Of a backdrop chain it blurs what is behind the window.
    Blur(f64),
    /// The colour saturation: 1 unchanged, 0 grey, up to 2.
    Saturate(f64),
    /// The brightness gain: 1 unchanged, up to 2.
    Brightness(f64),
    /// The contrast gain about mid grey: 1 unchanged, up to 2.
    Contrast(f64),
    /// A shader of the shell's own: one `shader` effect on the window's node, drawn when the compositor's `compositor.user-shaders`
    /// setting is on (the seat's `user_shaders`) and otherwise reported by [`Client::style_warnings`]. The same in either filter list.
    Shader {
        /// The WGSL source, at most 16 KiB. It defines `fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) ->
        /// vec4<f32>`: the window's premultiplied colour at `uv` (`(0, 0)` is the top left), the numbers, and the time in seconds.
        /// It may call `backdrop(uv)`, the colour of what is behind the window, with secure windows black. The compositor checks
        /// the source before it accepts it, and refuses the window's whole effect list when it does not validate.
        source: String,
        /// The numbers the source reads as `p[0]` to `p[7]`: at most 8, each finite.
        params: Vec<f64>,
    },
}

impl Filter {
    /// A shader filter with `source` and `params`.
    pub fn shader(source: &str, params: &[f64]) -> Filter {
        Filter::Shader {
            source: source.to_owned(),
            params: params.to_vec(),
        }
    }
}

/// The radius of a window's corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CornerRadius {
    /// One radius, in logical pixels, for every corner.
    All(f64),
    /// A radius per corner. The tree has one radius for every corner, so the largest of the four is drawn.
    Corners {
        /// Top left.
        top_left: f64,
        /// Top right.
        top_right: f64,
        /// Bottom right.
        bottom_right: f64,
        /// Bottom left.
        bottom_left: f64,
    },
}

/// A property of a style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StyleProperty {
    /// `opacity`.
    Opacity,
    /// `cornerRadius`.
    CornerRadius,
    /// `background`.
    Background,
    /// `borders`.
    Borders,
    /// `shadows`.
    Shadows,
    /// `filter`.
    Filter,
    /// `backdropFilter`.
    BackdropFilter,
}

impl StyleProperty {
    fn raw(self) -> u32 {
        match self {
            StyleProperty::Opacity => abi::STD_STYLE_OPACITY,
            StyleProperty::CornerRadius => abi::STD_STYLE_CORNER_RADIUS,
            StyleProperty::Background => abi::STD_STYLE_BACKGROUND,
            StyleProperty::Borders => abi::STD_STYLE_BORDERS,
            StyleProperty::Shadows => abi::STD_STYLE_SHADOWS,
            StyleProperty::Filter => abi::STD_STYLE_FILTER,
            StyleProperty::BackdropFilter => abi::STD_STYLE_BACKDROP_FILTER,
        }
    }
}

/// Where a property of a client's style came from: `default`, `global`, `workspace:<id>`, `where(<query>)` or `client`.
pub type Origin = String;

/// A client's resolved style: what its layers resolve to.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedStyle {
    /// From 0 (transparent) to 1.
    pub opacity: f64,
    /// The corner radius.
    pub corner_radius: CornerRadius,
    /// The background colour, or none.
    pub background: Option<String>,
    /// The borders.
    pub borders: Vec<Border>,
    /// The shadows.
    pub shadows: Vec<Shadow>,
    /// The filters of the window's own content.
    pub filter: Vec<Filter>,
    /// The filters of what is behind the window.
    pub backdrop_filter: Vec<Filter>,
    /// The layer each property came from, in the order of [`StyleProperty`].
    pub origin: [Origin; 7],
}

impl ComputedStyle {
    /// The layer `property` came from.
    pub fn origin_of(&self, property: StyleProperty) -> &str {
        &self.origin[property as usize]
    }
}

/// The style of one scope. Cheap to clone; it belongs to the shell thread.
#[derive(Clone)]
pub struct Style {
    std: Std,
    id: GnoblinStdStyle,
}

impl std::fmt::Debug for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Style").field("id", &self.id).finish()
    }
}

fn entry_of_border(border: &Border) -> GnoblinStdStyleEntry {
    GnoblinStdStyleEntry {
        size: size_of::<GnoblinStdStyleEntry>() as u32,
        kind: abi::STD_ENTRY_BORDER,
        inset: u32::from(border.inset),
        reserved: 0,
        a: border.width,
        b: border.offset,
        c: 0.0,
        d: 0.0,
        color: core_abi::GnoblinStr::borrowed(&border.color),
        source: core_abi::GnoblinStr::EMPTY,
        params: core::ptr::null(),
        n_params: 0,
    }
}

fn entry_of_shadow(shadow: &Shadow) -> GnoblinStdStyleEntry {
    GnoblinStdStyleEntry {
        size: size_of::<GnoblinStdStyleEntry>() as u32,
        kind: abi::STD_ENTRY_SHADOW,
        inset: u32::from(shadow.inset),
        reserved: 0,
        a: shadow.x,
        b: shadow.y,
        c: shadow.blur,
        d: shadow.spread,
        color: core_abi::GnoblinStr::borrowed(&shadow.color),
        source: core_abi::GnoblinStr::EMPTY,
        params: core::ptr::null(),
        n_params: 0,
    }
}

fn entry_of_filter(filter: &Filter) -> GnoblinStdStyleEntry {
    let (kind, amount) = match filter {
        Filter::Blur(radius) => (abi::STD_ENTRY_BLUR, *radius),
        Filter::Saturate(amount) => (abi::STD_ENTRY_SATURATE, *amount),
        Filter::Brightness(amount) => (abi::STD_ENTRY_BRIGHTNESS, *amount),
        Filter::Contrast(amount) => (abi::STD_ENTRY_CONTRAST, *amount),
        Filter::Shader { .. } => (abi::STD_ENTRY_SHADER, 0.0),
    };
    let (source, params): (&str, &[f64]) = match filter {
        Filter::Shader { source, params } => (source, params),
        _ => ("", &[]),
    };
    GnoblinStdStyleEntry {
        size: size_of::<GnoblinStdStyleEntry>() as u32,
        kind,
        inset: 0,
        reserved: 0,
        a: amount,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        color: core_abi::GnoblinStr::EMPTY,
        source: core_abi::GnoblinStr::borrowed(source),
        params: if params.is_empty() {
            core::ptr::null()
        } else {
            params.as_ptr()
        },
        n_params: params.len(),
    }
}

/// The entries the library lent, copied.
fn copied(entries: &GnoblinStdStyleEntries) -> Vec<GnoblinStdStyleEntry> {
    if entries.ptr.is_null() || entries.len == 0 {
        return Vec::new();
    }
    // SAFETY: the library lends `len` entries at `ptr` until the next call into it on this thread; they are copied now.
    unsafe { std::slice::from_raw_parts(entries.ptr, entries.len) }.to_vec()
}

fn border_of(raw: &GnoblinStdStyleEntry) -> Border {
    Border {
        width: raw.a,
        color: lent_text(&raw.color),
        offset: raw.b,
        inset: raw.inset != 0,
    }
}

fn shadow_of(raw: &GnoblinStdStyleEntry) -> Shadow {
    Shadow {
        x: raw.a,
        y: raw.b,
        blur: raw.c,
        spread: raw.d,
        color: lent_text(&raw.color),
        inset: raw.inset != 0,
    }
}

fn filter_of(raw: &GnoblinStdStyleEntry) -> Option<Filter> {
    Some(match raw.kind {
        abi::STD_ENTRY_BLUR => Filter::Blur(raw.a),
        abi::STD_ENTRY_SATURATE => Filter::Saturate(raw.a),
        abi::STD_ENTRY_BRIGHTNESS => Filter::Brightness(raw.a),
        abi::STD_ENTRY_CONTRAST => Filter::Contrast(raw.a),
        abi::STD_ENTRY_SHADER => Filter::Shader {
            source: lent_text(&raw.source),
            params: if raw.params.is_null() || raw.n_params == 0 {
                Vec::new()
            } else {
                // SAFETY: the library lends `n_params` numbers at `params` until the next call into it on this thread; they are
                // copied now, as the rest of the entry is.
                unsafe { std::slice::from_raw_parts(raw.params, raw.n_params) }.to_vec()
            },
        },
        _ => return None,
    })
}

fn radius_of(raw: &GnoblinStdProp) -> CornerRadius {
    if raw.kind == abi::STD_PROP_CORNERS {
        CornerRadius::Corners {
            top_left: raw.a,
            top_right: raw.b,
            bottom_right: raw.c,
            bottom_left: raw.d,
        }
    } else {
        CornerRadius::All(raw.a)
    }
}

fn prop_of_radius(radius: CornerRadius) -> GnoblinStdProp {
    match radius {
        CornerRadius::All(value) => PropValue::Number(value).raw(),
        CornerRadius::Corners {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } => {
            let mut raw = PropValue::Number(0.0).raw();
            raw.kind = abi::STD_PROP_CORNERS;
            (raw.a, raw.b, raw.c, raw.d) = (top_left, top_right, bottom_right, bottom_left);
            raw
        }
    }
}

impl Style {
    pub(crate) fn scope(std: &Std, scope: u32, target: u64) -> Result<Style> {
        let mut id = 0;
        // SAFETY: `style_scope` writes one handle through the pointer it is given, a live local.
        std.check(unsafe { (std.api().style_scope)(std.ctx(), scope, target, &mut id) })?;
        Ok(Style { std: std.clone(), id })
    }

    fn set_prop(&self, property: StyleProperty, raw: &GnoblinStdProp) -> Result<()> {
        // SAFETY: the value (and any text it borrows) is read for the call.
        self.std
            .check(unsafe { (self.std.api().style_set)(self.std.ctx(), self.id, property.raw(), raw) })
            .map(|_| ())
    }

    fn get_prop(&self, property: StyleProperty) -> Result<GnoblinStdProp> {
        let mut raw = PropValue::Number(0.0).raw();
        // SAFETY: `style_get` fills the struct up to the `size` the caller set; `raw` is a live local. Text it lends is copied by the
        // caller before the next call.
        self.std
            .check(unsafe { (self.std.api().style_get)(self.std.ctx(), self.id, property.raw(), &mut raw) })?;
        Ok(raw)
    }

    fn set_list(&self, property: StyleProperty, entries: &[GnoblinStdStyleEntry]) -> Result<()> {
        // SAFETY: the entries (and the colours they borrow from the caller's lists) are read for the call.
        self.std
            .check(unsafe {
                (self.std.api().style_set_list)(
                    self.std.ctx(),
                    self.id,
                    property.raw(),
                    entries.as_ptr(),
                    entries.len(),
                )
            })
            .map(|_| ())
    }

    fn list(&self, property: StyleProperty) -> Result<Vec<GnoblinStdStyleEntry>> {
        let mut out = GnoblinStdStyleEntries {
            ptr: core::ptr::null(),
            len: 0,
        };
        // SAFETY: `style_list` writes the array it lends through the pointer it is given, a live local; it is copied before the next
        // call into the library.
        self.std
            .check(unsafe { (self.std.api().style_list)(self.std.ctx(), self.id, property.raw(), &mut out) })?;
        Ok(copied(&out))
    }

    /// The opacity this scope sets, from 0 (invisible) to 1; `None` when it inherits.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn opacity(&self) -> Result<Option<f64>> {
        let raw = self.get_prop(StyleProperty::Opacity)?;
        Ok((raw.kind == abi::STD_PROP_NUMBER).then_some(raw.a))
    }

    /// Sets the opacity, written to the window node's `opacity`.
    ///
    /// # Errors
    ///
    /// `opacity` is not a number from 0 to 1 (the text starts with the property's path), or the client is gone.
    pub fn set_opacity(&self, opacity: f64) -> Result<()> {
        self.set_prop(StyleProperty::Opacity, &PropValue::Number(opacity).raw())
    }

    /// The corner radius this scope sets; `None` when it inherits.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn corner_radius(&self) -> Result<Option<CornerRadius>> {
        let raw = self.get_prop(StyleProperty::CornerRadius)?;
        Ok((raw.kind != abi::STD_PROP_UNSET).then(|| radius_of(&raw)))
    }

    /// Sets the corner radius.
    ///
    /// # Errors
    ///
    /// A radius is not a finite number of at least 0, or the client is gone.
    pub fn set_corner_radius(&self, radius: CornerRadius) -> Result<()> {
        self.set_prop(StyleProperty::CornerRadius, &prop_of_radius(radius))
    }

    /// The background this scope sets: `None` when it inherits, `Some(None)` for no background.
    ///
    /// # Errors
    ///
    /// The generation retired.
    #[allow(clippy::option_option)]
    pub fn background(&self) -> Result<Option<Option<String>>> {
        let raw = self.get_prop(StyleProperty::Background)?;
        Ok(match raw.kind {
            abi::STD_PROP_TEXT => Some(Some(lent_text(&raw.text))),
            abi::STD_PROP_NULL => Some(None),
            _ => None,
        })
    }

    /// Sets the background colour, or clears it with `None`. The tree has no background for a window: it is kept in the computed
    /// style and reported by [`Client::style_warnings`].
    ///
    /// # Errors
    ///
    /// The colour is not `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`, or the client is gone.
    pub fn set_background(&self, colour: Option<&str>) -> Result<()> {
        match colour {
            Some(colour) => self.set_prop(StyleProperty::Background, &PropValue::from(colour).raw()),
            None => {
                let mut raw = PropValue::Number(0.0).raw();
                raw.kind = abi::STD_PROP_NULL;
                self.set_prop(StyleProperty::Background, &raw)
            }
        }
    }

    /// The borders this scope has: its own, else a copy of what it inherits.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn borders(&self) -> Result<Vec<Border>> {
        Ok(self.list(StyleProperty::Borders)?.iter().map(border_of).collect())
    }

    /// Replaces the border list: at most [`MAX_STYLE_LIST`] entries, all checked first.
    ///
    /// # Errors
    ///
    /// An entry is bad (the text names its path, `…style.borders[1].width`), the list is too long, or the client is gone. Nothing
    /// changes.
    pub fn set_borders(&self, borders: &[Border]) -> Result<()> {
        let entries: Vec<_> = borders.iter().map(entry_of_border).collect();
        self.set_list(StyleProperty::Borders, &entries)
    }

    /// The shadows this scope has: its own, else a copy of what it inherits. The first is on top.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn shadows(&self) -> Result<Vec<Shadow>> {
        Ok(self.list(StyleProperty::Shadows)?.iter().map(shadow_of).collect())
    }

    /// Replaces the shadow list.
    ///
    /// # Errors
    ///
    /// As for [`Style::set_borders`].
    pub fn set_shadows(&self, shadows: &[Shadow]) -> Result<()> {
        let entries: Vec<_> = shadows.iter().map(entry_of_shadow).collect();
        self.set_list(StyleProperty::Shadows, &entries)
    }

    /// The filters of the window's own content. No renderer draws them.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn filter(&self) -> Result<Vec<Filter>> {
        Ok(self.list(StyleProperty::Filter)?.iter().filter_map(filter_of).collect())
    }

    /// Replaces the content filter chain.
    ///
    /// # Errors
    ///
    /// As for [`Style::set_borders`].
    pub fn set_filter(&self, filters: &[Filter]) -> Result<()> {
        let entries: Vec<_> = filters.iter().map(entry_of_filter).collect();
        self.set_list(StyleProperty::Filter, &entries)
    }

    /// The filters of what is behind the window. A blur in the chain is the one the compositor draws.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn backdrop_filter(&self) -> Result<Vec<Filter>> {
        Ok(self
            .list(StyleProperty::BackdropFilter)?
            .iter()
            .filter_map(filter_of)
            .collect())
    }

    /// Replaces the backdrop filter chain.
    ///
    /// # Errors
    ///
    /// As for [`Style::set_borders`].
    pub fn set_backdrop_filter(&self, filters: &[Filter]) -> Result<()> {
        let entries: Vec<_> = filters.iter().map(entry_of_filter).collect();
        self.set_list(StyleProperty::BackdropFilter, &entries)
    }

    /// Takes a property back to inheriting.
    ///
    /// # Errors
    ///
    /// The generation retired, or the client is gone.
    pub fn unset(&self, property: StyleProperty) -> Result<()> {
        // SAFETY: `style_unset` takes the handle and a property.
        self.std
            .check(unsafe { (self.std.api().style_unset)(self.std.ctx(), self.id, property.raw()) })
            .map(|_| ())
    }

    /// Takes every property back to inheriting.
    ///
    /// # Errors
    ///
    /// As for [`Style::unset`].
    pub fn reset(&self) -> Result<()> {
        // SAFETY: `style_unset` takes the handle and a property; zero is every property.
        self.std
            .check(unsafe { (self.std.api().style_unset)(self.std.ctx(), self.id, 0) })
            .map(|_| ())
    }
}

impl Client {
    /// The client's own style: the top layer of its cascade.
    ///
    /// # Errors
    ///
    /// The client is gone.
    pub fn style(&self) -> Result<Style> {
        Style::scope(self.std(), abi::STD_STYLE_SCOPE_CLIENT, self.window().id().0)
    }

    /// The style the client's layers resolve to now, with the layer each property came from.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn computed_style(&self) -> Result<ComputedStyle> {
        let std = self.std();
        let node = self.window().id().0;
        let scalar = |property: StyleProperty| -> Result<GnoblinStdProp> {
            let mut raw = PropValue::Number(0.0).raw();
            // SAFETY: `style_computed` fills the struct up to the `size` the caller set; `raw` is a live local.
            std.check(unsafe { (std.api().style_computed)(std.ctx(), node, property.raw(), &mut raw) })?;
            Ok(raw)
        };
        let list = |property: StyleProperty| -> Result<Vec<GnoblinStdStyleEntry>> {
            let mut out = GnoblinStdStyleEntries {
                ptr: core::ptr::null(),
                len: 0,
            };
            // SAFETY: `style_computed_list` writes the array it lends through the pointer it is given; it is copied before the next
            // call into the library.
            std.check(unsafe { (std.api().style_computed_list)(std.ctx(), node, property.raw(), &mut out) })?;
            Ok(copied(&out))
        };
        let origin = |property: StyleProperty| -> Result<Origin> {
            let mut out = core_abi::GnoblinStr::EMPTY;
            // SAFETY: `style_origin` writes a string it lends through the pointer it is given; it is copied now.
            std.check(unsafe { (std.api().style_origin)(std.ctx(), node, property.raw(), &mut out) })?;
            Ok(lent_text(&out))
        };
        let background = scalar(StyleProperty::Background)?;
        Ok(ComputedStyle {
            opacity: scalar(StyleProperty::Opacity)?.a,
            corner_radius: radius_of(&scalar(StyleProperty::CornerRadius)?),
            background: (background.kind == abi::STD_PROP_TEXT).then(|| lent_text(&background.text)),
            borders: list(StyleProperty::Borders)?.iter().map(border_of).collect(),
            shadows: list(StyleProperty::Shadows)?.iter().map(shadow_of).collect(),
            filter: list(StyleProperty::Filter)?.iter().filter_map(filter_of).collect(),
            backdrop_filter: list(StyleProperty::BackdropFilter)?
                .iter()
                .filter_map(filter_of)
                .collect(),
            origin: [
                origin(StyleProperty::Opacity)?,
                origin(StyleProperty::CornerRadius)?,
                origin(StyleProperty::Background)?,
                origin(StyleProperty::Borders)?,
                origin(StyleProperty::Shadows)?,
                origin(StyleProperty::Filter)?,
                origin(StyleProperty::BackdropFilter)?,
            ],
        })
    }

    /// What the client's style asks for that is not drawn, then what the compositor reports it skipped: `not applied: style.<what>`
    /// and `the compositor does not draw the effect <kind>: <reasons>`.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn style_warnings(&self) -> Result<Vec<String>> {
        let std = self.std();
        let mut out = core_abi::GnoblinStr::EMPTY;
        // SAFETY: `style_notes` writes a string it lends through the pointer it is given; it is copied now.
        std.check(unsafe { (std.api().style_notes)(std.ctx(), self.window().id().0, &mut out) })?;
        let text = lent_text(&out);
        Ok(text.lines().map(str::to_owned).collect())
    }
}

impl Collection<Client> {
    /// The style of this collection of clients: the global style for the root, a workspace's for its clients, a selection's for a
    /// `where_`.
    ///
    /// # Errors
    ///
    /// The generation retired.
    pub fn style(&self) -> Result<Style> {
        Style::scope(self.std_ref(), abi::STD_STYLE_SCOPE_COLLECTION, self.raw_id())
    }
}
