//! The window menu as data, and the rules that build it from window state.
//!
//! The compositor builds a [`Menu`] for one window and hands it to a provider.
//! The menu says what the window can do. Items the window cannot do are
//! present but disabled, so the user can see them. Two items are different:
//! Restore appears only when the window is maximised, and the
//! move-to-workspace submenu appears only with more than one workspace. Those
//! two would be meaningless otherwise, so they are left out.
//!
//! Extension items come after the built-in items, separated by a rule. An
//! extension may only add items whose ids are in its own namespace, which
//! [`Contribution::new`] checks.

use crate::item::{Action, IdKind, ItemId, ItemIdError, check_extension_name};

/// The label of each built-in action, in English.
///
/// Labels are presentation, not identity: a provider may translate them, but
/// the id is what gets sent back.
const LABEL_RESTORE: &str = "Restore";
const LABEL_MOVE: &str = "Move";
const LABEL_RESIZE: &str = "Resize";
const LABEL_MINIMIZE: &str = "Minimize";
const LABEL_MAXIMIZE: &str = "Maximize";
const LABEL_TOGGLE_ABOVE: &str = "Always on top";
const LABEL_TOGGLE_ALL_WORKSPACES: &str = "Always on visible workspace";
const LABEL_MOVE_TO_WORKSPACE: &str = "Move to workspace";
const LABEL_CLOSE: &str = "Close";

/// What a window can do right now. Each flag decides whether an item is enabled.
///
/// Built by the compositor from the live window. The menu is only as current as
/// this value, so it must be rebuilt when the window changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    /// The window is maximised. Enables Restore and turns Maximize off.
    pub maximised: bool,
    /// The window is minimised. Turns Minimize off.
    pub minimised: bool,
    /// The window can be resized. Enables Resize; Maximize needs it too.
    pub resizable: bool,
    /// The window can be moved. Enables Move.
    pub movable: bool,
    /// The window can be closed. Enables Close.
    pub closable: bool,
    /// The window is kept above ordinary windows. Checks the toggle.
    pub always_on_top: bool,
    /// The window is shown on every workspace. Checks the toggle.
    pub on_all_workspaces: bool,
}

/// The workspaces the compositor has, and the one the window is on.
///
/// Workspace numbers are 1-based, as in the item ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Workspaces {
    /// The number of workspaces. At least one.
    count: u32,
    /// The workspace the window is on, from 1 to `count`.
    current: u32,
}

impl Workspaces {
    /// Makes a workspace set, or `None` when `count` is zero or `current` is
    /// outside `1..=count`.
    #[must_use]
    pub fn new(count: u32, current: u32) -> Option<Self> {
        (count >= 1 && (1..=count).contains(&current)).then_some(Self { count, current })
    }

    /// The number of workspaces.
    #[must_use]
    pub fn count(self) -> u32 {
        self.count
    }

    /// The workspace the window is on.
    #[must_use]
    pub fn current(self) -> u32 {
        self.current
    }
}

/// The kind of a menu item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A choice the user can make. Has an id.
    Action,
    /// A visual divider. Never chosen and never highlighted. Has no id.
    Separator,
    /// A group that opens to show `children`. Has an id but is never chosen.
    Submenu,
}

/// One entry in a menu.
///
/// Fields are public so a provider can read them. The constructors keep the
/// id rule: a separator has no id, and every other kind has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// The id sent back when the item is chosen. `None` only for separators.
    pub id: Option<ItemId>,
    /// The text shown to the user.
    pub label: String,
    /// The name of an icon, if any. Interpreted by the provider.
    pub icon: Option<String>,
    /// Whether the item can be chosen. A disabled item is shown but refused.
    pub enabled: bool,
    /// Whether the item is a toggle that is currently on.
    pub checked: bool,
    /// What the item is.
    pub kind: ItemKind,
    /// The items of a submenu. Empty for every other kind.
    pub children: Vec<MenuItem>,
    /// A hint for the shortcut that chooses this item, such as `Alt+F4`.
    /// Display only; it does not bind a key.
    pub accelerator: Option<String>,
}

impl MenuItem {
    /// An enabled, unchecked action with `id` and `label`.
    #[must_use]
    pub fn action(id: ItemId, label: impl Into<String>) -> Self {
        Self {
            id: Some(id),
            label: label.into(),
            icon: None,
            enabled: true,
            checked: false,
            kind: ItemKind::Action,
            children: Vec::new(),
            accelerator: None,
        }
    }

    /// A divider between groups of items.
    #[must_use]
    pub fn separator() -> Self {
        Self {
            id: None,
            label: String::new(),
            icon: None,
            enabled: false,
            checked: false,
            kind: ItemKind::Separator,
            children: Vec::new(),
            accelerator: None,
        }
    }

    /// An enabled submenu with `id`, `label` and `children`.
    #[must_use]
    pub fn submenu(id: ItemId, label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self {
            id: Some(id),
            label: label.into(),
            icon: None,
            enabled: true,
            checked: false,
            kind: ItemKind::Submenu,
            children,
            accelerator: None,
        }
    }
}

/// A complete menu for one window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    /// The top-level items, in display order.
    pub items: Vec<MenuItem>,
}

