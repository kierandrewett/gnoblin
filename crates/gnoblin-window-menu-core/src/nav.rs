//! Keyboard navigation for an open menu, as an explicit state machine.
//!
//! [`Navigator`] owns a [`Menu`] and its [`NavState`]. The state is either
//! `Closed` or `Open`, and an open menu holds a `trail`, the chain of submenus
//! entered from the root, and a `highlight`, the index of the highlighted item
//! at the current level. Every key goes through [`Navigator::handle`].
//!
//! # Transition table
//!
//! `Open` means the menu is open at any depth. "Selectable" means an item that is
//! not a separator and is enabled. Highlight never rests on an item that is not
//! selectable.
//!
//! | State  | MenuKey            | Condition                | Result |
//! |--------|----------------|--------------------------|--------|
//! | Closed | any            | always                   | refuse `Closed` |
//! | Open   | Down           | a selectable item exists | highlight next selectable, wrapping |
//! | Open   | Up             | a selectable item exists | highlight previous selectable, wrapping |
//! | Open   | Home / End     | a selectable item exists | highlight first / last selectable |
//! | Open   | Down/Up/Home/End | none selectable        | refuse `NothingSelectable` |
//! | Open   | Enter / Space  | action, enabled          | close; `Chose` the id |
//! | Open   | Enter / Space  | submenu                  | enter it, as Right |
//! | Open   | Enter / Space  | nothing highlighted      | refuse `NothingHighlighted` |
//! | Open   | Right          | submenu highlighted      | enter it; highlight its first selectable |
//! | Open   | Right          | otherwise                | refuse `NotASubmenu` |
//! | Open   | Left           | inside a submenu         | leave it; highlight the submenu |
//! | Open   | Left           | at the root              | refuse `AtRoot` |
//! | Open   | Escape         | inside a submenu         | leave it, as Left |
//! | Open   | Escape         | at the root              | close; `Dismissed(Escape)` |
//!
//! Entering a submenu whose items are all disabled leaves nothing highlighted.
//! Escape closes one level at a time, so the user can back out without losing
//! the menu.

use crate::item::ItemId;
use crate::menu::{ItemKind, Menu, MenuItem};
use crate::provider::DismissReason;

/// A key the navigator understands. The compositor maps real key events onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    /// Move the highlight up, wrapping at the top.
    Up,
    /// Move the highlight down, wrapping at the bottom.
    Down,
    /// Highlight the first selectable item.
    Home,
    /// Highlight the last selectable item.
    End,
    /// Choose the highlighted item, or enter a highlighted submenu.
    Enter,
    /// The same as [`MenuKey::Enter`].
    Space,
    /// Enter the highlighted submenu.
    Right,
    /// Leave the current submenu.
    Left,
    /// Leave the current submenu, or close the menu at the root.
    Escape,
}

/// Where the navigator is. Changed only by [`Navigator`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavState {
    /// No menu is open. Keys are refused.
    Closed,
    /// A menu is open.
    Open {
        /// The indices of the submenus entered from the root, outermost first.
        /// Empty at the root.
        trail: Vec<usize>,
        /// The highlighted item at the current level, or `None` when no item is selectable.
        highlight: Option<usize>,
    },
}

/// What a handled key did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavOutcome {
    /// The highlight moved, or stayed where it was.
    Highlighted,
    /// A submenu was entered.
    SubmenuEntered,
    /// A submenu was left.
    SubmenuLeft,
    /// An enabled action was chosen. The menu is now closed.
    Chose(ItemId),
    /// The menu was dismissed at the root. The menu is now closed.
    Dismissed(DismissReason),
}

/// Why a key was refused. The state did not change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavRefusal {
    /// The key was pressed while no menu was open.
    Closed,
    /// [`Navigator::open`] was called while a menu was already open.
    AlreadyOpen,
    /// The level has no selectable item to move to.
    NothingSelectable,
    /// Enter or Space was pressed with nothing highlighted.
    NothingHighlighted,
    /// Right was pressed on something that is not a submenu.
    NotASubmenu,
    /// Left was pressed at the root, where there is no submenu to leave.
    AtRoot,
}

