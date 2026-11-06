//! The values a dialog screen takes in and reports out: the key a screen receives,
//! the result it settles with, the dialog it draws and a directory listing it is
//! given. These are plain data; drawing them is the UI crates' job.

use super::dialog::{AppChooser, Consent, FileChooser, FileEntry};
use super::interface::Grant;

/// A key as the screens receive it. Keys a dialog does not use are ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKey {
    Escape,
    Enter,
    Tab,
    Up,
    Down,
    Space,
    Backspace,
    Text(char),
}

/// The result a screen reports once its model has decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    Granted(Grant),
    Denied,
    Cancelled,
}

/// The dialog a screen draws. The model is the source of truth; the screen adds
/// only presentation state (cursor, focus, the last refusal) beside it.
#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    Consent(Consent),
    /// A FileChooser in any of its modes.
    Files(FileChooser),
    /// An AppChooser.
    App(AppChooser),
}

/// A directory listing the caller read for a FileChooser. The screen does no
/// I/O: it shows exactly the entries it is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryListing {
    /// The folder this listing is for, as the caller named it.
    pub folder: String,
    /// The entries to show, in the order given.
    pub entries: Vec<FileEntry>,
}
