//! `menu_entry`: what a person sees in the menu and what runs.
//!
//! A shell makes entries as objects and hands them to the menu through a `menu_provider`; the card reads each one into an [`Entry`] when
//! it opens (its fields as they are then), and a submenu is read when it opens. The stock entries are made natively and shown the same
//! way: their `run` is an [`MenuAction`] the library does itself, not a closure.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Closure, ErrorCode, Fail, Obj, Res, Val};
use std::rc::Rc;

/// What a stock entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    Maximise,
    Restore,
    Minimise,
    MoveWithKeys,
    ResizeWithKeys,
    ToggleAbove,
    Close,
    /// Moves the window to the workspace with this id.
    MoveToWorkspace(String),
}

/// What an entry runs after the menu closes.
#[derive(Clone)]
pub(crate) enum EntryRun {
    None,
    Closure(Closure),
    MenuAction(MenuAction),
}

/// What an entry opens beside it instead of running.
#[derive(Clone)]
pub(crate) enum Sub {
    None,
    /// The entries, read when the parent card opened.
    List(Vec<Entry>),
    /// Entries a shell's function answers when the submenu opens.
    Source(Closure),
    /// The stock "Move to workspace" list: one entry for each workspace.
    Workspaces,
}

/// An entry as the card holds it.
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) label: String,
    pub(crate) enabled: bool,
    pub(crate) checked: bool,
    pub(crate) separator: bool,
    pub(crate) run: EntryRun,
    pub(crate) sub: Sub,
}

impl Entry {
    /// A plain entry that runs an action of the library.
    pub(crate) fn of(label: &str, run: MenuAction) -> Entry {
        Entry {
            label: label.to_owned(),
            enabled: true,
            checked: false,
            separator: false,
            run: EntryRun::MenuAction(run),
            sub: Sub::None,
        }
    }

    /// A gap between groups of entries.
    pub(crate) fn gap() -> Entry {
        Entry {
            label: String::new(),
            enabled: true,
            checked: false,
            separator: true,
            run: EntryRun::None,
            sub: Sub::None,
        }
    }

    /// Whether the entry opens a submenu: it has one, even an empty one.
    pub(crate) fn opens(&self) -> bool {
        !matches!(self.sub, Sub::None)
    }
}

/// The state of a `menu_entry` object.
pub(crate) struct EntryObj {
    pub(crate) entry: Entry,
    /// The entries of `submenu`, as the shell gave them.
    submenu: Option<Vec<Obj>>,
}

impl EntryObj {
    pub(crate) fn from(entry: Entry) -> EntryObj {
        EntryObj { entry, submenu: None }
    }
}

/// How deep a submenu may be before the card refuses it: an entry that holds itself would never end.
const DEPTH: usize = 8;

/// Reads the entry of the object `obj`, and the submenus under it.
///
/// # Errors
///
/// The object is not an entry or is gone, or its submenus nest deeper than a card shows.
pub(crate) fn read(cx: &Cx<'_>, obj: Obj) -> Res<Entry> {
    read_at(cx, obj, 0)
}

fn read_at(cx: &Cx<'_>, obj: Obj, depth: usize) -> Res<Entry> {
    if depth > DEPTH {
        return Err(Fail::refused(
            ErrorCode::InvalidArgument,
            "a menu entry holds itself in its submenu",
        ));
    }
    let (mut entry, submenu) = cx.with_state(obj, |held: &mut EntryObj| (held.entry.clone(), held.submenu.clone()))?;
    if let Some(items) = submenu {
        let mut list = Vec::with_capacity(items.len());
        for item in items {
            list.push(read_at(cx, item, depth + 1)?);
        }
        entry.sub = Sub::List(list);
    }
    Ok(entry)
}

/// Reads every entry of a list of objects.
///
/// # Errors
///
/// A value is not an entry.
pub(crate) fn read_all(cx: &Cx<'_>, list: &Val) -> Res<Vec<Entry>> {
    list.list()?.iter().map(|value| read(cx, value.object()?)).collect()
}

