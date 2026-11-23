//! The built-in layouts as classes: `master_stack_layout`, `grid_layout`, `monocle_layout`, `dwindle_layout`, `strip_layout`,
//! `arc_layout` and `carousel_layout`.
//!
//! The classes are written from `gnoblin_std_abi::layouts::LAYOUT_KINDS` (`crates/gnoblin-sdkgen/src/layout_schema.rs`), and so is
//! their implementation: this module walks the same table. An option is a property, checked against the table's type and bound with the
//! words the C table's functions use; an action is a method, or, when it only reads something that may be absent, a read-only property.
//! An action works on the workspace that has the layout, because a layout that holds a tree of windows is about one workspace's windows
//! (a layout that no workspace has refuses it).

use std::rc::Rc;

use gnoblin_core::Rect;
use gnoblin_object::server::{Builder, ClassBuilder, Cx, Inits};
use gnoblin_object::{ErrorCode, Fail, Res, Val};
use gnoblin_std_abi::layouts::{ActionDesc, LAYOUT_KINDS, LayoutKindDesc, OptionDesc, ValueDesc, ValueShape};

use super::layouts::{OwnedLayout, layout_members};
use super::values::{client_of, client_val};
use super::{edit, look};
use crate::builtin::{ActionEnv, Builtin, OptionSet, Val as Value, decode_value, encode_value};
use crate::floating::grow;
use crate::layout::LayoutObj;
use crate::nodes;
use crate::object::record::enumeration;
use crate::state::Id;

/// An option or an answer of the table as the value of a property or a result.
fn object_val(desc: &ValueDesc, value: &Value) -> Val {
    match (value, desc.ty) {
        (Value::None, _) => Val::None,
        (Value::Bool(flag), _) => Val::Bool(*flag),
        (Value::Num(number), ValueShape::Enum(_)) => enumeration(*number as u32 + 1),
        (Value::Num(number), ValueShape::Int) => Val::Int(*number as i64),
        (Value::Num(number), _) => Val::Double(*number),
        (Value::Node(node), _) => client_val(*node),
        (Value::Text(text), _) => Val::Text(text.clone()),
    }
}

/// A value a shell gave as the table's value, before the table checks it.
fn table_val(desc: &ValueDesc, value: &Val) -> Res<Value> {
    if value.is_none() {
        return Ok(Value::None);
    }
    Ok(match desc.ty {
        ValueShape::Bool => Value::Bool(value.bool()?),
        ValueShape::Number => Value::Num(value.double()?),
        ValueShape::Int => Value::Num(value.int()? as f64),
        ValueShape::Enum(_) => Value::Num(f64::from(value.enum_value()?.wrapping_sub(1))),
        ValueShape::Node => Value::Node(client_of(value)?),
        ValueShape::Text => Value::Text(value.text()?.to_owned()),
    })
}

/// A value of the option, argument or answer `name` of `owner`, checked against its declaration as the C table's functions check it:
/// a number in range, a member of its enumeration, present when it has to be.
fn checked(owner: &str, name: &str, desc: &ValueDesc, value: &Val) -> Res<Value> {
    let raw = encode_value(&table_val(desc, value)?, Some(desc));
    decode_value(owner, name, desc, &raw).map_err(Fail::from)
}

/// The built-in layout of `held`.
fn builtin_of(held: &OwnedLayout) -> &Builtin {
    held.layout
        .as_builtin()
        .unwrap_or_else(|| unreachable!("an object of a built-in class holds a built-in layout"))
}

/// What an action needs to know about `workspace`, as the table's `workspace_layout_act` gathers it.
fn action_env(st: &crate::state::State, workspace: Id) -> crate::error::Res<ActionEnv> {
    let clients = st.workspace(workspace)?.clients.clone();
    let focused = clients
        .iter()
        .copied()
        .find(|client| nodes::flag(&st.g, *client, "focused"));
    let focused_size = focused.and_then(|client| {
        let own = nodes::rect_of(&st.g, client);
        if !(own.width > 0.0 && own.height > 0.0) {
            return None;
        }
        let outer = grow(&Rect::new(0.0, 0.0, own.width, own.height), &st.inset_of(client));
        Some((outer.width, outer.height))
    });
    Ok(ActionEnv {
        clients,
        focused,
        focused_size,
    })
}

