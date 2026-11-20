//! The style functions of the std table: scopes, scalars, lists, and what a client's cascade resolved to.
#![allow(unsafe_code)]

use std::cell::RefCell;

use gnoblin_core_abi::{self as abi, GnoblinCtx, GnoblinNode, GnoblinStr};
use gnoblin_std_abi as sabi;
use gnoblin_std_abi::{GnoblinStdProp, GnoblinStdStyle, GnoblinStdStyleEntries, GnoblinStdStyleEntry};

use super::cascade::{Computed, Scalar, Scope};
use super::edit::ScalarIn;
use super::values::{self, Entry, Prop, Radii, ScalarProp};
use crate::animation::table::prop_struct;
use crate::error::{Fail, Res};
// The style functions need no desktop: they open the library's model alone (`state::lookup_quiet`).
use crate::table::{call, fill_out, lent, mutate_quiet as mutate, put, read_ext, read_quiet as read};

/// The entries a read lent, with the colours, shader sources and shader numbers they point at.
struct LentEntries {
    raws: Vec<GnoblinStdStyleEntry>,
    colours: Vec<String>,
    sources: Vec<String>,
    params: Vec<Vec<f64>>,
}

thread_local! {
    /// What a read lent, valid until the next call on this thread.
    static LENT_ENTRIES: RefCell<LentEntries> = const {
        RefCell::new(LentEntries {
            raws: Vec::new(),
            colours: Vec::new(),
            sources: Vec::new(),
            params: Vec::new(),
        })
    };
    /// The text a read lent.
    static LENT_TEXT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Lends `entries` as an array valid until the next call on this thread.
fn lend_entries(entries: &[Entry]) -> GnoblinStdStyleEntries {
    LENT_ENTRIES.with(|lent_entries| {
        let mut held = lent_entries.borrow_mut();
        let LentEntries {
            raws,
            colours,
            sources,
            params,
        } = &mut *held;
        raws.clear();
        colours.clear();
        sources.clear();
        params.clear();
        // What the pointers point into is filled first and never moved again, so the pointers stay put.
        for entry in entries {
            colours.push(values::entry_out(entry).3.unwrap_or("").to_owned());
            let (source, numbers) = values::entry_shader(entry);
            sources.push(source.to_owned());
            params.push(numbers.to_vec());
        }
        for (at, entry) in entries.iter().enumerate() {
            let (kind, inset, [a, b, c, d], _) = values::entry_out(entry);
            raws.push(GnoblinStdStyleEntry {
                size: size_of::<GnoblinStdStyleEntry>() as u32,
                kind,
                inset: u32::from(inset),
                reserved: 0,
                a,
                b,
                c,
                d,
                color: lent(&colours[at]),
                source: lent(&sources[at]),
                params: if params[at].is_empty() {
                    core::ptr::null()
                } else {
                    params[at].as_ptr()
                },
                n_params: params[at].len(),
            });
        }
        GnoblinStdStyleEntries {
            ptr: if raws.is_empty() {
                core::ptr::null()
            } else {
                raws.as_ptr()
            },
            len: raws.len(),
        }
    })
}

/// A scalar as the ABI hands it back.
fn scalar_prop(scalar: &Scalar) -> GnoblinStdProp {
    match scalar {
        Scalar::Opacity(value) => prop_struct(Some(&crate::animation::staging::Prop::Number(*value))),
        Scalar::CornerRadius(radii) => radii_prop(*radii),
        Scalar::Background(Some(colour)) => prop_struct(Some(&crate::animation::staging::Prop::Text(colour.clone()))),
        Scalar::Background(None) => {
            let mut raw = prop_struct(None);
            raw.kind = sabi::STD_PROP_NULL;
            raw
        }
    }
}

fn radii_prop(radii: Radii) -> GnoblinStdProp {
    match radii {
        Radii::All(value) => prop_struct(Some(&crate::animation::staging::Prop::Number(value))),
        Radii::Corners([a, b, c, d]) => {
            let mut raw = prop_struct(None);
            raw.kind = sabi::STD_PROP_CORNERS;
            (raw.a, raw.b, raw.c, raw.d) = (a, b, c, d);
            raw
        }
    }
}

/// The resolved scalar `prop` of a computed style.
fn computed_scalar(computed: &Computed, prop: ScalarProp) -> GnoblinStdProp {
    scalar_prop(&match prop {
        ScalarProp::Opacity => Scalar::Opacity(computed.opacity),
        ScalarProp::CornerRadius => Scalar::CornerRadius(computed.corner_radius),
        ScalarProp::Background => Scalar::Background(computed.background.clone()),
    })
}

pub(crate) unsafe extern "C" fn style_scope(
    ctx: GnoblinCtx,
    scope: u32,
    target: u64,
    out: *mut GnoblinStdStyle,
) -> i32 {
    call("style_scope", || {
        let scope = match scope {
            sabi::STD_STYLE_SCOPE_COLLECTION => Scope::Collection(target),
            sabi::STD_STYLE_SCOPE_CLIENT => Scope::Client(target),
            other => return Err(Fail::type_error(format!("{other} is not a style scope"))),
        };
        let id = mutate(ctx, |st| st.style_for(scope))?;
        // SAFETY: the caller guarantees `out` is null or a writable handle.
        unsafe { put(out, id) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_set(
    ctx: GnoblinCtx,
    style: GnoblinStdStyle,
    prop: u32,
    value: *const GnoblinStdProp,
) -> i32 {
    call("style_set", || {
        let Some(Prop::Scalar(prop)) = Prop::from_abi(prop) else {
            return Err(Fail::type_error(format!("{prop} is not a scalar style property")));
        };
        // SAFETY: the caller guarantees `value` is null or a readable struct; its text is read below, for the call.
        let raw = unsafe { read_ext(value, "a style value") }?;
        mutate(ctx, |st| {
            let scope = st.style_scope_of(style)?;
            st.style_guard(scope)?;
            let path = format!("{}.{}", st.style_path(scope), Prop::Scalar(prop).name());
            let input = match (prop, raw.kind) {
                (_, sabi::STD_PROP_UNSET) => return st.style_remove(style, Some(Prop::Scalar(prop))),
                (ScalarProp::Opacity, sabi::STD_PROP_NUMBER) => ScalarIn::Opacity(raw.a),
                (ScalarProp::CornerRadius, sabi::STD_PROP_NUMBER) => ScalarIn::CornerRadius(Radii::All(raw.a)),
                (ScalarProp::CornerRadius, sabi::STD_PROP_CORNERS) => {
                    ScalarIn::CornerRadius(Radii::Corners([raw.a, raw.b, raw.c, raw.d]))
                }
                (ScalarProp::Background, sabi::STD_PROP_NULL) => ScalarIn::Background(None),
                (ScalarProp::Background, sabi::STD_PROP_TEXT) => {
                    // SAFETY: the caller guarantees the text is readable for the call.
                    let text = unsafe { crate::table::utf8(raw.text.ptr, raw.text.len) }?;
                    ScalarIn::Background(Some(text.to_owned()))
                }
                (ScalarProp::Opacity, _) => return Err(values::fail(&path, "must be a finite number between 0 and 1")),
                (ScalarProp::CornerRadius, _) => {
                    return Err(values::fail(
                        &path,
                        "must be a finite number >= 0 or { topLeft, topRight, bottomRight, bottomLeft }",
                    ));
                }
                (ScalarProp::Background, _) => {
                    return Err(values::fail(
                        &path,
                        "must be a #rgb, #rgba, #rrggbb or #rrggbbaa colour, or null",
                    ));
                }
            };
            st.style_put_scalar(style, &input)
        })?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_set_list(
    ctx: GnoblinCtx,
    style: GnoblinStdStyle,
    prop: u32,
    entries: *const GnoblinStdStyleEntry,
    n: usize,
) -> i32 {
    call("style_set_list", || {
        let Some(Prop::List(list)) = Prop::from_abi(prop) else {
            return Err(Fail::type_error(format!("{prop} is not a list style property")));
        };
        let raws: &[GnoblinStdStyleEntry] = if entries.is_null() || n == 0 {
            &[]
        } else {
            // SAFETY: the caller guarantees `n` readable entries at `entries`.
            unsafe { std::slice::from_raw_parts(entries, n) }
        };
        mutate(ctx, |st| {
            let scope = st.style_scope_of(style)?;
            st.style_guard(scope)?;
            let path = format!("{}.{}", st.style_path(scope), Prop::List(list).name());
            if raws.len() > values::MAX_STYLE_LIST {
                return Err(values::fail(
                    &path,
                    &format!("holds at most {} entries, got {}", values::MAX_STYLE_LIST, raws.len()),
                ));
            }
            let checked = raws
                .iter()
                .enumerate()
                .map(|(at, raw)| values::entry_in(list, raw, &format!("{path}[{at}]")))
                .collect::<Res<Vec<Entry>>>()?;
            st.style_put_entries(style, list, checked)
        })?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_unset(ctx: GnoblinCtx, style: GnoblinStdStyle, prop: u32) -> i32 {
    call("style_unset", || {
        let which = if prop == 0 {
            None
        } else {
            Some(Prop::from_abi(prop).ok_or_else(|| Fail::type_error(format!("{prop} is not a style property")))?)
        };
        mutate(ctx, |st| st.style_remove(style, which))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_get(
    ctx: GnoblinCtx,
    style: GnoblinStdStyle,
    prop: u32,
    out: *mut GnoblinStdProp,
) -> i32 {
    call("style_get", || {
        let Some(Prop::Scalar(prop)) = Prop::from_abi(prop) else {
            return Err(Fail::type_error(format!("{prop} is not a scalar style property")));
        };
        let raw = read(ctx, |st| {
            let store = st
                .styles
                .stores
                .get(&style)
                .ok_or_else(|| Fail::stale(format!("style {style} does not exist")))?;
            Ok(store.scalars.get(&prop).map_or_else(|| prop_struct(None), scalar_prop))
        })?;
        // SAFETY: the caller guarantees `out` is a writable struct whose first field is its size.
        unsafe { fill_out(out, &raw, "style_get") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_list(
    ctx: GnoblinCtx,
    style: GnoblinStdStyle,
    prop: u32,
    out: *mut GnoblinStdStyleEntries,
) -> i32 {
    call("style_list", || {
        let Some(Prop::List(list)) = Prop::from_abi(prop) else {
            return Err(Fail::type_error(format!("{prop} is not a list style property")));
        };
        let entries = read(ctx, |st| {
            let store = st
                .styles
                .stores
                .get(&style)
                .ok_or_else(|| Fail::stale(format!("style {style} does not exist")))?;
            Ok(match store.lists.get(&list) {
                Some(own) => own.clone(),
                None => st.layered_list(&st.layers_below(store.scope), list),
            })
        })?;
        // SAFETY: the caller guarantees `out` is null or a writable struct.
        unsafe { put(out, lend_entries(&entries)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_computed(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    prop: u32,
    out: *mut GnoblinStdProp,
) -> i32 {
    call("style_computed", || {
        let Some(Prop::Scalar(prop)) = Prop::from_abi(prop) else {
            return Err(Fail::type_error(format!("{prop} is not a scalar style property")));
        };
        let raw = read(ctx, |st| Ok(computed_scalar(&st.computed_style(client), prop)))?;
        // SAFETY: the caller guarantees `out` is a writable struct whose first field is its size.
        unsafe { fill_out(out, &raw, "style_computed") }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_computed_list(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    prop: u32,
    out: *mut GnoblinStdStyleEntries,
) -> i32 {
    call("style_computed_list", || {
        let Some(Prop::List(list)) = Prop::from_abi(prop) else {
            return Err(Fail::type_error(format!("{prop} is not a list style property")));
        };
        let entries = read(ctx, |st| Ok(st.computed_style(client).list(list).to_vec()))?;
        // SAFETY: the caller guarantees `out` is null or a writable struct.
        unsafe { put(out, lend_entries(&entries)) }?;
        Ok(abi::E_OK)
    })
}

/// Lends `text` as a string valid until the next call on this thread.
fn lend_text(text: String) -> GnoblinStr {
    LENT_TEXT.with(|lent_text| {
        *lent_text.borrow_mut() = text;
        lent(&lent_text.borrow())
    })
}

pub(crate) unsafe extern "C" fn style_origin(
    ctx: GnoblinCtx,
    client: GnoblinNode,
    prop: u32,
    out: *mut GnoblinStr,
) -> i32 {
    call("style_origin", || {
        let prop = Prop::from_abi(prop).ok_or_else(|| Fail::type_error(format!("{prop} is not a style property")))?;
        let origin = read(ctx, |st| {
            Ok(st
                .computed_style(client)
                .origin
                .get(prop.name())
                .cloned()
                .unwrap_or_else(|| "default".to_owned()))
        })?;
        // SAFETY: the caller guarantees `out` is null or a writable string.
        unsafe { put(out, lend_text(origin)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_notes(ctx: GnoblinCtx, client: GnoblinNode, out: *mut GnoblinStr) -> i32 {
    call("style_notes", || {
        let notes = read(ctx, |st| Ok(st.style_warnings(client).join("\n")))?;
        // SAFETY: the caller guarantees `out` is null or a writable string.
        unsafe { put(out, lend_text(notes)) }?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_rank(ctx: GnoblinCtx, style: GnoblinStdStyle, rank: f64) -> i32 {
    call("style_rank", || {
        mutate(ctx, |st| st.style_set_rank(style, rank))?;
        Ok(abi::E_OK)
    })
}

pub(crate) unsafe extern "C" fn style_reset(ctx: GnoblinCtx) -> i32 {
    call("style_reset", || {
        mutate(ctx, |st| {
            let reloaded = st.g.info().is_ok_and(|info| info.reload_count > 0);
            st.styles.reset(reloaded);
            Ok(())
        })?;
        Ok(abi::E_OK)
    })
}
