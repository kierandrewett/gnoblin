//! What the compositor has to do for a portal request, as a seam with no pretence.
//!
//! Every interface in this backend ends in one call the compositor owns: open a stream,
//! take a screenshot, connect an EIS session, bind a shortcut, hand over the clipboard.
//! None of it can be written here. The portal is an IPC client
//! (`xtask/src/deps.rs`, `CLIENT_SIDE`), it may not link the compositor or an engine
//! crate, and the object schema declares no member for a screenshot, a stream, a
//! shortcut or a clipboard transfer.
//!
//! So the call is a trait, and the traits have exactly one default implementation:
//! [`Missing`], which refuses with a sentence naming what is missing. A trait object that
//! cannot do the thing is not a stub that pretends to work: every caller that reaches it
//! gets `org.freedesktop.portal.Error.NotSupported` and a reason a person can act on, and
//! the composition root can only change that by supplying an implementation — which is a
//! visible, single place to look.
//!
//! This is the same rule `crates/gnoblin-capture/src/lib.rs` states for PipeWire: a
//! feature that is not implemented is a missing feature, not a function that returns
//! nothing.
//!
//! # What is real, and what is a seam
//!
//! | Interface | Real here | Seam |
//! | --- | --- | --- |
//! | `Settings` | everything: the values come from the `settings` object | none |
//! | `Inhibit` | the request and monitoring session, `Close`, `QueryEndResponse` | [`InhibitBackend`] |
//! | `Screenshot` | request, consent, handle, cancel | [`CaptureBackend::screenshot`] |
//! | `ScreenCast` | session, source selection, consent, close | [`CaptureBackend::open_stream`] |
//! | `RemoteDesktop` | session, device selection, consent, close | [`CaptureBackend`] and [`EisBackend`] |
//! | `InputCapture` | session, barriers, enable/release, close | [`EisBackend`] |
//! | `GlobalShortcuts` | session, bindings list, consent, close | [`ShortcutsBackend`] |
//! | `Clipboard` | session, consent, the transfer state machine | [`ClipboardBackend`] |
//!
//! "# Real" means the state machine, the consent question, the refusals and the lifetime
//! are this crate's and are tested; it does not mean a stream reaches an app, because no
//! stream can until the compositor exposes one.

use std::collections::BTreeMap;
use std::fmt;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use gnoblin_capture::Target;
use gnoblin_model::Value;

/// The compositor's handle for one opened stream.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StreamId(u32);

impl StreamId {
    /// Wraps the backend's token.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw token.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "StreamId({})", self.0)
    }
}

/// One stream the compositor opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    /// The handle the caller closes it with.
    pub id: StreamId,
    /// The PipeWire node the app reads.
    pub node: u32,
    /// The stream's properties, in the `a{sv}` the interface returns: `position`,
    /// `size`, `source_type` and `mapping_id` are the keys the frontend forwards.
    pub properties: BTreeMap<String, Value>,
}

/// What a screenshot should cover.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenshotOptions {
    /// The app's window identifier, for a dialogue that centres on it.
    pub parent_window: String,
    /// Whether the frontend asked for an interactive choice.
    pub interactive: bool,
    /// Whether the frontend asked for a modal dialogue.
    pub modal: bool,
}

/// What a screen cast should capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamRequest {
    /// What to capture: an output or one window, in `gnoblin-capture`'s identities.
    pub target: Target,
    /// Whether the cursor is drawn into the frames (1 hidden, 2 embedded, 4 metadata).
    pub cursor_mode: u32,
    /// Whether the caller allowed more than one source.
    pub multiple: bool,
}

/// Why a screenshot did not produce a picture, and what the portal answers for each reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenshotError {
    /// The compositor cannot take screenshots here at all: the app is told the call is
    /// unsupported.
    Unavailable(Unavailable),
    /// The session refused the capture (locked, no output, no frame yet): the request ends
    /// with response 2, the same as a cancelled one.
    Refused(Unavailable),
    /// The picture was taken but could not be stored: the app is told why.
    Write(Unavailable),
}

