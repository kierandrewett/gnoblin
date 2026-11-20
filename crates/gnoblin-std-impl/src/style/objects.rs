//! The classes of `style.toml`: `style`, the lists a style is edited through (`style_list` and its three kinds), and the records they
//! take and give (`shadow`, `border`, `filter`, `shader`, `corner_radius`, `style_values`, `style_query`, `computed_style`).
//!
//! `data/js/gnoblin-std/style/` is the reference. A style object is a name for a scope of the library's cascade (`cascade.rs`): the
//! values live in the library's `StyleState`, so the functions of the std table and these classes see one set of styles, and a value is
//! refused with the same words by whichever way it came (`edit.rs`, `values.rs`). What the TypeScript classes did by intercepting
//! writes to the arrays and the elements they handed out is done here by values (read a list, change it, write it back) and by the
//! list objects, which make the scope's own copy of an inherited list on the first change.

use std::cell::RefCell;
use std::collections::HashMap;

use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, ClassBuilder, Cx};
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_std_abi as sabi;

use super::cascade::Scalar;
use super::edit::ScalarIn;
use super::values::{self, Draft, Entry, ListProp, Prop, Radii, ScalarProp};
use crate::collection::Want;
use crate::error::Res as StdRes;
use crate::object::record::{
    edit, enumeration, field, flag_field, look, number_field, numbers_field, record, text_field,
};
use crate::state::{Id, State};

thread_local! {
    /// The object of each style: one for each scope, so asking for a scope again gives the one object.
    static STYLE_OBJECTS: RefCell<HashMap<(GnoblinCtx, Id), Obj>> = RefCell::new(HashMap::new());
}

/// What a `style` object holds: where its style lives.
struct StyleRef {
    ctx: GnoblinCtx,
    style: Id,
}

/// What a list object holds: the style it edits and which of its lists.
struct ListRef {
    ctx: GnoblinCtx,
    style: Id,
    list: ListProp,
}

// ---- values ---------------------------------------------------------------------------------------------------------------------

fn some_number(value: f64) -> Val {
    Val::Double(value)
}

/// The place of a value of `style_property`, counted from one, which is `Prop::ALL`'s order.
fn property_val(prop: Prop) -> Val {
    enumeration(Prop::ALL.iter().position(|each| *each == prop).unwrap_or(0) as u32 + 1)
}

fn property_of(value: &Val) -> Res<Prop> {
    let place = value.enum_value()?;
    Prop::ALL
        .get((place as usize).wrapping_sub(1))
        .copied()
        .ok_or_else(|| Fail::type_error("not a value of `style_property`"))
}

/// The places of `filter_kind`, counted from one.
const BLUR: u32 = 1;
const SATURATE: u32 = 2;
const BRIGHTNESS: u32 = 3;
const CONTRAST: u32 = 4;
const SHADER: u32 = 5;

/// A list element in the pieces a draft is made of.
struct Parts {
    kind: u32,
    inset: bool,
    numbers: [f64; 4],
    color: String,
    source: String,
    params: Vec<f64>,
}

impl Parts {
    fn draft(&self) -> Draft<'_> {
        Draft {
            kind: self.kind,
            inset: self.inset,
            a: self.numbers[0],
            b: self.numbers[1],
            c: self.numbers[2],
            d: self.numbers[3],
            color: &self.color,
            source: &self.source,
            params: &self.params,
        }
    }
}

/// The pieces of a `shadow` record: its fields, a missing one as the default.
fn shadow_parts(value: &Val) -> Res<Parts> {
    let fields = value.record()?;
    Ok(Parts {
        kind: sabi::STD_ENTRY_SHADOW,
        inset: flag_field(fields, 5, false)?,
        numbers: [
            number_field(fields, 0, 0.0)?,
            number_field(fields, 1, 0.0)?,
            number_field(fields, 2, 12.0)?,
            number_field(fields, 3, 0.0)?,
        ],
        color: text_field(fields, 4, "#00000080")?,
        source: String::new(),
        params: Vec::new(),
    })
}

