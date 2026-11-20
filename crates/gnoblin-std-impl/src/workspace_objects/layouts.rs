//! The layout objects: what a workspace holds as its layout, the floating layout, and the members every layout of the library has because
//! it extends the interface `layout` (`compute` and its hooks), run natively.
//!
//! A workspace holds a layout as an object. A layout of the library's own (the floating one, a built-in kind) is an object of a class that
//! extends `layout`: it holds the library's layout (`OwnedLayout`), which the workspace uses while it has it. A layout a shell implements is an
//! object of the interface itself, and the library calls the shell's methods (`shell_layout.rs`). Asking a workspace for its layout gives
//! the object it was given, or, for a layout the library made itself, an object made then and remembered.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gnoblin_core::Rect;
use gnoblin_core_abi::GnoblinCtx;
use gnoblin_object::server::{Builder, ClassBuilder, Cx, Inits};
use gnoblin_object::{Fail, Obj, Res, Val};
use gnoblin_std_abi as sabi;

use super::shell_layout::{self, LayoutMemberIds, ObjectLayout};
use super::values::{checked_rect, member_val, rect_of, rect_val, wire_of};
use crate::floating::{DEFAULT_CASCADE_STEP, PlaceMode, apart_from, place_rect};
use crate::layout::{Known, LayoutObj};
use crate::object::record::{field, flag_field, number_field};
use crate::state::Id;

/// The layouts a shell has an object for: the layout weakly, and the object.
type LayoutObjects = HashMap<(GnoblinCtx, usize), (Weak<LayoutObj>, Obj)>;

thread_local! {
    /// The object that stands for each layout a shell was given or made: by the layout's address, with a weak reference to check that
    /// the address still names that layout.
    static LAYOUT_OBJECTS: RefCell<LayoutObjects> = RefCell::new(HashMap::new());
}

/// What an object of a layout class of the library holds: the layout, and the workspace that uses it.
pub(super) struct OwnedLayout {
    pub(super) layout: Rc<LayoutObj>,
    /// The workspace the layout places, by the id the library gave it. A layout places one workspace.
    pub(super) on: Option<Id>,
}

fn key(ctx: GnoblinCtx, layout: &Rc<LayoutObj>) -> (GnoblinCtx, usize) {
    (ctx, Rc::as_ptr(layout) as usize)
}

/// Remembers that `obj` stands for `layout`.
pub(super) fn remember(ctx: GnoblinCtx, layout: &Rc<LayoutObj>, obj: Obj) {
    LAYOUT_OBJECTS.with(|held| {
        held.borrow_mut().insert(key(ctx, layout), (Rc::downgrade(layout), obj));
    });
}

/// The generation ended: the layouts it held are gone.
pub(super) fn retired(ctx: GnoblinCtx) {
    LAYOUT_OBJECTS.with(|held| held.borrow_mut().retain(|(owner, _), _| *owner != ctx));
}

/// The object that stands for `layout`, when a shell has one.
fn known(cx: &Cx<'_>, layout: &Rc<LayoutObj>) -> Option<Obj> {
    let found = LAYOUT_OBJECTS.with(|held| held.borrow().get(&key(cx.ctx(), layout)).cloned());
    let (weak, obj) = found?;
    let same = weak.upgrade().is_some_and(|alive| Rc::ptr_eq(&alive, layout));
    (same && cx.class_of(obj).is_ok()).then_some(obj)
}

/// The class and the state of the object a layout of the library is, or none for a layout a shell of the table made.
fn class_of_layout(layout: &Rc<LayoutObj>) -> Option<String> {
    match &layout.kind {
        crate::layout::Kind::Floating { .. } => Some("floating_layout".to_owned()),
        crate::layout::Kind::Builtin(builtin) => Some(format!("{}_layout", builtin.kind.name)),
        crate::layout::Kind::Foreign(_) | crate::layout::Kind::Object(_) => None,
    }
}

/// The object of `layout`, which `owner` uses: the one a shell has, or one made now for a layout the library made. Hands it out. None
/// for a layout a shell of the table gave, which has no object.
pub(super) fn object_of(cx: &mut Cx<'_>, layout: &Rc<LayoutObj>, owner: Id) -> Res<Val> {
    if let Some(obj) = known(cx, layout) {
        return cx.handout(obj);
    }
    let Some(class) = class_of_layout(layout) else {
        return Ok(Val::None);
    };
    let obj = cx.spawn(
        &class,
        OwnedLayout {
            layout: Rc::clone(layout),
            on: Some(owner),
        },
    )?;
    remember(cx.ctx(), layout, obj);
    cx.handout(obj)
}