/// The capture call the compositor owns.
pub trait CaptureBackend: Send + Sync {
    /// Take one screenshot of the screen and return the URI of the file it wrote.
    fn screenshot(&self, options: &ScreenshotOptions) -> Result<String, ScreenshotError>;

    /// Read the pixel under the pointer, as sRGB components in [0, 1].
    fn pick_color(&self) -> Result<(f64, f64, f64), Unavailable>;

    /// Open a stream for one target.
    fn open_stream(&self, request: &StreamRequest) -> Result<Stream, Unavailable>;

    /// Close a stream this backend opened.
    fn close_stream(&self, stream: StreamId);
}

/// The input call the compositor owns.
pub trait EisBackend: Send {
    /// Connect an EIS session and return the descriptor the app reads.
    ///
    /// The descriptor is the sender's half of a socket pair: `ConnectToEIS` hands a file
    /// descriptor to the caller, and the compositor's EIS server speaks libei on the other
    /// half. A backend that cannot open one refuses here rather than returning a
    /// descriptor that never delivers an event.
    fn connect(&mut self, session: &str, app_id: &str) -> Result<OwnedFd, Unavailable>;
}

/// The inhibit call the compositor owns.
pub trait InhibitBackend: Send {
    /// Hold `flags` (1 logout, 2 user switch, 4 suspend, 8 idle) until the returned value
    /// is released.
    fn inhibit(&mut self, app_id: &str, flags: u32, reason: &str) -> Result<InhibitHandle, Unavailable>;

    /// Release one inhibition.
    fn release(&mut self, handle: InhibitHandle);
}

/// What a backend gave an inhibition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InhibitHandle(u64);

impl InhibitHandle {
    /// Wraps the backend's token.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw token.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One shortcut a caller wants bound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shortcut {
    /// The caller's id, which the activation signal carries back.
    pub id: String,
    /// The description the user reads.
    pub description: String,
    /// The preferred trigger, as `g_variant` text (`CTRL+SHIFT+a`), if the caller named one.
    pub trigger: Option<String>,
}

/// The shortcut call the compositor owns.
pub trait ShortcutsBackend: Send {
    /// Bind these shortcuts for this session.
    fn bind(&mut self, session: &str, shortcuts: &[Shortcut]) -> Result<Vec<Shortcut>, Unavailable>;

    /// Reopen the user's shortcut editor for this session.
    fn configure(&mut self, session: &str) -> Result<(), Unavailable>;
}

/// The clipboard call the compositor owns.
pub trait ClipboardBackend: Send {
    /// Ask to own the session's clipboard.
    fn request(&mut self, session: &str) -> Result<(), Unavailable>;

    /// Offer these MIME types to the compositor's selection.
    fn set_selection(&mut self, session: &str, mime_types: &[String]) -> Result<(), Unavailable>;

    /// A descriptor to write the caller's data into, for one serial.
    fn selection_write(&mut self, session: &str, serial: u32) -> Result<OwnedFd, Unavailable>;

    /// A descriptor to read the compositor's selection from.
    fn selection_read(&mut self, session: &str, mime_type: &str) -> Result<OwnedFd, Unavailable>;

    /// Report whether the caller's write succeeded.
    fn write_done(&mut self, session: &str, serial: u32, success: bool);
}

/// Why a compositor call could not be made.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unavailable {
    /// What is missing, in a sentence the app's user can read.
    pub reason: String,
}

impl Unavailable {
    /// The refusal for a call the object schema has no member for.
    #[must_use]
    pub fn missing(member: &str) -> Unavailable {
        Unavailable {
            reason: format!(
                "the compositor serves no `{member}` member in IPC 0.2.0, so the portal cannot make this call"
            ),
        }
    }