/// Runs the action of the built-in layout `layout` on the workspace `workspace`: the arguments checked, the layout changed, the
/// workspace asked to compute again, the keyboard handed on when the action says so.
fn act(
    cx: &Cx<'_>,
    kind: &'static LayoutKindDesc,
    action: &'static ActionDesc,
    held: &OwnedLayout,
    args: &[Val],
) -> Res<Value> {
    let owner = format!("{}.{}", kind.name, action.name);
    let mut values: Vec<Value> = Vec::with_capacity(action.args.len());
    for (desc, value) in action.args.iter().zip(args) {
        values.push(if value.is_none() && desc.value.optional {
            Value::None
        } else {
            checked(&owner, desc.name, &desc.value, value)?
        });
    }
    let Some(workspace) = held.on else {
        return Err(Fail::refused(
            ErrorCode::InvalidState,
            format!(
                "the {} layout acts on the workspace that has it: give it to a workspace first",
                kind.name
            ),
        ));
    };
    let env = look(cx.ctx(), |st| action_env(st, workspace))?;
    // The layout runs outside the library's borrow of its state.
    let builtin: &Builtin = builtin_of(held);
    let outcome = builtin.act(action.name, &values, &env).map_err(Fail::from)?;
    edit(cx.ctx(), |st| {
        if outcome.changed {
            st.workspace_mut(workspace)?.dirty = true;
        }
        if let Some(client) = outcome.focus {
            let _ = st.g.window_handle(gnoblin_core::NodeId(client)).focus();
        }
        Ok(())
    })?;
    Ok(outcome.value)
}

/// The class of a kind: its options, its actions and the members of the interface it extends.
fn register_kind<'a>(builder: &'a mut Builder, kind: &'static LayoutKindDesc) -> ClassBuilder<'a, OwnedLayout> {
    let mut class = builder
        .class::<OwnedLayout>(&format!("{}_layout", kind.name))
        .ctor(move |cx, inits| make(cx, kind, inits));
    for option in kind.options {
        class = class
            .get(option.name, move |held, _| {
                Ok(object_val(&option.value, &builtin_of(held).option(option.name)))
            })
            .set(option.name, move |held, cx, value| {
                set_option(cx, kind, option, held, &value)
            });
    }
    for action in kind.actions {
        class = if action.is_reading() {
            let returns = action.returns.unwrap_or_else(|| unreachable!("a reading answers"));
            class.get(action.name, move |held, cx| {
                act(cx, kind, action, held, &[]).map(|answer| object_val(&returns, &answer))
            })
        } else {
            class.method(action.name, move |held, cx, args| {
                let answer = act(cx, kind, action, held, args)?;
                Ok(match &action.returns {
                    Some(returns) => object_val(returns, &answer),
                    None => Val::None,
                })
            })
        };
    }
    layout_members(class)
}

/// Makes the layout of `kind` with the options the shell gave: each checked, the rest at the table's defaults.
fn make(cx: &mut Cx<'_>, kind: &'static LayoutKindDesc, inits: &mut Inits) -> Res<OwnedLayout> {
    let mut options = OptionSet::of(kind.name, kind.options);
    for option in kind.options {
        if let Some(value) = inits.take(option.name) {
            options.put(option.name, checked(kind.name, option.name, &option.value, &value)?);
        }
    }
    let layout: Rc<LayoutObj> = LayoutObj::builtin(Builtin::new(kind, options));
    super::layouts::remember(cx.ctx(), &layout, cx.me());
    Ok(OwnedLayout { layout, on: None })
}

/// Sets an option of a layout: checked, and the workspace that has it asked to compute again.
fn set_option(
    cx: &mut Cx<'_>,
    kind: &'static LayoutKindDesc,
    option: &'static OptionDesc,
    held: &OwnedLayout,
    value: &Val,
) -> Res<()> {
    let checked = checked(kind.name, option.name, &option.value, value)?;
    builtin_of(held).set_option(option.name, checked);
    if let Some(workspace) = held.on {
        edit(cx.ctx(), |st| {
            st.workspace_mut(workspace)?.dirty = true;
            Ok(())
        })?;
    }
    Ok(())
}

/// Implements the classes of the built-in kinds.
pub(super) fn register(builder: &mut Builder) {
    for kind in LAYOUT_KINDS {
        let _ = register_kind(builder, kind);
    }
}