/// What a workspace `owner` uses of the object `obj` it was given: a layout of the library's own (which then places this workspace and
/// no other), or a new layout that calls the shell's methods.
///
/// # Errors
///
/// The object is not a layout, or its layout places another workspace.
pub(super) fn adopt(cx: &mut Cx<'_>, obj: Obj, owner: Option<Id>) -> Res<Rc<LayoutObj>> {
    let class = cx.class_of(obj)?;
    if class == cx.class_named("layout")? {
        let shell = cx.shell(obj)?;
        let ids = LayoutMemberIds::of(cx)?;
        let layout = LayoutObj::object(ObjectLayout::new(cx.ctx(), shell, ids));
        remember(cx.ctx(), &layout, obj);
        return Ok(layout);
    }
    cx.with_state(obj, |held: &mut OwnedLayout| {
        match (held.on, owner) {
            (Some(on), Some(owner)) if on != owner => {
                return Err(Fail::refused(
                    "that layout already places another workspace: a layout places one workspace, so make another for this one",
                ));
            }
            (Some(_), None) => {
                return Err(Fail::refused(
                    "that layout already places a workspace: a layout places one workspace, so make another for this one",
                ));
            }
            _ => {}
        }
        held.on = owner;
        Ok(Rc::clone(&held.layout))
    })?
}

/// Records that the workspace `owner` has the layout `obj`.
pub(super) fn claimed(cx: &Cx<'_>, obj: Obj, owner: Id) {
    let _ = cx.with_state(obj, |held: &mut OwnedLayout| held.on = Some(owner));
}

/// The workspace stopped using the layout `obj`.
pub(super) fn released(cx: &Cx<'_>, obj: Obj) {
    let _ = cx.with_state(obj, |held: &mut OwnedLayout| held.on = None);
}

// ---- the members every layout of the library has ---------------------------------------------------------------------------------

/// The people of the layout's `compute`, and its hooks, as the library runs them. The context of a call is a value a shell makes by
/// hand, so the layout answers what it would answer a workspace with those clients and that area.
pub(super) fn layout_members<'a>(class: ClassBuilder<'a, OwnedLayout>) -> ClassBuilder<'a, OwnedLayout> {
    class
        .method("compute", |held, _, args| {
            let cx = shell_layout::context_of(&args[0])?;
            let placements = held.layout.compute(&cx).map_err(Fail::from)?;
            Ok(Val::List(placements.iter().map(shell_layout::placement_val).collect()))
        })
        .method("client_added", |held, cx, args| {
            let context = shell_layout::context_of(&args[0])?;
            let client = args[1].uint()?;
            let g = gnoblin_core::private::gnoblin_for(cx.ctx()).map_err(|error| Fail::err(error.to_string()))?;
            let none = HashMap::new();
            let floated = std::collections::HashSet::new();
            held.layout.client_added(
                &g,
                &context,
                client,
                &Known {
                    insets: &none,
                    floated: &floated,
                },
            );
            Ok(Val::None)
        })
        .method("client_removed", |held, cx, args| {
            let context = shell_layout::context_of(&args[0])?;
            let client = args[1].uint()?;
            if let Some(next) = held.layout.client_removed(&context, client)
                && let Ok(g) = gnoblin_core::private::gnoblin_for(cx.ctx())
            {
                let _ = g.window_handle(gnoblin_core::NodeId(next)).focus();
            }
            Ok(Val::None)
        })
        .method("area_changed", |held, _, args| {
            held.layout.area_changed(&rect_of(&args[0])?, &rect_of(&args[1])?);
            Ok(Val::None)
        })
        .method("clients_swapped", |held, _, args| {
            held.layout.clients_swapped(args[0].uint()?, args[1].uint()?);
            Ok(Val::None)
        })
        .method("revision", |held, _, _| Ok(Val::Uint(held.layout.revision())))
        .method("find_split", |held, _, args| {
            let (x, y) = (args[1].double()?, args[2].double()?);
            if !x.is_finite() || !y.is_finite() {
                return Err(Fail::type_error("a drag's point must be two finite numbers"));
            }
            Ok(Val::List(
                held.layout
                    .find_split(args[0].uint()?, x, y)
                    .iter()
                    .map(shell_layout::split_val)
                    .collect(),
            ))
        })
        .method("set_split", |held, _, args| {
            let ratio = args[1].double()?;
            let cross = if args[2].is_none() { f64::NAN } else { args[2].double()? };
            if !ratio.is_finite() || cross.is_infinite() {
                return Err(Fail::type_error("a divider's ratio must be a finite number"));
            }
            held.layout.set_split(args[0].uint()?, ratio, cross);
            Ok(Val::None)
        })
        .method("reset_split", |held, _, args| {
            held.layout.reset_split(args[0].uint()?);
            Ok(Val::None)
        })
}

// ---- the floating layout --------------------------------------------------------------------------------------------------------

/// The mode and the step the floating layout of `layout` has.
fn floating_of(layout: &LayoutObj) -> Option<(PlaceMode, f64)> {
    match &layout.kind {
        crate::layout::Kind::Floating { mode, step, .. } => Some((*mode, *step)),
        _ => None,
    }
}