/// The pieces of a `border` record.
fn border_parts(value: &Val) -> Res<Parts> {
    let fields = value.record()?;
    Ok(Parts {
        kind: sabi::STD_ENTRY_BORDER,
        inset: flag_field(fields, 3, false)?,
        numbers: [number_field(fields, 0, 1.0)?, number_field(fields, 2, 0.0)?, 0.0, 0.0],
        color: text_field(fields, 1, "#ffffff")?,
        source: String::new(),
        params: Vec::new(),
    })
}

/// The pieces of a `filter` record: a blur's radius or a grade's gain when it has none is what the TypeScript filter had.
fn filter_parts(value: &Val) -> Res<Parts> {
    let fields = value.record()?;
    let kind = field(fields, 0)
        .ok_or_else(|| Fail::type_error("a filter needs its `kind`"))?
        .enum_value()?;
    let shader = field(fields, 2).map(Val::record).transpose()?;
    if (kind == SHADER) != shader.is_some() {
        return Err(Fail::type_error(if kind == SHADER {
            "a `shader` filter needs its `shader`"
        } else {
            "only a `shader` filter has a `shader`"
        }));
    }
    let (wire, default) = match kind {
        BLUR => (sabi::STD_ENTRY_BLUR, 12.0),
        SATURATE => (sabi::STD_ENTRY_SATURATE, 1.0),
        BRIGHTNESS => (sabi::STD_ENTRY_BRIGHTNESS, 1.0),
        CONTRAST => (sabi::STD_ENTRY_CONTRAST, 1.0),
        SHADER => (sabi::STD_ENTRY_SHADER, 0.0),
        _ => return Err(Fail::type_error("not a value of `filter_kind`")),
    };
    let (source, params) = match shader {
        Some(shader) => (
            field(shader, 0)
                .ok_or_else(|| Fail::type_error("a `shader` needs its `source`"))?
                .text()?
                .to_owned(),
            numbers_field(shader, 1)?,
        ),
        None => (String::new(), Vec::new()),
    };
    Ok(Parts {
        kind: wire,
        inset: false,
        numbers: [number_field(fields, 1, default)?, 0.0, 0.0, 0.0],
        color: String::new(),
        source,
        params,
    })
}

/// The converter of a list's entries from records.
fn parts_of(list: ListProp, value: &Val) -> Res<Parts> {
    match list {
        ListProp::Shadows => shadow_parts(value),
        ListProp::Borders => border_parts(value),
        ListProp::Filter | ListProp::BackdropFilter => filter_parts(value),
    }
}

fn list_parts(list: ListProp, value: &Val) -> Res<Vec<Parts>> {
    value.list()?.iter().map(|item| parts_of(list, item)).collect()
}

/// A list element as a record of its class.
fn entry_val(entry: &Entry) -> Val {
    match entry {
        Entry::Shadow {
            x,
            y,
            blur,
            spread,
            color,
            inset,
        } => record(vec![
            some_number(*x),
            some_number(*y),
            some_number(*blur),
            some_number(*spread),
            Val::Text(color.clone()),
            Val::Bool(*inset),
        ]),
        Entry::Border {
            width,
            color,
            offset,
            inset,
        } => record(vec![
            some_number(*width),
            Val::Text(color.clone()),
            some_number(*offset),
            Val::Bool(*inset),
        ]),
        Entry::Blur(radius) => filter_val(BLUR, Some(*radius), None),
        Entry::Saturate(amount) => filter_val(SATURATE, Some(*amount), None),
        Entry::Brightness(amount) => filter_val(BRIGHTNESS, Some(*amount), None),
        Entry::Contrast(amount) => filter_val(CONTRAST, Some(*amount), None),
        Entry::Shader { source, params } => filter_val(
            SHADER,
            None,
            Some(record(vec![
                Val::Text(source.clone()),
                Val::List(params.iter().map(|param| Val::Double(*param)).collect()),
            ])),
        ),
    }
}

