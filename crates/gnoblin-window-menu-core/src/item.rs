//! Item ids and the named compositor actions they can name.
//!
//! An [`ItemId`] is the only thing a provider sends back to the compositor, so
//! it is the trust boundary of the menu. Its text is parsed once, at the edge,
//! into one of three shapes, and nothing downstream re-reads the string:
//!
//! - a compositor action, such as `window.close`, which [`MenuAction`] names;
//! - the group `window.move-to-workspace`, which is a submenu and never a choice;
//! - an extension item, `<extension>.<id>`, which routes to that extension.
//!
//! The `window.` namespace belongs to the compositor. An extension cannot name
//! it, so no extension item can ever be routed as an action. The grammar is
//! documented on [`ItemId::parse`].

use std::fmt;

/// The namespace the compositor owns. No extension may be named `window`.
const WINDOW_NAMESPACE: &str = "window";

/// The submenu that holds one item per workspace. It is a group, not an action.
const MOVE_TO_WORKSPACE_GROUP: &str = "window.move-to-workspace";

/// The prefix of a per-workspace action, followed by the 1-based number.
const MOVE_TO_WORKSPACE_PREFIX: &str = "window.move-to-workspace.";

/// A compositor-owned window action.
///
/// Each variant has exactly one id, given by [`MenuAction::id`]. The compositor, not
/// a provider, decides what each action does once the menu has closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuAction {
    /// Restore a maximised window to its previous size.
    Restore,
    /// Start a keyboard-driven move of the window.
    Move,
    /// Start a keyboard-driven resize of the window.
    Resize,
    /// Minimise the window.
    Minimize,
    /// Maximise the window.
    Maximize,
    /// Toggle whether the window stays above ordinary windows.
    ToggleAbove,
    /// Toggle whether the window is shown on every workspace.
    ToggleVisibleOnAllWorkspaces,
    /// Move the window to the workspace with this 1-based number.
    MoveToWorkspace(u32),
    /// Ask the window to close.
    Close,
}

impl MenuAction {
    /// The id this action is written as on the wire and in the menu.
    ///
    /// Stable: a provider stores these strings, so they must not change once
    /// shipped.
    #[must_use]
    pub fn id(self) -> ItemId {
        ItemId(match self {
            Self::Restore => "window.restore".to_owned(),
            Self::Move => "window.move".to_owned(),
            Self::Resize => "window.resize".to_owned(),
            Self::Minimize => "window.minimize".to_owned(),
            Self::Maximize => "window.maximize".to_owned(),
            Self::ToggleAbove => "window.toggle-above".to_owned(),
            Self::ToggleVisibleOnAllWorkspaces => "window.toggle-visible-on-all-workspaces".to_owned(),
            Self::MoveToWorkspace(number) => format!("{MOVE_TO_WORKSPACE_PREFIX}{number}"),
            Self::Close => "window.close".to_owned(),
        })
    }
}

/// The identifier of a menu item, validated when it is made.
///
/// Built only by [`ItemId::parse`], [`ItemId::action`] or [`ItemId::extension`],
/// so a value of this type is always well formed. Equality is textual.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(String);

/// The reason a string is not a valid [`ItemId`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemIdError {
    /// The string is empty.
    Empty,
    /// The string starts with `window.` but names no known action or group.
    UnknownWindowAction(String),
    /// The move-to-workspace number is not a canonical number from 1 upward.
    BadWorkspaceNumber(String),
    /// The extension part is empty, or uses characters outside `[a-z0-9-]`,
    /// or starts with a non-letter.
    BadExtension(String),
    /// The extension is named `window`, which the compositor owns.
    ReservedExtension,
    /// The local part is empty or uses characters outside `[a-z0-9._-]`.
    BadLocal(String),
    /// The id has no `.` separator, so it is neither an action nor an extension item.
    NoNamespace(String),
}

impl fmt::Display for ItemIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("item id is empty"),
            Self::UnknownWindowAction(id) => write!(f, "`{id}` is not a known window action"),
            Self::BadWorkspaceNumber(id) => write!(f, "`{id}` does not name a workspace from 1"),
            Self::BadExtension(id) => write!(f, "`{id}` has a bad extension name"),
            Self::ReservedExtension => f.write_str("`window` is reserved for compositor actions"),
            Self::BadLocal(id) => write!(f, "`{id}` has a bad local id"),
            Self::NoNamespace(id) => write!(f, "`{id}` has no namespace"),
        }
    }
}

impl std::error::Error for ItemIdError {}

/// What a validated [`ItemId`] names, once its namespace is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdKind<'a> {
    /// A compositor action, which [`ItemId::action`] returns.
    Action(MenuAction),
    /// A group of actions, such as the move-to-workspace submenu. Never a choice.
    Group,
    /// An extension item, with the extension name and the id it chose.
    Extension {
        /// The extension that contributed the item.
        extension: &'a str,
        /// The id the extension chose, without its namespace.
        local: &'a str,
    },
}