impl std::fmt::Display for NavRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Closed => "the menu is closed",
            Self::AlreadyOpen => "the menu is already open",
            Self::NothingSelectable => "no item can be selected here",
            Self::NothingHighlighted => "nothing is highlighted",
            Self::NotASubmenu => "the highlighted item is not a submenu",
            Self::AtRoot => "the menu is at its root",
        })
    }
}

impl std::error::Error for NavRefusal {}

/// Owns a menu and runs the keyboard state machine over it.
#[derive(Clone, Debug)]
pub struct Navigator {
    /// The menu being navigated. Fixed for the life of one opening.
    menu: Menu,
    /// The current state.
    state: NavState,
}

impl Navigator {
    /// A navigator over `menu`, starting closed.
    #[must_use]
    pub fn new(menu: Menu) -> Self {
        Self {
            menu,
            state: NavState::Closed,
        }
    }

    /// The menu being navigated.
    #[must_use]
    pub fn menu(&self) -> &Menu {
        &self.menu
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> &NavState {
        &self.state
    }

    /// Opens the menu at the root.
    ///
    /// With `select_first`, the first selectable item is highlighted, as a
    /// keyboard invocation does. Without it, nothing is highlighted, as a
    /// pointer invocation does.
    ///
    /// # Errors
    ///
    /// Refuses with [`NavRefusal::AlreadyOpen`] when the menu is open.
    pub fn open(&mut self, select_first: bool) -> Result<(), NavRefusal> {
        if self.state != NavState::Closed {
            return Err(NavRefusal::AlreadyOpen);
        }
        let highlight = if select_first {
            first_selectable(&self.menu.items)
        } else {
            None
        };
        self.state = open(&[], highlight);
        Ok(())
    }

    /// Closes the menu without choosing anything. The state becomes `Closed`.
    pub fn close(&mut self) {
        self.state = NavState::Closed;
    }

    /// Handles one key, following the transition table in the module docs.
    ///
    /// # Errors
    ///
    /// Returns a [`NavRefusal`] for a key that cannot act in the current state. The
    /// state is unchanged in that case.
    pub fn handle(&mut self, key: MenuKey) -> Result<NavOutcome, NavRefusal> {
        let NavState::Open { trail, highlight } = &self.state else {
            return Err(NavRefusal::Closed);
        };
        let items = level(&self.menu.items, trail);
        let (next, outcome) = match key {
            MenuKey::Down => move_highlight(trail, *highlight, items, true)?,
            MenuKey::Up => move_highlight(trail, *highlight, items, false)?,
            MenuKey::Home => jump(trail, first_selectable(items))?,
            MenuKey::End => jump(trail, last_selectable(items))?,
            MenuKey::Enter | MenuKey::Space => activate(trail, *highlight, items)?,
            MenuKey::Right => enter_submenu(trail, *highlight, items)?,
            MenuKey::Left => leave_submenu(trail)?,
            MenuKey::Escape if trail.is_empty() => (NavState::Closed, NavOutcome::Dismissed(DismissReason::Escape)),
            MenuKey::Escape => leave_submenu(trail)?,
        };
        self.state = next;
        Ok(outcome)
    }
}

/// Moves the highlight one selectable step, wrapping.
fn move_highlight(
    trail: &[usize],
    highlight: Option<usize>,
    items: &[MenuItem],
    forward: bool,
) -> Result<(NavState, NavOutcome), NavRefusal> {
    let next = step(items, highlight, forward).ok_or(NavRefusal::NothingSelectable)?;
    Ok((open(trail, Some(next)), NavOutcome::Highlighted))
}

/// Highlights a known item, or refuses when there is none.
fn jump(trail: &[usize], target: Option<usize>) -> Result<(NavState, NavOutcome), NavRefusal> {
    let index = target.ok_or(NavRefusal::NothingSelectable)?;
    Ok((open(trail, Some(index)), NavOutcome::Highlighted))
}

/// Chooses an enabled action, or enters a submenu.
fn activate(
    trail: &[usize],
    highlight: Option<usize>,
    items: &[MenuItem],
) -> Result<(NavState, NavOutcome), NavRefusal> {
    let index = highlight.ok_or(NavRefusal::NothingHighlighted)?;
    let item = items.get(index).ok_or(NavRefusal::NothingHighlighted)?;
    match item.kind {
        ItemKind::Action => {
            let id = item.id.clone().ok_or(NavRefusal::NothingHighlighted)?;
            Ok((NavState::Closed, NavOutcome::Chose(id)))
        }
        ItemKind::Submenu => enter_submenu(trail, highlight, items),
        ItemKind::Separator => Err(NavRefusal::NothingHighlighted),
    }
}

/// Enters the highlighted submenu, highlighting its first selectable child.
fn enter_submenu(
    trail: &[usize],
    highlight: Option<usize>,
    items: &[MenuItem],
) -> Result<(NavState, NavOutcome), NavRefusal> {
    let index = highlight.ok_or(NavRefusal::NotASubmenu)?;
    let item = items.get(index).ok_or(NavRefusal::NotASubmenu)?;
    if item.kind != ItemKind::Submenu {
        return Err(NavRefusal::NotASubmenu);
    }
    let mut deeper = trail.to_vec();
    deeper.push(index);
    Ok((
        open(&deeper, first_selectable(&item.children)),
        NavOutcome::SubmenuEntered,
    ))
}

/// Leaves the current submenu, highlighting it in the parent level.
fn leave_submenu(trail: &[usize]) -> Result<(NavState, NavOutcome), NavRefusal> {
    let (&left, parent) = trail.split_last().ok_or(NavRefusal::AtRoot)?;
    Ok((open(parent, Some(left)), NavOutcome::SubmenuLeft))
}

/// An open state at `trail` with `highlight`.
fn open(trail: &[usize], highlight: Option<usize>) -> NavState {
    NavState::Open {
        trail: trail.to_vec(),
        highlight,
    }
}

/// True when the item can be highlighted: not a separator, and enabled.
fn selectable(item: &MenuItem) -> bool {
    item.kind != ItemKind::Separator && item.enabled
}

/// The first selectable index in `items`.
fn first_selectable(items: &[MenuItem]) -> Option<usize> {
    items.iter().position(selectable)
}

/// The last selectable index in `items`.
fn last_selectable(items: &[MenuItem]) -> Option<usize> {
    items.iter().rposition(selectable)
}

/// The next selectable index after `from` in the given direction, wrapping.
///
/// With no `from`, forward gives the first selectable item and backward gives
/// the last. Returns `None` when nothing is selectable.
fn step(items: &[MenuItem], from: Option<usize>, forward: bool) -> Option<usize> {
    let len = items.len();
    if len == 0 {
        return None;
    }
    let Some(start) = from else {
        return if forward {
            first_selectable(items)
        } else {
            last_selectable(items)
        };
    };
    (1..=len).find_map(|offset| {
        let index = if forward {
            (start + offset) % len
        } else {
            (start + len - (offset % len)) % len
        };
        selectable(&items[index]).then_some(index)
    })
}

/// The items at the level reached by following `trail` from the root.
///
/// Every index in a trail was validated when it was entered, so a missing one
/// is a broken invariant. It yields an empty slice rather than panicking, so
/// the navigator refuses instead of crashing.
fn level<'a>(root: &'a [MenuItem], trail: &[usize]) -> &'a [MenuItem] {
    trail.iter().fold(root, |items, &index| match items.get(index) {
        Some(item) => item.children.as_slice(),
        None => &[],
    })
}