/// A set of items an extension adds to the menu.
///
/// Built only by [`Contribution::new`], which checks that every id is in the
/// extension's namespace. An extension cannot add a compositor action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contribution {
    /// The extension the items belong to. Its name is the id namespace.
    extension: String,
    /// The items, checked against `extension`.
    items: Vec<MenuItem>,
}

/// The reason an extension's contribution is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContributionError {
    /// The extension name or an item id is malformed.
    BadId(ItemIdError),
    /// An item id is not in the extension's namespace.
    ForeignId {
        /// The extension that made the contribution.
        extension: String,
        /// The id that is outside its namespace.
        id: String,
    },
}

impl std::fmt::Display for ContributionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadId(err) => write!(f, "bad contributed id: {err}"),
            Self::ForeignId { extension, id } => {
                write!(f, "extension `{extension}` may not contribute `{id}`")
            }
        }
    }
}

impl std::error::Error for ContributionError {}

impl Contribution {
    /// Makes a contribution from `extension`, checking every item.
    ///
    /// # Errors
    ///
    /// Refuses a malformed extension name, an id outside `<extension>.`, or a
    /// submenu child that breaks either rule.
    pub fn new(extension: impl Into<String>, items: Vec<MenuItem>) -> Result<Self, ContributionError> {
        let extension = extension.into();
        check_extension_name(&extension).map_err(ContributionError::BadId)?;
        check_items(&extension, &items)?;
        Ok(Self { extension, items })
    }

    /// The extension that made the contribution.
    #[must_use]
    pub fn extension(&self) -> &str {
        &self.extension
    }
}

/// Checks that every id in `items`, and in their submenus, is in `extension`'s namespace.
fn check_items(extension: &str, items: &[MenuItem]) -> Result<(), ContributionError> {
    for item in items {
        if item.kind != ItemKind::Separator {
            let Some(id) = &item.id else {
                return Err(ContributionError::BadId(ItemIdError::Empty));
            };
            let owned = matches!(
                id.kind(),
                IdKind::Extension { extension: owner, .. } if owner == extension
            );
            if !owned {
                return Err(ContributionError::ForeignId {
                    extension: extension.to_owned(),
                    id: id.as_str().to_owned(),
                });
            }
        }
        check_items(extension, &item.children)?;
    }
    Ok(())
}

/// Builds the menu for a window.
///
/// The built-in items come first, in this order: Restore (only when maximised),
/// Move, Resize, Minimize, Maximize, Always on top, Always on visible workspace,
/// Move to workspace (only with more than one workspace), a separator, Close.
/// Then a separator and each contribution's items, in the order given.
///
/// Enablement follows `state`: Move needs `movable`, Resize needs `resizable`,
/// Maximize needs `resizable` and a window that is not already maximised,
/// Minimize needs a window that is not already minimised, and Close needs
/// `closable`. The two toggles are always enabled, and their check mark follows
/// the state. A workspace submenu item is disabled for the current workspace.
#[must_use]
pub fn build_menu(state: &WindowState, workspaces: &Workspaces, contributions: &[Contribution]) -> Menu {
    let mut items = Vec::new();

    if state.maximised {
        items.push(MenuItem::action(Action::Restore.id(), LABEL_RESTORE));
    }
    items.push(enabled(MenuItem::action(Action::Move.id(), LABEL_MOVE), state.movable));
    items.push(enabled(
        MenuItem::action(Action::Resize.id(), LABEL_RESIZE),
        state.resizable,
    ));
    items.push(enabled(
        MenuItem::action(Action::Minimize.id(), LABEL_MINIMIZE),
        !state.minimised,
    ));
    items.push(enabled(
        MenuItem::action(Action::Maximize.id(), LABEL_MAXIMIZE),
        state.resizable && !state.maximised,
    ));
    items.push(checked(
        MenuItem::action(Action::ToggleAbove.id(), LABEL_TOGGLE_ABOVE),
        state.always_on_top,
    ));
    items.push(checked(
        MenuItem::action(Action::ToggleVisibleOnAllWorkspaces.id(), LABEL_TOGGLE_ALL_WORKSPACES),
        state.on_all_workspaces,
    ));
    if workspaces.count() > 1 {
        items.push(move_to_workspace(workspaces));
    }
    items.push(MenuItem::separator());
    items.push(enabled(
        MenuItem::action(Action::Close.id(), LABEL_CLOSE),
        state.closable,
    ));

    for contribution in contributions {
        items.push(MenuItem::separator());
        items.extend(contribution.items.iter().cloned());
    }

    Menu { items }
}

/// The move-to-workspace submenu: one item per workspace, the current one disabled.
fn move_to_workspace(workspaces: &Workspaces) -> MenuItem {
    let children = (1..=workspaces.count())
        .map(|number| {
            let item = MenuItem::action(Action::MoveToWorkspace(number).id(), format!("Workspace {number}"));
            enabled(item, number != workspaces.current())
        })
        .collect();
    MenuItem::submenu(ItemId::move_to_workspace_group(), LABEL_MOVE_TO_WORKSPACE, children)
}

/// Sets `enabled` on an item and returns it.
fn enabled(mut item: MenuItem, value: bool) -> MenuItem {
    item.enabled = value;
    item
}

/// Sets `checked` on an item and returns it.
fn checked(mut item: MenuItem, value: bool) -> MenuItem {
    item.checked = value;
    item
}