fn filter_val(kind: u32, amount: Option<f64>, shader: Option<Val>) -> Val {
    record(vec![
        Val::Enum { class: 0, value: kind },
        amount.map_or(Val::None, Val::Double),
        shader.unwrap_or(Val::None),
    ])
}

fn entries_val(entries: &[Entry]) -> Val {
    Val::List(entries.iter().map(entry_val).collect())
}

/// The radii a `corner_radius` record gives: one for all corners, or the four of them. A corner left out is not a number, so it is
/// refused with the words a missing corner had.
fn radii_of(value: &Val) -> Res<Radii> {
    let fields = value.record()?;
    let all = field(fields, 0).map(Val::double).transpose()?;
    let corners = [1, 2, 3, 4].map(|at| field(fields, at).map(Val::double).transpose());
    let mut given = [None; 4];
    for (at, corner) in corners.into_iter().enumerate() {
        given[at] = corner?;
    }
    match all {
        Some(all) if given.iter().all(Option::is_none) => Ok(Radii::All(all)),
        Some(_) => Err(Fail::type_error(
            "cornerRadius: give one radius for all corners, or one for each corner, and not both",
        )),
        None => Ok(Radii::Corners(given.map(|corner| corner.unwrap_or(f64::NAN)))),
    }
}

fn radii_val(radii: Radii) -> Val {
    match radii {
        Radii::All(all) => record(vec![Val::Double(all), Val::None, Val::None, Val::None, Val::None]),
        Radii::Corners([a, b, c, d]) => record(vec![
            Val::None,
            Val::Double(a),
            Val::Double(b),
            Val::Double(c),
            Val::Double(d),
        ]),
    }
}

