//! The window menu as pure data: its items and ids, the menu built from a
//! window's state, validation of a provider's choice, placement on the output,
//! keyboard navigation, and the contract a menu provider implements.
//!
//! The crate performs no I/O and holds no clock. The compositor owns the
//! window, the output, the sockets and the layer, and calls into this crate.
//! Each part is in its own module:
//!
//! - [`item`]: [`ItemId`] and the compositor's named [`MenuAction`]s. An id is
//!   the trust boundary, so it is parsed once, at the edge.
//! - [`menu`]: [`Menu`] and [`MenuItem`], the [`MenuWindowState`] it is built
//!   from, extension [`Contribution`]s, and [`build_menu`].
//! - [`choice`]: [`validate_choice`], which checks a provider's reply against
//!   the menu it was sent and says where it routes.
//! - [`place`]: [`place`], which chooses where an open menu goes.
//! - [`nav`]: [`Navigator`], the keyboard state machine, with its transition
//!   table in the module docs.
//! - [`provider`]: the [`MenuProvider`] trait, the dismissal reasons and the
//!   fallback rules from D29.
//!
//! The design is in `design/rust-compositor.md` (D29).

pub mod choice;
pub mod item;
pub mod menu;
pub mod nav;
pub mod place;
pub mod provider;

pub use choice::{MenuChoiceError, MenuRouted, validate_choice};
pub use item::{IdKind, ItemId, ItemIdError, MenuAction};
pub use menu::{
    Contribution, ContributionError, ItemKind, Menu, MenuItem, MenuWindowState, MenuWorkspaces, build_menu,
};
pub use nav::{MenuKey, NavOutcome, NavRefusal, NavState, Navigator};
pub use place::{MenuPlacement, place};
pub use provider::{
    DismissReason, MenuProvider, OpenError, OpenRequest, ProviderEvent, ProviderSource, REPLY_TIMEOUT, reply_overdue,
    source_for,
};