impl ItemId {
    /// Parses a textual id.
    ///
    /// The grammar, in the order it is tried:
    ///
    /// 1. A `window.`-prefixed id must be one of the named actions, the group
    ///    `window.move-to-workspace`, or `window.move-to-workspace.<n>`. Any other
    ///    `window.` id is refused, so a typo cannot fall through to an extension.
    /// 2. Any other id has the form `<extension>.<local>`. The extension is
    ///    `[a-z][a-z0-9-]*` and is split at the first `.`. The local part is
    ///    non-empty and uses `[a-z0-9._-]`.
    ///
    /// A workspace number must be canonical: `1`, `2`, and so on, with no sign,
    /// no leading zero and no zero.
    ///
    /// # Errors
    ///
    /// Returns an [`ItemIdError`] naming the first rule the text breaks.
    pub fn parse(text: &str) -> Result<Self, ItemIdError> {
        if text.is_empty() {
            return Err(ItemIdError::Empty);
        }
        if let Some(rest) = text.strip_prefix(WINDOW_NAMESPACE).and_then(|r| r.strip_prefix('.')) {
            return Self::parse_window(text, rest);
        }
        let Some((extension, local)) = text.split_once('.') else {
            return Err(ItemIdError::NoNamespace(text.to_owned()));
        };
        check_extension_name(extension)?;
        if !is_local_name(local) {
            return Err(ItemIdError::BadLocal(text.to_owned()));
        }
        Ok(Self(text.to_owned()))
    }

    /// Parses the part after `window.`, which the caller has already matched.
    fn parse_window(text: &str, rest: &str) -> Result<Self, ItemIdError> {
        let known = matches!(
            rest,
            "restore"
                | "move"
                | "resize"
                | "minimize"
                | "maximize"
                | "toggle-above"
                | "toggle-visible-on-all-workspaces"
                | "close"
                | "move-to-workspace"
        );
        if known {
            return Ok(Self(text.to_owned()));
        }
        if let Some(number) = rest.strip_prefix("move-to-workspace.") {
            return if is_canonical_workspace(number) {
                Ok(Self(text.to_owned()))
            } else {
                Err(ItemIdError::BadWorkspaceNumber(text.to_owned()))
            };
        }
        Err(ItemIdError::UnknownWindowAction(text.to_owned()))
    }

    /// The id of a compositor action.
    #[must_use]
    pub fn action(action: MenuAction) -> Self {
        action.id()
    }

    /// The id of an extension item, `<extension>.<local>`.
    ///
    /// # Errors
    ///
    /// Refuses a bad extension name, the reserved name `window`, or a bad local
    /// part, with the same rules as [`ItemId::parse`].
    pub fn extension(extension: &str, local: &str) -> Result<Self, ItemIdError> {
        check_extension_name(extension)?;
        if !is_local_name(local) {
            return Err(ItemIdError::BadLocal(local.to_owned()));
        }
        Ok(Self(format!("{extension}.{local}")))
    }

    /// The id of the move-to-workspace submenu.
    #[must_use]
    pub fn move_to_workspace_group() -> Self {
        Self(MOVE_TO_WORKSPACE_GROUP.to_owned())
    }

    /// The text of the id, exactly as a provider sends it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Says what the id names.
    ///
    /// Extension ids are only ever `<extension>.<local>` with a non-`window`
    /// extension, which [`ItemId::parse`] guarantees, so the split here cannot
    /// fail.
    #[must_use]
    pub fn kind(&self) -> IdKind<'_> {
        if let Some(action) = self.action_name() {
            return IdKind::Action(action);
        }
        if self.0.starts_with("window.") {
            // Parsing refuses every `window.` id except the group, so this is the group.
            return IdKind::Group;
        }
        let (extension, local) = self.0.split_once('.').unwrap_or((self.0.as_str(), ""));
        IdKind::Extension { extension, local }
    }

    /// The compositor action this id names. `None` for the group and for extension ids.
    #[must_use]
    pub fn action_name(&self) -> Option<MenuAction> {
        Some(match self.0.as_str() {
            "window.restore" => MenuAction::Restore,
            "window.move" => MenuAction::Move,
            "window.resize" => MenuAction::Resize,
            "window.minimize" => MenuAction::Minimize,
            "window.maximize" => MenuAction::Maximize,
            "window.toggle-above" => MenuAction::ToggleAbove,
            "window.toggle-visible-on-all-workspaces" => MenuAction::ToggleVisibleOnAllWorkspaces,
            "window.close" => MenuAction::Close,
            other => {
                let number = other.strip_prefix(MOVE_TO_WORKSPACE_PREFIX)?;
                MenuAction::MoveToWorkspace(number.parse().ok()?)
            }
        })
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Checks an extension name: `[a-z][a-z0-9-]*`, and not the reserved `window`.
///
/// # Errors
///
/// Returns [`ItemIdError::ReservedExtension`] for `window`, or
/// [`ItemIdError::BadExtension`] for any other malformed name.
pub fn check_extension_name(extension: &str) -> Result<(), ItemIdError> {
    if extension == WINDOW_NAMESPACE {
        return Err(ItemIdError::ReservedExtension);
    }
    if is_extension_name(extension) {
        Ok(())
    } else {
        Err(ItemIdError::BadExtension(extension.to_owned()))
    }
}

/// True for an extension name: a lowercase letter, then lowercase letters, digits or `-`.
fn is_extension_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// True for a local id: non-empty, from `[a-z0-9._-]`.
fn is_local_name(local: &str) -> bool {
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// True for a canonical workspace number: digits only, no leading zero, not zero.
fn is_canonical_workspace(number: &str) -> bool {
    number.parse::<u32>().is_ok_and(|n| n >= 1 && n.to_string() == number)
}
