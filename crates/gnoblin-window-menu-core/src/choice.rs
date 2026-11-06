//! Checks a provider's reply against the menu before anything runs.
//!
//! A provider sends back one [`ItemId`]. That id is untrusted, so the
//! compositor checks it against the menu it sent out. The check has three
//! outcomes: a compositor action, an extension item, or a refusal. An extension
//! item never becomes a compositor action, and a compositor action is never
//! reached by a provider naming an id that is not in the menu.

use crate::item::{ItemId, MenuAction};
use crate::menu::{ItemKind, Menu, MenuItem};

/// Where a valid choice goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuRouted {
    /// A compositor action. The compositor runs it after the menu closes.
    Action(MenuAction),
    /// An extension item. The compositor sends the choice to that extension.
    Extension {
        /// The extension that contributed the item.
        extension: String,
        /// The id the extension chose, without its namespace.
        local: String,
    },
}

/// Why a choice is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuChoiceError {
    /// The id is not in the menu. A provider sent an id it was never offered.
    NotInMenu(String),
    /// The id is in the menu but disabled. The user cannot choose it.
    Disabled(String),
    /// The id names a submenu. A submenu opens; it is not chosen.
    IsSubmenu(String),
}

impl std::fmt::Display for MenuChoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInMenu(id) => write!(f, "`{id}` is not in the menu"),
            Self::Disabled(id) => write!(f, "`{id}` is disabled"),
            Self::IsSubmenu(id) => write!(f, "`{id}` is a submenu, not a choice"),
        }
    }
}

impl std::error::Error for MenuChoiceError {}

/// Checks that `id` is an enabled action in `menu`, and says where it goes.
///
/// The id must appear in the menu, anywhere including inside submenus, and its
/// item must be enabled and of kind [`ItemKind::Action`]. Each id appears once
/// in a menu built by [`crate::build_menu`], so the first match decides.
///
/// # Errors
///
/// Returns [`MenuChoiceError::NotInMenu`] for an unknown id, [`MenuChoiceError::Disabled`]
/// for a disabled one, and [`MenuChoiceError::IsSubmenu`] for a group.
pub fn validate_choice(menu: &Menu, id: &ItemId) -> Result<MenuRouted, MenuChoiceError> {
    let item = find(&menu.items, id).ok_or_else(|| MenuChoiceError::NotInMenu(id.as_str().to_owned()))?;
    match item.kind {
        ItemKind::Submenu => return Err(MenuChoiceError::IsSubmenu(id.as_str().to_owned())),
        ItemKind::Separator => return Err(MenuChoiceError::NotInMenu(id.as_str().to_owned())),
        ItemKind::Action => {}
    }
    if !item.enabled {
        return Err(MenuChoiceError::Disabled(id.as_str().to_owned()));
    }
    route(id)
}

/// Says where an id goes once it is known to be in the menu and enabled.
///
/// Routing depends only on the id, so an extension id can never reach an
/// action. The group id is refused here because it is not a choice.
fn route(id: &ItemId) -> Result<MenuRouted, MenuChoiceError> {
    if let Some(action) = id.action_name() {
        return Ok(MenuRouted::Action(action));
    }
    match id.kind() {
        crate::item::IdKind::Extension { extension, local } => Ok(MenuRouted::Extension {
            extension: extension.to_owned(),
            local: local.to_owned(),
        }),
        // Parsing already refused every other window id, so only the group is left.
        crate::item::IdKind::Group | crate::item::IdKind::Action(_) => {
            Err(MenuChoiceError::IsSubmenu(id.as_str().to_owned()))
        }
    }
}

/// Finds the first item with `id`, searching submenus depth first.
fn find<'a>(items: &'a [MenuItem], id: &ItemId) -> Option<&'a MenuItem> {
    items.iter().find_map(|item| {
        if item.id.as_ref() == Some(id) {
            return Some(item);
        }
        find(&item.children, id)
    })
}