fn mode_val(mode: PlaceMode) -> Val {
    member_val(match mode {
        PlaceMode::FreeSpace => sabi::STD_PLACE_SMART,
        PlaceMode::Center => sabi::STD_PLACE_CENTER,
        PlaceMode::Cascade => sabi::STD_PLACE_CASCADE,
    })
}

/// The placement mode a value of `place_mode` says.
pub(super) fn mode_of(value: &Val) -> Res<PlaceMode> {
    let wire = wire_of(value)?;
    PlaceMode::from_abi(wire).ok_or_else(|| Fail::type_error("not a value of `place_mode`"))
}

/// A cascade step that is finite and at least 0; 0 reads as the library's 32.
pub(super) fn step_of(value: f64) -> Res<f64> {
    if !value.is_finite() || value < 0.0 {
        return Err(Fail::type_error("a cascade step must be a finite number of at least 0"));
    }
    Ok(if value == 0.0 { DEFAULT_CASCADE_STEP } else { value })
}

fn floating_ctor(cx: &mut Cx<'_>, inits: &mut Inits) -> Res<OwnedLayout> {
    let mode = inits
        .take("placement")
        .map_or(Ok(PlaceMode::FreeSpace), |value| mode_of(&value))?;
    let step = inits
        .take("cascade_step")
        .map_or(Ok(DEFAULT_CASCADE_STEP), |value| step_of(value.double()?))?;
    let layout = LayoutObj::floating(mode, step);
    remember(cx.ctx(), &layout, cx.me());
    Ok(OwnedLayout { layout, on: None })
}

/// Where a new window of a size goes among the windows that are already there.
fn place(args: &[Val]) -> Res<Val> {
    let fields = args[0].record()?;
    let mode = match field(fields, 0) {
        Some(mode) => mode_of(mode)?,
        None => PlaceMode::FreeSpace,
    };
    let step = step_of(number_field(fields, 1, 0.0)?)?;
    let width = number_field(fields, 2, 0.0)?;
    let height = number_field(fields, 3, 0.0)?;
    let area = checked_rect(
        field(fields, 4).ok_or_else(|| Fail::type_error("a floating query needs its `area`"))?,
        "the area",
    )?;
    let occupied: Vec<Rect> = match field(fields, 5) {
        Some(list) => list
            .list()?
            .iter()
            .map(|rect| checked_rect(rect, "a neighbour"))
            .collect::<Res<_>>()?,
        None => Vec::new(),
    };
    if !width.is_finite() || !height.is_finite() {
        return Err(Fail::type_error(
            "a placement's size, step, area and neighbours must be finite numbers",
        ));
    }
    let spot = place_rect(mode, width, height, &area, &occupied, step);
    let placed = if flag_field(fields, 6, false)? {
        apart_from(&spot, &occupied, &area, step)
    } else {
        spot
    };
    Ok(rect_val(&placed))
}

/// Implements the interface-derived classes of the module that are not the built-in kinds.
pub(super) fn register(builder: &mut Builder) {
    let class = builder
        .class::<OwnedLayout>("floating_layout")
        .ctor(floating_ctor)
        .get("placement", |held, _| {
            Ok(floating_of(&held.layout).map_or(Val::None, |(mode, _)| mode_val(mode)))
        })
        .set("placement", |held, cx, value| {
            let mode = mode_of(&value)?;
            let (_, step) = floating_of(&held.layout).unwrap_or((PlaceMode::FreeSpace, DEFAULT_CASCADE_STEP));
            replace_floating(cx, held, mode, step);
            Ok(())
        })
        .get("cascade_step", |held, _| {
            Ok(floating_of(&held.layout).map_or(Val::None, |(_, step)| Val::Double(step)))
        })
        .set("cascade_step", |held, cx, value| {
            let step = step_of(value.double()?)?;
            let (mode, _) = floating_of(&held.layout).unwrap_or((PlaceMode::FreeSpace, DEFAULT_CASCADE_STEP));
            replace_floating(cx, held, mode, step);
            Ok(())
        })
        .function("place", |_, args| place(args));
    let _ = layout_members(class);
}

/// Gives the floating layout another mode or step. The workspace that uses it places the windows it has already as it did: the new
/// mode is for the windows that join after.
fn replace_floating(cx: &mut Cx<'_>, held: &mut OwnedLayout, mode: PlaceMode, step: f64) {
    let layout = LayoutObj::floating(mode, step);
    let ctx = cx.ctx();
    let old = std::mem::replace(&mut held.layout, Rc::clone(&layout));
    remember(ctx, &layout, cx.me());
    if let Some(workspace) = held.on {
        // The workspace gets the new layout and the old one is dropped outside every borrow.
        let _ = super::edit(ctx, |st| {
            let ws = st.workspace_mut(workspace)?;
            if Rc::ptr_eq(&ws.layout, &old) {
                let before = std::mem::replace(&mut ws.layout, layout);
                ws.revision = None;
                ws.dirty = true;
                st.graveyard.push(before);
            }
            Ok(())
        });
    }
    drop(old);
}
