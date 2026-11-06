//! The window list and the screen size the adapter reports, read from the object graph.
//!
//! `org.gnome.Shell.Introspect`'s `GetWindows` is a dictionary of windows keyed by identity,
//! and the portal's window picker is its consumer: it needs a title, an application id and a
//! size for every window, and it asks again when `WindowsChanged` says the list moved. This
//! module turns the graph's `window` objects into exactly that, and the enabled `output`
//! objects into `ScreenSize`.
//!
//! Reading the outputs here rather than only the windows is deliberate: the screen's size is
//! the union of the logical monitors, not the bounding box of the windows on it, and the
//! outputs are the same objects `org.gnome.Mutter.DisplayConfig` reports, so two adapters
//! cannot disagree about the desktop's extent.
//!
//! # Failure modes first
//!
//! 1. **Two windows claim one identity.** Refused rather than merged, because the dictionary
//!    key is what a caller uses to act on a window: [`IntrospectState::read`].
//! 2. **A window property is missing or mistyped.** Refused with the window and property
//!    named, rather than a blank title in a picker: [`WindowInfo::read`].
//! 3. **Nothing is enabled.** The screen size is `None`, and the property refuses rather
//!    than reporting `(0, 0)`, which a caller would read as a zero-sized desktop:
//!    [`IntrospectState::screen_size`].

use gnoblin_compat_core::LockState;
use gnoblin_model::{ObjectView, TypeKey, Value};

use crate::error::IntrospectError;

/// The object type this adapter reads for windows.
pub const WINDOW: &str = "window";

/// The object type it reads for the screen's extent.
pub const OUTPUT: &str = "output";

/// The lock object's type.
pub const LOCK: &str = "lock";

/// One window, as the interface's dictionary describes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WindowInfo {
    /// The compositor's identity for the window.
    pub id: u64,
    /// The title the client set.
    pub title: String,
    /// The Wayland app id the client set.
    pub app_id: String,
    /// Whether the window holds keyboard focus.
    pub focused: bool,
    /// The frame width.
    pub width: u32,
    /// The frame height.
    pub height: u32,
}

impl WindowInfo {
    /// Read one `window` object.
    ///
    /// # Errors
    ///
    /// [`IntrospectError::Unreadable`] for a property that is absent or is not the value the
    /// schema declares.
    fn read(view: &ObjectView) -> Result<WindowInfo, IntrospectError> {
        Ok(WindowInfo {
            id: uint(view, "id")?,
            title: text(view, "title")?,
            app_id: text(view, "app_id")?,
            focused: boolean(view, "focused")?,
            width: uint(view, "width")? as u32,
            height: uint(view, "height")? as u32,
        })
    }

    /// The application the window belongs to, which is what
    /// `RunningApplicationsChanged` tracks.
    pub fn application(&self) -> &str {
        &self.app_id
    }
}

/// One enabled logical monitor's extent, which is what `ScreenSize` is the union of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Extent {
    right: i32,
    bottom: i32,
}

/// Every window the graph publishes, and the screen's size.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct IntrospectState {
    /// The windows, sorted by identity so a report and a caller agree on order.
    pub windows: Vec<WindowInfo>,
    /// The screen's size, or `None` when no output is enabled.
    pub screen: Option<(i32, i32)>,
    /// The gate's embargo value.
    pub lock: LockState,
}