/// The text of a background: the empty text is none.
fn background_of(value: &Val) -> Res<Option<String>> {
    let text = value.text()?;
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

fn background_val(background: Option<&str>) -> Val {
    Val::Text(background.unwrap_or("").to_owned())
}

/// A scalar as the value its property holds.
fn scalar_val(scalar: &Scalar) -> Val {
    match scalar {
        Scalar::Opacity(value) => Val::Double(*value),
        Scalar::CornerRadius(radii) => radii_val(*radii),
        Scalar::Background(background) => background_val(background.as_deref()),
    }
}

/// The terms of a `style_query` record, in the order of its fields: the keys the library selects on, each given value one term, and each
/// alternative of an `any_` field one term of the key it stands for.
fn terms_of(value: &Val) -> Res<Vec<(String, Want)>> {
    let fields = value.record()?;
    let mut terms = Vec::new();
    let texts = [(0, "id"), (1, "app_id"), (2, "title"), (3, "workspace")];
    for (at, key) in texts {
        if let Some(text) = field(fields, at) {
            terms.push((key.to_owned(), Want::Text(text.text()?.to_owned())));
        }
    }
    let flags = [(4, "focused"), (5, "fullscreen"), (6, "maximised"), (7, "minimised")];
    for (at, key) in flags {
        if let Some(flag) = field(fields, at) {
            terms.push((key.to_owned(), Want::Bool(flag.bool()?)));
        }
    }
    let numbers = [(8, "x"), (9, "y"), (10, "width"), (11, "height")];
    for (at, key) in numbers {
        if let Some(number) = field(fields, at) {
            terms.push((key.to_owned(), Want::Number(number.double()?)));
        }
    }
    let alternatives = [(12, "id"), (13, "app_id"), (14, "title")];
    for (at, key) in alternatives {
        if let Some(list) = field(fields, at) {
            for item in list.list()? {
                terms.push((key.to_owned(), Want::Text(item.text()?.to_owned())));
            }
        }
    }
    Ok(terms)
}

// ---- the style --------------------------------------------------------------------------------------------------------------------

/// The object of a style, made the first time it is asked for.
fn style_val(cx: &mut Cx<'_>, id: Id) -> Res<Val> {
    let ctx = cx.ctx();
    let known = STYLE_OBJECTS.with(|objects| objects.borrow().get(&(ctx, id)).copied());
    if let Some(obj) = known
        && cx
            .with_state(obj, |held: &mut StyleRef| held.ctx == ctx && held.style == id)
            .unwrap_or(false)
    {
        return cx.handout(obj);
    }
    let obj = cx.spawn("style", StyleRef { ctx, style: id })?;
    STYLE_OBJECTS.with(|objects| objects.borrow_mut().insert((ctx, id), obj));
    cx.handout(obj)
}

/// The object of a list of a style, for the shell to edit through.
fn list_val(cx: &mut Cx<'_>, style: Id, list: ListProp) -> Res<Val> {
    let class = match list {
        ListProp::Shadows => "shadow_list",
        ListProp::Borders => "border_list",
        ListProp::Filter | ListProp::BackdropFilter => "filter_list",
    };
    let obj = cx.spawn(
        class,
        ListRef {
            ctx: cx.ctx(),
            style,
            list,
        },
    )?;
    cx.handout(obj)
}

type StyleClass<'a> = ClassBuilder<'a, StyleRef>;

/// The scalar members: what the scope sets, else what it inherits, and a write that is checked.
fn scalar_members(class: StyleClass<'_>) -> StyleClass<'_> {
    class
        .get("opacity", |h, _| {
            look(h.ctx, |st| st.style_scalar(h.style, ScalarProp::Opacity)).map(|scalar| scalar_val(&scalar))
        })
        .set("opacity", |h, _, value| {
            let input = ScalarIn::Opacity(value.double()?);
            edit(h.ctx, |st| st.style_put_scalar(h.style, &input))
        })
        .get("corner_radius", |h, _| {
            look(h.ctx, |st| st.style_scalar(h.style, ScalarProp::CornerRadius)).map(|scalar| scalar_val(&scalar))
        })
        .set("corner_radius", |h, _, value| {
            let input = ScalarIn::CornerRadius(radii_of(&value)?);
            edit(h.ctx, |st| st.style_put_scalar(h.style, &input))
        })
        .get("background", |h, _| {
            look(h.ctx, |st| st.style_scalar(h.style, ScalarProp::Background)).map(|scalar| scalar_val(&scalar))
        })
        .set("background", |h, _, value| {
            let input = ScalarIn::Background(background_of(&value)?);
            edit(h.ctx, |st| st.style_put_scalar(h.style, &input))
        })
}

/// The list members: the entries the scope sets, else what it inherits, and a write of the whole list that is checked.
fn list_members(class: StyleClass<'_>) -> StyleClass<'_> {
    let mut class = class;
    for (name, list) in [
        ("borders", ListProp::Borders),
        ("shadows", ListProp::Shadows),
        ("filter", ListProp::Filter),
        ("backdrop_filter", ListProp::BackdropFilter),
    ] {
        class = class
            .get(name, move |h, _| {
                look(h.ctx, |st| st.style_entries(h.style, list)).map(|(entries, _)| entries_val(&entries))
            })
            .set(name, move |h, _, value| put_items(h.ctx, h.style, list, &value));
    }
    class
}

/// Sets a list from the records of a value, every entry checked first.
fn put_items(ctx: GnoblinCtx, style: Id, list: ListProp, value: &Val) -> Res<()> {
    let parts = list_parts(list, value)?;
    let drafts: Vec<Draft<'_>> = parts.iter().map(Parts::draft).collect();
    edit(ctx, |st| st.style_put_drafts(style, list, &drafts))
}

/// What the scope sets, as a `style_values` record.
fn values_of(st: &State, style: Id) -> StdRes<Val> {
    let mut fields = Vec::new();
    for prop in [ScalarProp::Opacity, ScalarProp::CornerRadius, ScalarProp::Background] {
        fields.push(
            st.style_own_scalar(style, prop)?
                .map_or(Val::None, |scalar| scalar_val(&scalar)),
        );
    }
    for list in [
        ListProp::Borders,
        ListProp::Shadows,
        ListProp::Filter,
        ListProp::BackdropFilter,
    ] {
        let (entries, own) = st.style_entries(style, list)?;
        fields.push(if own { entries_val(&entries) } else { Val::None });
    }
    Ok(record(fields))
}

/// Makes the scope set exactly what a `style_values` record says.
fn assign_values(ctx: GnoblinCtx, style: Id, value: &Val) -> Res<()> {
    let fields = value.record()?;
    let mut scalars = Vec::new();
    if let Some(opacity) = field(fields, 0) {
        scalars.push(ScalarIn::Opacity(opacity.double()?));
    }
    if let Some(radius) = field(fields, 1) {
        scalars.push(ScalarIn::CornerRadius(radii_of(radius)?));
    }
    if let Some(background) = field(fields, 2) {
        scalars.push(ScalarIn::Background(background_of(background)?));
    }
    let mut parts: Vec<(ListProp, Vec<Parts>)> = Vec::new();
    for (at, list) in [
        (3, ListProp::Borders),
        (4, ListProp::Shadows),
        (5, ListProp::Filter),
        (6, ListProp::BackdropFilter),
    ] {
        if let Some(items) = field(fields, at) {
            parts.push((list, list_parts(list, items)?));
        }
    }
    let lists: Vec<(ListProp, Vec<Draft<'_>>)> = parts
        .iter()
        .map(|(list, held)| (*list, held.iter().map(Parts::draft).collect()))
        .collect();
    edit(ctx, |st| st.style_replace(style, scalars, lists))
}

/// A client's resolved style as a `computed_style` record.
fn computed_val(st: &State, client: u64) -> Val {
    let computed = st.computed_style(client);
    let origin = Prop::ALL.map(|prop| {
        Val::Text(
            computed
                .origin
                .get(prop.name())
                .cloned()
                .unwrap_or_else(|| "default".to_owned()),
        )
    });
    record(vec![
        Val::Double(computed.opacity),
        radii_val(computed.corner_radius),
        background_val(computed.background.as_deref()),
        entries_val(computed.list(ListProp::Borders)),
        entries_val(computed.list(ListProp::Shadows)),
        entries_val(computed.list(ListProp::Filter)),
        entries_val(computed.list(ListProp::BackdropFilter)),
        record(origin.to_vec()),
    ])
}

/// A style of the scope the library made, as the value that hands it to the shell.
fn made_style(cx: &mut Cx<'_>, work: impl FnOnce(&mut State) -> StdRes<Id>) -> Res<Val> {
    let id = edit(cx.ctx(), work)?;
    style_val(cx, id)
}

fn register_style(builder: &mut Builder) {
    let class = builder.class::<StyleRef>("style").ctor(|_, _| {
        Err(Fail::refused(
            "a style is the style of a scope: ask for it with `of_clients`, `of_workspace`, `of_client` or `where`",
        ))
    });
    let class = scalar_members(class);
    let _ = list_members(class)
        .get("path", |h, _| {
            look(h.ctx, |st| st.style_scope_of(h.style).map(|scope| st.style_path(scope))).map(Val::Text)
        })
        .get("alive", |h, _| {
            look(h.ctx, |st| match st.style_scope_of(h.style)? {
                super::cascade::Scope::Client(node) => Ok(crate::nodes::alive(&st.g, node)),
                super::cascade::Scope::Collection(_) => Ok(true),
            })
            .map(Val::Bool)
        })
        .get("priority", |h, _| {
            look(h.ctx, |st| st.style_rank_of(h.style)).map(Val::Double)
        })
        .set("priority", |h, _, value| {
            let rank = value.double()?;
            edit(h.ctx, |st| st.style_set_rank(h.style, rank))
        })
        .method("is_set", |h, _, args| {
            let prop = property_of(&args[0])?;
            look(h.ctx, |st| {
                Ok(match prop {
                    Prop::Scalar(scalar) => st.style_own_scalar(h.style, scalar)?.is_some(),
                    Prop::List(list) => st.style_entries(h.style, list)?.1,
                })
            })
            .map(Val::Bool)
        })
        .method("unset", |h, _, args| {
            let prop = property_of(&args[0])?;
            edit(h.ctx, |st| st.style_remove(h.style, Some(prop)))?;
            Ok(Val::None)
        })
        .method("reset", |h, _, _| {
            edit(h.ctx, |st| st.style_remove(h.style, None))?;
            Ok(Val::None)
        })
        .method("values", |h, _, _| look(h.ctx, |st| values_of(st, h.style)))
        .method("assign", |h, _, args| {
            assign_values(h.ctx, h.style, &args[0])?;
            Ok(Val::None)
        })
        .method("where", |h, cx, args| {
            let terms = terms_of(&args[0])?;
            let priority = (!args[1].is_none()).then(|| args[1].double()).transpose()?;
            made_style(cx, |st| st.style_where(h.style, terms, priority))
        })
        .method("shadow_list", |h, cx, _| list_val(cx, h.style, ListProp::Shadows))
        .method("border_list", |h, cx, _| list_val(cx, h.style, ListProp::Borders))
        .method("filter_list", |h, cx, _| list_val(cx, h.style, ListProp::Filter))
        .method("backdrop_filter_list", |h, cx, _| {
            list_val(cx, h.style, ListProp::BackdropFilter)
        })
        .function("of_clients", |cx, _| {
            made_style(cx, |st| st.style_for(super::cascade::Scope::Collection(st.roots[0])))
        })
        .function("of_client", |cx, args| {
            let client = args[0].uint()?;
            made_style(cx, |st| st.style_for(super::cascade::Scope::Client(client)))
        })
        .function("of_workspace", |cx, args| {
            let workspace = args[0].text()?.to_owned();
            made_style(cx, |st| st.style_of_workspace(&workspace))
        })
        .function("computed", |cx, args| {
            let client = args[0].uint()?;
            look(cx.ctx(), |st| Ok(computed_val(st, client)))
        })
        .function("warnings", |cx, args| {
            let client = args[0].uint()?;
            look(cx.ctx(), |st| {
                Ok(Val::List(
                    st.style_warnings(client).into_iter().map(Val::Text).collect(),
                ))
            })
        });
}

// ---- the lists ----------------------------------------------------------------------------------------------------------------------

type ListClass<'a> = ClassBuilder<'a, ListRef>;

/// The index an `index` argument names in a list of `len` entries, counted from the end when negative; `len` itself is allowed when
/// `end` says the position after the last entry is a place.
fn place(path: &str, index: i64, len: usize, end: bool) -> StdRes<usize> {
    let at = if index < 0 { len as i64 + index } else { index };
    let top = if end { len as i64 } else { len as i64 - 1 };
    if (0..=top).contains(&at) {
        Ok(at as usize)
    } else {
        Err(crate::error::Fail::new(
            gnoblin_core_abi::E_NO_SUCH_NODE,
            format!("{path}: there is no entry at {index}"),
        ))
    }
}

/// Changes the entries of a list: the scope's own, or a copy of what it inherits that the scope then owns. `work` gets the entries and
/// the path of the list, and a failure of it changes nothing.
fn change_list(h: &ListRef, work: impl FnOnce(&mut Vec<Entry>, &str) -> StdRes<()>) -> Res<()> {
    edit(h.ctx, |st| {
        let scope = st.style_scope_of(h.style)?;
        st.style_guard(scope)?;
        let path = format!("{}.{}", st.style_path(scope), Prop::List(h.list).name());
        let (mut entries, _) = st.style_entries(h.style, h.list)?;
        work(&mut entries, &path)?;
        st.style_put_entries(h.style, h.list, entries)
    })
}

/// Fails for a list that holds the most entries a list does.
fn room(entries: &[Entry], path: &str) -> StdRes<()> {
    if entries.len() >= values::MAX_STYLE_LIST {
        return Err(values::fail(
            path,
            &format!(
                "holds at most {} entries, so another cannot be added",
                values::MAX_STYLE_LIST
            ),
        ));
    }
    Ok(())
}

/// The entry a record of the list's kind stands for, checked as the entry at `at` of the list.
fn entry_at(list: ListProp, parts: &Parts, path: &str, at: usize) -> StdRes<Entry> {
    values::entry_from(list, &parts.draft(), &format!("{path}[{at}]"))
}

/// The members every kind of list has.
fn list_base(class: ListClass<'_>) -> ListClass<'_> {
    class
        .get("style", |h, cx| style_val(cx, h.style))
        .get("property", |h, _| Ok(property_val(Prop::List(h.list))))
        .get("length", |h, _| {
            look(h.ctx, |st| st.style_entries(h.style, h.list)).map(|(entries, _)| Val::Uint(entries.len() as u64))
        })
        .get("overridden", |h, _| {
            look(h.ctx, |st| st.style_entries(h.style, h.list)).map(|(_, own)| Val::Bool(own))
        })
        .method("remove", |h, _, args| {
            let index = args[0].int()?;
            change_list(h, |entries, path| {
                let at = place(path, index, entries.len(), false)?;
                entries.remove(at);
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("clear", |h, _, _| {
            change_list(h, |entries, _| {
                entries.clear();
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("unset", |h, _, _| {
            edit(h.ctx, |st| st.style_remove(h.style, Some(Prop::List(h.list))))?;
            Ok(Val::None)
        })
}

/// The members a kind of list has of its own: its entries as records.
fn list_entries(class: ListClass<'_>) -> ListClass<'_> {
    list_base(class)
        .get("items", |h, _| {
            look(h.ctx, |st| st.style_entries(h.style, h.list)).map(|(entries, _)| entries_val(&entries))
        })
        .set("items", |h, _, value| put_items(h.ctx, h.style, h.list, &value))
        .method("at", |h, _, args| {
            let index = args[0].int()?;
            let (entries, _) = look(h.ctx, |st| st.style_entries(h.style, h.list))?;
            let path = format!("style.{}", Prop::List(h.list).name());
            let at = place(&path, index, entries.len(), false).map_err(Fail::from)?;
            Ok(entry_val(&entries[at]))
        })
        .method("add", |h, _, args| {
            let parts = parts_of(h.list, &args[0])?;
            let mut made = None;
            change_list(h, |entries, path| {
                room(entries, path)?;
                let entry = entry_at(h.list, &parts, path, entries.len())?;
                made = Some(entry_val(&entry));
                entries.push(entry);
                Ok(())
            })?;
            Ok(made.unwrap_or(Val::None))
        })
        .method("insert", |h, _, args| {
            let index = args[0].int()?;
            let parts = parts_of(h.list, &args[1])?;
            change_list(h, |entries, path| {
                room(entries, path)?;
                let at = place(path, index, entries.len(), true)?;
                let entry = entry_at(h.list, &parts, path, at)?;
                entries.insert(at, entry);
                Ok(())
            })?;
            Ok(Val::None)
        })
        .method("replace", |h, _, args| {
            let index = args[0].int()?;
            let parts = parts_of(h.list, &args[1])?;
            change_list(h, |entries, path| {
                let at = place(path, index, entries.len(), false)?;
                entries[at] = entry_at(h.list, &parts, path, at)?;
                Ok(())
            })?;
            Ok(Val::None)
        })
}

/// A list is made by a style, never by itself.
fn refuse_list() -> Fail {
    Fail::refused(
        "a list is a list of a style: ask for it with `shadow_list`, `border_list` or `filter_list` of the style",
    )
}

fn register_lists(builder: &mut Builder) {
    let _ = list_base(builder.class::<ListRef>("style_list").ctor(|_, _| Err(refuse_list())));
    for name in ["shadow_list", "border_list", "filter_list"] {
        let _ = list_entries(builder.class::<ListRef>(name).ctor(|_, _| Err(refuse_list())));
    }
}

/// Implements the classes of the style module.
pub(crate) fn register(builder: &mut Builder) {
    register_style(builder);
    register_lists(builder);
}