/// Lets the entries a shell answered with end like any object once the library has read them: the class is retained so that an answer
/// survives the shell letting go of its handles as it answers, and an entry nobody holds then ends here.
pub(crate) fn settle(cx: &Cx<'_>, list: &Val) {
    for value in list.list().into_iter().flatten() {
        if let Ok(obj) = value.object() {
            cx.unpin(obj);
        }
    }
}

/// Makes an object of each entry, for the shell to hold: the library's stock entries as a provider adds to them.
///
/// # Errors
///
/// An object cannot be made.
pub(crate) fn hand_out(cx: &Cx<'_>, entries: Vec<Entry>) -> Res<Val> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let obj = cx.spawn("menu_entry", EntryObj::from(entry))?;
        out.push(cx.handout(obj)?);
        // The shell holds it now; it ends when the shell lets go.
        cx.unpin(obj);
    }
    Ok(Val::List(out))
}

pub(crate) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<EntryObj>("menu_entry")
        .ctor(|_cx, inits| {
            let mut entry = Entry {
                label: String::new(),
                enabled: true,
                checked: false,
                separator: false,
                run: EntryRun::None,
                sub: Sub::None,
            };
            if let Some(label) = inits.take("label") {
                entry.label = label.text()?.to_owned();
            }
            Ok(EntryObj::from(entry))
        })
        .get("label", |held, _| Ok(Val::Text(held.entry.label.clone())))
        .set("label", |held, _, value| {
            value.text()?.clone_into(&mut held.entry.label);
            Ok(())
        })
        .get("enabled", |held, _| Ok(Val::Bool(held.entry.enabled)))
        .set("enabled", |held, _, value| {
            held.entry.enabled = value.bool()?;
            Ok(())
        })
        .get("checked", |held, _| Ok(Val::Bool(held.entry.checked)))
        .set("checked", |held, _, value| {
            held.entry.checked = value.bool()?;
            Ok(())
        })
        .get("separator", |held, _| Ok(Val::Bool(held.entry.separator)))
        .set("separator", |held, _, value| {
            held.entry.separator = value.bool()?;
            Ok(())
        })
        .get("run", |held, _| {
            Ok(match &held.entry.run {
                EntryRun::Closure(closure) => Val::Closure(Rc::clone(closure)),
                _ => Val::None,
            })
        })
        .set("run", |held, _, value| {
            held.entry.run = if value.is_none() {
                EntryRun::None
            } else {
                EntryRun::Closure(Rc::clone(value.closure()?))
            };
            Ok(())
        })
        .get("submenu", |held, cx| {
            let Some(items) = &held.submenu else {
                return Ok(Val::None);
            };
            items
                .iter()
                .map(|item| cx.handout(*item))
                .collect::<Res<Vec<_>>>()
                .map(Val::List)
        })
        .set("submenu", |held, cx, value| {
            if value.is_none() {
                held.submenu = None;
                if matches!(held.entry.sub, Sub::List(_)) {
                    held.entry.sub = Sub::None;
                }
                return Ok(());
            }
            let me = cx.me();
            let mut items = Vec::new();
            for item in value.list()? {
                let obj = item.object()?;
                if obj == me {
                    return Err(Fail::refused(
                        ErrorCode::InvalidArgument,
                        "a menu entry cannot be its own submenu",
                    ));
                }
                cx.set_parent(obj, me)?;
                items.push(obj);
            }
            held.submenu = Some(items);
            Ok(())
        })
        .get("submenu_entries", |held, _| {
            Ok(match &held.entry.sub {
                Sub::Source(closure) => Val::Closure(Rc::clone(closure)),
                _ => Val::None,
            })
        })
        .set("submenu_entries", |held, _, value| {
            if value.is_none() {
                if matches!(held.entry.sub, Sub::Source(_)) {
                    held.entry.sub = Sub::None;
                }
            } else {
                held.entry.sub = Sub::Source(Rc::clone(value.closure()?));
            }
            Ok(())
        });
}