impl IntrospectState {
    /// Read the state from every object a subscription delivered.
    ///
    /// # Errors
    ///
    /// [`IntrospectError::DuplicateWindow`] when two windows claim one identity, and the
    /// property refusals of [`WindowInfo::read`] and [`IntrospectError::Unreadable`] for an
    /// output that cannot be measured.
    pub fn read<'a>(views: impl IntoIterator<Item = &'a ObjectView>) -> Result<IntrospectState, IntrospectError> {
        let mut windows: Vec<WindowInfo> = Vec::new();
        let mut extent: Option<Extent> = None;
        let mut locked = None;
        for view in views {
            if view.ty() == &TypeKey::gnoblin(WINDOW) {
                let window = WindowInfo::read(view)?;
                if windows.iter().any(|other| other.id == window.id) {
                    return Err(IntrospectError::DuplicateWindow { id: window.id });
                }
                windows.push(window);
            }
            if view.ty() == &TypeKey::gnoblin(OUTPUT) && boolean(view, "enabled")? {
                let right = integer(view, "x")?.saturating_add(uint(view, "width")? as i32);
                let bottom = integer(view, "y")?.saturating_add(uint(view, "height")? as i32);
                extent = Some(match extent {
                    None => Extent { right, bottom },
                    Some(previous) => Extent {
                        right: previous.right.max(right),
                        bottom: previous.bottom.max(bottom),
                    },
                });
            }
            if view.ty() == &TypeKey::gnoblin(LOCK) {
                locked = Some(boolean(view, "locked")?);
            }
        }
        windows.sort_by_key(|window| window.id);
        let lock = match locked {
            Some(true) => LockState::Locked,
            Some(false) => LockState::Unlocked,
            None => {
                return Err(IntrospectError::Unreadable {
                    object: "lock".to_owned(),
                    property: "locked".to_owned(),
                    reason: "the graph publishes no `lock` object".to_owned(),
                });
            }
        };
        Ok(IntrospectState {
            windows,
            screen: extent.map(|extent| (extent.right, extent.bottom)),
            lock,
        })
    }

    /// The screen's size.
    ///
    /// # Errors
    ///
    /// [`IntrospectError::NoScreen`] when no output is enabled: a zero-sized desktop is a
    /// worse answer than a refusal, because a caller lays a picker out from this number.
    pub fn screen_size(&self) -> Result<(i32, i32), IntrospectError> {
        self.screen.ok_or(IntrospectError::NoScreen)
    }

    /// The identities in the list, which is what `WindowsChanged` tracks.
    pub fn identities(&self) -> Vec<u64> {
        self.windows.iter().map(|window| window.id).collect()
    }

    /// The applications in the list, which is what `RunningApplicationsChanged` tracks.
    pub fn applications(&self) -> Vec<&str> {
        let mut applications: Vec<&str> = self.windows.iter().map(WindowInfo::application).collect();
        applications.sort_unstable();
        applications.dedup();
        applications
    }
}

fn unreadable(object: &str, property: &str, reason: &str) -> IntrospectError {
    IntrospectError::Unreadable {
        object: object.to_owned(),
        property: property.to_owned(),
        reason: reason.to_owned(),
    }
}

/// The object's name, which every error message uses.
fn object_name(view: &ObjectView) -> String {
    match view.property("title") {
        Some(Value::Text(title)) => format!("window `{title}`"),
        _ => format!("object#{}", view.id().get()),
    }
}

fn text(view: &ObjectView, property: &str) -> Result<String, IntrospectError> {
    match view.property(property) {
        Some(Value::Text(value)) => Ok(value.clone()),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected a string, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

fn uint(view: &ObjectView, property: &str) -> Result<u64, IntrospectError> {
    match view.property(property) {
        Some(Value::Uint(value)) => Ok(*value),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected a uint, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

fn integer(view: &ObjectView, property: &str) -> Result<i32, IntrospectError> {
    match view.property(property) {
        Some(Value::Int(value)) => i32::try_from(*value)
            .map_err(|_| unreadable(&object_name(view), property, &format!("{value} does not fit an int"))),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected an int, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}

fn boolean(view: &ObjectView, property: &str) -> Result<bool, IntrospectError> {
    match view.property(property) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(unreadable(
            &object_name(view),
            property,
            &format!("expected a bool, found {}", other.kind()),
        )),
        None => Err(unreadable(&object_name(view), property, "the property is absent")),
    }
}