    /// The refusal for a backend that refuses for its own reason.
    #[must_use]
    pub fn refused(reason: impl Into<String>) -> Unavailable {
        Unavailable { reason: reason.into() }
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for Unavailable {}

/// Every compositor call the backend makes, in one value.
///
/// One struct rather than five constructor arguments. The composition root supplies capture
/// from the compositor's IPC when a shell is reachable; every other call stays [`Missing`]
/// until its IPC member exists, and each refusal names what is missing.
pub struct Backends {
    /// The capture call.
    pub capture: Arc<dyn CaptureBackend>,
    /// The input call.
    pub eis: Box<dyn EisBackend>,
    /// The inhibit call.
    pub inhibit: Box<dyn InhibitBackend>,
    /// The shortcut call.
    pub shortcuts: Box<dyn ShortcutsBackend>,
    /// The clipboard call.
    pub clipboard: Box<dyn ClipboardBackend>,
}

impl Default for Backends {
    /// All five refuse, naming what is missing. This is what a session with no
    /// compositor-side support runs with, and every refusal says so.
    fn default() -> Self {
        Backends {
            capture: Arc::new(Missing),
            eis: Box::new(Missing),
            inhibit: Box::new(Missing),
            shortcuts: Box::new(Missing),
            clipboard: Box::new(Missing),
        }
    }
}

impl fmt::Debug for Backends {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Backends")
    }
}

/// The one implementation the composition root starts with: it refuses, naming the member.
///
/// One type for all five traits, because the reason is the same one in each case and five
/// copies of it would be five chances to drift.
#[derive(Clone, Copy, Debug, Default)]
pub struct Missing;

impl CaptureBackend for Missing {
    fn screenshot(&self, _options: &ScreenshotOptions) -> Result<String, ScreenshotError> {
        Err(ScreenshotError::Unavailable(Unavailable::missing("screenshot")))
    }

    fn pick_color(&self) -> Result<(f64, f64, f64), Unavailable> {
        Err(Unavailable::missing("pick-color"))
    }

    fn open_stream(&self, _request: &StreamRequest) -> Result<Stream, Unavailable> {
        Err(Unavailable::missing("capture-open-stream"))
    }

    fn close_stream(&self, _stream: StreamId) {}
}

impl EisBackend for Missing {
    fn connect(&mut self, _session: &str, _app_id: &str) -> Result<OwnedFd, Unavailable> {
        Err(Unavailable::missing("eis-connect"))
    }
}

impl InhibitBackend for Missing {
    fn inhibit(&mut self, _app_id: &str, _flags: u32, _reason: &str) -> Result<InhibitHandle, Unavailable> {
        Err(Unavailable::missing("inhibit"))
    }

    fn release(&mut self, _handle: InhibitHandle) {}
}

impl ShortcutsBackend for Missing {
    fn bind(&mut self, _session: &str, _shortcuts: &[Shortcut]) -> Result<Vec<Shortcut>, Unavailable> {
        Err(Unavailable::missing("shortcuts-bind"))
    }

    fn configure(&mut self, _session: &str) -> Result<(), Unavailable> {
        Err(Unavailable::missing("shortcuts-configure"))
    }
}

impl ClipboardBackend for Missing {
    fn request(&mut self, _session: &str) -> Result<(), Unavailable> {
        Err(Unavailable::missing("clipboard-request"))
    }

    fn set_selection(&mut self, _session: &str, _mime_types: &[String]) -> Result<(), Unavailable> {
        Err(Unavailable::missing("clipboard-set-selection"))
    }

    fn selection_write(&mut self, _session: &str, _serial: u32) -> Result<OwnedFd, Unavailable> {
        Err(Unavailable::missing("clipboard-selection-write"))
    }

    fn selection_read(&mut self, _session: &str, _mime_type: &str) -> Result<OwnedFd, Unavailable> {
        Err(Unavailable::missing("clipboard-selection-read"))
    }

    fn write_done(&mut self, _session: &str, _serial: u32, _success: bool) {}
}
