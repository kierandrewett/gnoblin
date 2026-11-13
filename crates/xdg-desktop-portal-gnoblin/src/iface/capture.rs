//! `org.freedesktop.impl.portal.Screenshot` and `ScreenCast`.
//!
//! Both read frames, so both ask for [`PortalCapability::Capture`] and nothing else. What
//! differs is the lifetime: a screenshot is one frame inside one call, and its session is
//! the audit row the call leaves behind; a screen cast is a session the frontend starts,
//! configures and closes, and its streams end with it.
//!
//! # What is real, and what is not
//!
//! Real: the request handle and its cancellation, the session table, the source selection
//! as the caller asked for it and as this backend narrows it, the consent question with
//! D14's refusal, the response codes, the close and the stream bookkeeping.
//!
//! Real for a screenshot: the frames come from the compositor's `capture.screen` member,
//! through [`crate::capture_relay`], which writes the PNG and answers with its URI. Only
//! full-screen screenshots are served.
//!
//! The screen cast frames are the compositor's: [`crate::backends::CaptureBackend::open_stream`]
//! tells it the user approved a cast of the chosen source (`cast.grant`) and starts the PipeWire
//! stream (`cast.start`), and the node it answers is the one the app reads. The compositor
//! refuses a start for a session it was not told the user approved, so a portal that skipped the
//! question has no node to return. A compositor with no PipeWire daemon refuses, and the stream
//! list is never fabricated.
//!
//! # Sources: monitors, Views and windows
//!
//! `SelectSources` `types` is the portal's: [`SOURCE_MONITOR`] shares an output (the one the `output` option names,
//! or the first), and [`SOURCE_VIRTUAL`] shares a **View**, a scene node that draws a subtree of the desktop (the
//! shell's or an extension's own picture). The portal API cannot name a node, so the View is named by this
//! backend's own [`OPTION_VIEW`] option, a `t` holding the node id; the frontend forwards unknown options. A
//! virtual source with no `view` is refused at `SelectSources`, before the user is asked. The consent dialog,
//! the privacy indicator, the audit log and the lock rule are the ones a monitor share has: the user is asked, a
//! stream is a visible session while it runs, and a lock ends it. Secure windows inside the View are masked in
//! the stream as in every other capture.
//!
//! [`SOURCE_WINDOW`] shares **one window**, named by the compositor's identity for it in this backend's own
//! [`OPTION_WINDOW`] option (a `t`, the `id` of the `window` object). A selection that asks for windows only and
//! names none is refused at `SelectSources`, and so is one that names a window and a View. The user is asked about
//! that window every time: the question carries the window, the compositor words it with the window's own title and
//! app id, and an earlier answer for the app or for another window is never reused. The compositor refuses the
//! share unless it recorded that the user was asked about and allowed that very window. The stream is the window's
//! rectangle at the window's size, follows the window when it is moved or resized, shows a secure window as the
//! session's treatment and not as its pixels, and ends with the window or the lock. Its `position` and `size`
//! properties are the window's place and extent, a monitor's are the output's, and a View has a `size` only.
//!
//! # The narrower option set
//!
//! A sandboxed caller may not share another app's window or a View, and may not receive cursor
//! metadata, because each hands a sandboxed app something about the desktop it did not
//! ask the user for. [`narrow`] is the whole rule, and it runs before the selection is
//! recorded, so the session says what was allowed rather than what was asked.

use std::collections::BTreeMap;

use gnoblin_capture::Target;
use gnoblin_types::{NodeId, OutputId, WindowId};
use zbus::OwnedVariant;
use zbus::message::Header;
use zbus::{Connection, ObjectPath, fdo};

use super::{Portal, Reply, await_consent, export_request, refused, remove, sender, unsupported};
use crate::backends::{ScreenshotError, ScreenshotOptions, StreamProperty, StreamRequest};
use crate::consent::{ConsentOutcome, Interface, NewSession, PortalCapability};

use crate::log;

/// `types` bit: an existing monitor.
pub const SOURCE_MONITOR: u32 = 1;
/// `types` bit: one application window.
pub const SOURCE_WINDOW: u32 = 2;
/// `types` bit: a virtual monitor. In this backend it is a View, a scene node the shell or an extension draws and
/// names: the stream is what the View draws, at the View's size. The View is named by the [`OPTION_VIEW`] option of
/// `SelectSources`.
pub const SOURCE_VIRTUAL: u32 = 4;
/// The `SelectSources` option that names the View a [`SOURCE_VIRTUAL`] source is: a `t` (or `u`) holding the View's
/// node id. It is this backend's own key, because the portal API has no way to name a node, and the frontend passes
/// unknown options through; an app that does not know a View's node id has no View to name.
pub const OPTION_VIEW: &str = "view";
/// The `SelectSources` option that names the window a [`SOURCE_WINDOW`] source is: a `t` (or `u`) holding the
/// window's compositor identity, the `id` of its `window` object. It is this backend's own key, because the portal
/// API has no way to name a window without its picker.
pub const OPTION_WINDOW: &str = gnoblin_client::wire::PORTAL_OPTION_WINDOW;
/// `cursor_mode` bit: the cursor is not in the frames.
pub const CURSOR_HIDDEN: u32 = 1;
/// `cursor_mode` bit: the cursor is drawn into the frames.
pub const CURSOR_EMBEDDED: u32 = 2;
/// `cursor_mode` bit: the cursor travels as stream metadata.
pub const CURSOR_METADATA: u32 = 4;

/// The options dictionary every method of the impl interfaces carries.
pub type Options = BTreeMap<String, OwnedVariant>;

/// The version of the `Screenshot` interface this build serves.
const SCREENSHOT_VERSION: u32 = 2;
/// The version of the `ScreenCast` interface: streams with `mapping_id`, which 5 added,
/// are not served, so the reported version is the last one this build is complete for.
const SCREENCAST_VERSION: u32 = 3;

/// `org.freedesktop.impl.portal.Screenshot`.
#[derive(Debug)]
pub struct Screenshot {
    portal: Portal,
}

impl Screenshot {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> Screenshot {
        Screenshot {
            portal: Portal { shared, connection },
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Screenshot")]
impl Screenshot {
    /// Take one screenshot.
    ///
    /// The handle is exported as a request object before the prompt is offered, so the
    /// frontend can cancel the wait. The session the call records is the request's own
    /// history: it exists for the length of the call and is closed whatever the answer is.
    async fn screenshot(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = handle.as_str().to_owned();
        export_request(&self.portal, &path).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller.clone(),
                    app_id: app_id.clone(),
                    sandboxed: sandboxed(&options),
                    interface: Interface::Screenshot,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        let outcome = await_consent(
            &self.portal,
            &path,
            &caller,
            &path,
            PortalCapability::Capture,
            &parent_window,
        )
        .await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => {
                let options = ScreenshotOptions {
                    parent_window,
                    interactive: flag(&options, "interactive"),
                    modal: flag(&options, "modal"),
                };
                // The backend is cloned out of the lock: a capture takes a while, and every
                // other request needs the lock to answer.
                let shot = {
                    let capture = std::sync::Arc::clone(&self.portal.lock().capture);
                    capture.screenshot(&options)
                };
                match shot {
                    Ok(uri) => {
                        let mut results = BTreeMap::new();
                        results.insert("uri".to_owned(), text(&uri));
                        (0, results)
                    }
                    Err(ScreenshotError::Refused(reason)) => {
                        log::info!(detail = %reason, "the compositor refused a screenshot");
                        (2, BTreeMap::new())
                    }
                    Err(ScreenshotError::Unavailable(missing)) => {
                        let _ = self.portal.close_session(&caller, &path);
                        let _ = remove(&self.portal.connection, &path).await;
                        return Err(unsupported(missing));
                    }
                    Err(ScreenshotError::Write(problem)) => {
                        let _ = self.portal.close_session(&caller, &path);
                        let _ = remove(&self.portal.connection, &path).await;
                        return Err(fdo::Error::Failed(problem.reason));
                    }
                }
            }
            Ok(outcome) => (outcome.response(), BTreeMap::new()),
            Err(refusal) => {
                let _ = self.portal.close_session(&caller, &path);
                let _ = remove(&self.portal.connection, &path).await;
                return Err(refusal);
            }
        };
        let _ = self.portal.close_session(&caller, &path);
        let _ = remove(&self.portal.connection, &path).await;
        log::info!(app = %app_id, response = reply.0, "a screenshot request finished");
        Ok(reply)
    }

    /// Read one pixel.
    ///
    /// The colour is the compositor's call, and the default backend refuses it: a picked
    /// colour is a capture of what is under the pointer, so it goes through the same
    /// consent and the same seam as a screenshot.
    async fn pick_color(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = handle.as_str().to_owned();
        export_request(&self.portal, &path).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller.clone(),
                    app_id,
                    sandboxed: sandboxed(&options),
                    interface: Interface::Screenshot,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        let outcome = await_consent(
            &self.portal,
            &path,
            &caller,
            &path,
            PortalCapability::Capture,
            &parent_window,
        )
        .await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => {
                let picked = {
                    let shared = self.portal.lock();
                    shared.capture.pick_color()
                };
                match picked {
                    Ok((red, green, blue)) => {
                        let mut results = BTreeMap::new();
                        results.insert("color".to_owned(), color(red, green, blue));
                        (0, results)
                    }
                    Err(missing) => {
                        let _ = self.portal.close_session(&caller, &path);
                        let _ = remove(&self.portal.connection, &path).await;
                        return Err(unsupported(missing));
                    }
                }
            }
            Ok(outcome) => (outcome.response(), BTreeMap::new()),
            Err(refusal) => {
                let _ = self.portal.close_session(&caller, &path);
                let _ = remove(&self.portal.connection, &path).await;
                return Err(refusal);
            }
        };
        let _ = self.portal.close_session(&caller, &path);
        let _ = remove(&self.portal.connection, &path).await;
        Ok(reply)
    }

    /// The interface version.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        SCREENSHOT_VERSION
    }
}

/// `org.freedesktop.impl.portal.ScreenCast`.
#[derive(Debug)]
pub struct ScreenCast {
    portal: Portal,
}

impl ScreenCast {
    /// The interface sharing the portal's state.
    #[must_use]
    pub fn new(shared: std::sync::Arc<std::sync::Mutex<super::Shared>>, connection: Connection) -> ScreenCast {
        ScreenCast {
            portal: Portal { shared, connection },
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    /// Create a screen cast session.
    ///
    /// No prompt is made here: a session is a handle, and the user is asked when there is
    /// something to show them (at `Start`). That is also what makes D14's refusal land at
    /// the call the user was waiting for rather than at a creation the frontend makes
    /// speculatively.
    async fn create_session(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        {
            let mut shared = self.portal.lock();
            shared
                .guard
                .create(NewSession {
                    bus_name: caller.clone(),
                    app_id: app_id.clone(),
                    sandboxed: sandboxed(&options),
                    interface: Interface::ScreenCast,
                    path: path.clone(),
                })
                .map_err(refused)?;
        }
        super::export_session(&self.portal, &path).await?;
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        let mut results = BTreeMap::new();
        results.insert("session_id".to_owned(), text(&path));
        log::info!(app = %app_id, session = %path, "a screen cast session was created");
        Ok((0, results))
    }

    /// Record what the caller wants captured.
    async fn select_sources(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        let narrowed = {
            let mut shared = self.portal.lock();
            let session = shared
                .guard
                .session(&caller, &path)
                .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?
                .clone();
            let selection = narrow(session.owner.narrowed(), &options);
            // A virtual source is a View and a window source is one window, and both are named by the caller: the
            // portal's `SelectSources` has no other way to say which one. A selection that asks for one and names
            // none is refused here, before the user is asked about a share that could not be made.
            if let Some(problem) = unnamed_source(&selection) {
                return Err(fdo::Error::InvalidArgs(problem));
            }
            shared
                .guard
                .configure(&caller, &path, selection.clone())
                .map_err(refused)?;
            selection
        };
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        log::info!(app = %app_id,
        session = %path,
        types = narrowed.get("types").map(|value| format!("{value:?}")),
        "a screen cast source selection was recorded");
        Ok((0, BTreeMap::new()))
    }

    /// Start the session: ask the user, then open the stream.
    async fn start(
        &self,
        #[zbus(header)] header: Header<'_>,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        options: Options,
    ) -> fdo::Result<Reply> {
        let caller = sender(&header)?;
        let path = session_handle.as_str().to_owned();
        export_request(&self.portal, handle.as_str()).await?;
        let _ = options;
        let outcome = await_consent(
            &self.portal,
            handle.as_str(),
            &caller,
            &path,
            PortalCapability::Capture,
            &parent_window,
        )
        .await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => {
                let request = {
                    let shared = self.portal.lock();
                    let session = shared
                        .guard
                        .session(&caller, &path)
                        .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?;
                    StreamRequest {
                        session: path.clone(),
                        // The picker is the provider's, and this backend's prompt answers allow/deny, so the
                        // target is what the selection named: a View when it asked for a virtual source and gave
                        // the View's node id, the window when it asked for a window source and named one (and the
                        // user was asked about that window, see `Guard::ask`), else the output it named or the
                        // first one.
                        target: cast_target(&session.options),
                        cursor_mode: recorded_uint(&session.options, "cursor_mode").unwrap_or(CURSOR_EMBEDDED),
                        multiple: recorded_flag(&session.options, "multiple"),
                    }
                };
                // The compositor is called with the lock released: the thread that reads its
                // answers takes the same lock.
                let capture = self.portal.capture();
                match capture.open_stream(&request) {
                    Ok(stream) => {
                        let recorded = self.portal.lock().guard.record_stream(&path, stream.id);
                        let started = recorded.and_then(|()| {
                            let mut shared = self.portal.lock();
                            shared.guard.start(&caller, &path).map(|_| ())
                        });
                        if let Err(refusal) = started {
                            // The session went away while the stream was opening, or was
                            // started twice: nothing holds the stream, so it ends here.
                            capture.close_stream(stream.id);
                            return Err(refused(refusal));
                        }
                        let opened = vec![(stream.node, properties(&stream.properties))];
                        let mut results = BTreeMap::new();
                        results.insert("streams".to_owned(), streams(&opened));
                        (0, results)
                    }
                    Err(missing) => return Err(unsupported(missing)),
                }
            }
            Ok(outcome) => (outcome.response(), BTreeMap::new()),
            Err(refusal) => return Err(refusal),
        };
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        log::info!(app = %app_id, session = %path, response = reply.0, "a screen cast start finished");
        Ok(reply)
    }

    /// The source types this build can offer.
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        // A virtual monitor is a View: the compositor can stream one when the caller names it
        // (`OPTION_VIEW`), so it is advertised and a selection that names none is refused.
        SOURCE_MONITOR | SOURCE_WINDOW | SOURCE_VIRTUAL
    }

    /// The cursor modes this build can offer.
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED | CURSOR_METADATA
    }

    /// The interface version.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        SCREENCAST_VERSION
    }
}

/// Narrow a source selection to what this backend will record for this caller.
///
/// The wide set is a host app's: monitors and windows, several sources, and the cursor as
/// metadata. The narrow set is a sandboxed or unidentified caller's: monitors only,
/// one source, and no cursor metadata, because a window share and a cursor stream both
/// describe other applications to a caller the user was not asked about.
#[must_use]
pub fn narrow(narrowed: bool, options: &Options) -> BTreeMap<String, gnoblin_model::Value> {
    let requested_types = uint(options, "types").unwrap_or(SOURCE_MONITOR);
    let allowed_types = if narrowed {
        SOURCE_MONITOR
    } else {
        SOURCE_MONITOR | SOURCE_WINDOW | SOURCE_VIRTUAL
    };
    let cursor_allowed = if narrowed {
        CURSOR_HIDDEN | CURSOR_EMBEDDED
    } else {
        CURSOR_HIDDEN | CURSOR_EMBEDDED | CURSOR_METADATA
    };
    let mut selection = BTreeMap::new();
    selection.insert(
        "types".to_owned(),
        gnoblin_model::Value::Uint(u64::from(requested_types & allowed_types)),
    );
    selection.insert(
        "multiple".to_owned(),
        gnoblin_model::Value::Bool(!narrowed && flag(options, "multiple")),
    );
    selection.insert(
        "cursor_mode".to_owned(),
        gnoblin_model::Value::Uint(u64::from(
            uint(options, "cursor_mode").unwrap_or(CURSOR_HIDDEN) & cursor_allowed,
        )),
    );
    if let Some(output) = uint(options, "output") {
        selection.insert("output".to_owned(), gnoblin_model::Value::Uint(u64::from(output)));
    }
    // The View a virtual source is. It is recorded only when the caller may have a virtual source at all.
    if requested_types & allowed_types & SOURCE_VIRTUAL != 0
        && let Some(view) = wide_uint(options, OPTION_VIEW)
    {
        selection.insert(OPTION_VIEW.to_owned(), gnoblin_model::Value::Uint(view));
    }
    // The window a window source is, recorded only when the caller may have one at all. Naming a window is what
    // makes the share a window's: the consent question and the stream are about this one window.
    if requested_types & allowed_types & SOURCE_WINDOW != 0
        && let Some(window) = wide_uint(options, OPTION_WINDOW)
    {
        selection.insert(OPTION_WINDOW.to_owned(), gnoblin_model::Value::Uint(window));
    }
    // A persist mode needs a permission store, and this backend keeps none: the answer is
    // always "do not persist", which is what the results report rather than what was
    // asked for.
    selection.insert("persist_mode".to_owned(), gnoblin_model::Value::Uint(0));
    selection
}

/// Whether the request declared its caller sandboxed.
pub(crate) fn sandboxed(options: &Options) -> Option<bool> {
    options.get("sandboxed").and_then(|value| bool::try_from(value).ok())
}

/// One boolean option.
pub(crate) fn flag(options: &Options, name: &str) -> bool {
    options
        .get(name)
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or(false)
}

/// One unsigned option, narrowed to the width the interface uses.
pub(crate) fn uint(options: &Options, name: &str) -> Option<u32> {
    wide_uint(options, name).and_then(|raw| u32::try_from(raw).ok())
}

/// One unsigned option at its full width: a View's node id carries an epoch in its high half, so it does not fit the
/// `u` the interface's own options use.
pub(crate) fn wide_uint(options: &Options, name: &str) -> Option<u64> {
    let value = options.get(name)?;
    if let Ok(raw) = u32::try_from(value) {
        return Some(u64::from(raw));
    }
    if let Ok(raw) = u64::try_from(value) {
        return Some(raw);
    }
    i64::try_from(value).ok().and_then(|raw| u64::try_from(raw).ok())
}

/// What a recorded selection casts: the View it named when it asked for a virtual source, else the window it named
/// when it asked for a window source, else the output it named, else the first one.
pub(crate) fn cast_target(selection: &BTreeMap<String, gnoblin_model::Value>) -> Target {
    let types = recorded_uint(selection, "types").unwrap_or(0);
    if types & SOURCE_VIRTUAL != 0
        && let Some(gnoblin_model::Value::Uint(view)) = selection.get(OPTION_VIEW)
    {
        return Target::View(NodeId::new(*view));
    }
    if types & SOURCE_WINDOW != 0
        && let Some(gnoblin_model::Value::Uint(window)) = selection.get(OPTION_WINDOW)
    {
        return Target::Window(WindowId::new(*window));
    }
    Target::Output(OutputId::new(
        recorded_uint(selection, "output").map_or(gnoblin_client::wire::OUTPUT_FIRST, u64::from),
    ))
}

/// Why a recorded selection names no source the compositor could stream, if it does not.
///
/// A virtual source is a View and a window source is one window, and neither has a default: a selection that asks
/// for one and names none, or that names a View and a window both, is refused before the user is asked.
pub(crate) fn unnamed_source(selection: &BTreeMap<String, gnoblin_model::Value>) -> Option<String> {
    let types = recorded_uint(selection, "types").unwrap_or(0);
    let view = selection.contains_key(OPTION_VIEW);
    let window = selection.contains_key(OPTION_WINDOW);
    if types & SOURCE_VIRTUAL != 0 && !view {
        return Some(format!(
            "a VIRTUAL source is a View, and the selection names none: add the `{OPTION_VIEW}` option (a uint64 \
             node id)"
        ));
    }
    // A window source next to a monitor or a View is the app's own choice among sources, and what is cast is then
    // the monitor, or the View it named; only a selection that can be nothing but a window needs the window named.
    if types & SOURCE_WINDOW != 0 && types & (SOURCE_MONITOR | SOURCE_VIRTUAL) == 0 && !window {
        return Some(format!(
            "a WINDOW source is one window, and the selection names none: add the `{OPTION_WINDOW}` option (a \
             uint64 window identity)"
        ));
    }
    if view && window {
        return Some(format!(
            "the selection names both a View (`{OPTION_VIEW}`) and a window (`{OPTION_WINDOW}`): name one source"
        ));
    }
    None
}

/// One unsigned value from a recorded selection, narrowed to the width the interface uses.
///
/// The selection is what `narrow` wrote, so it holds `Uint`s; a value of another kind was
/// never recorded and reads as absent.
pub(crate) fn recorded_uint(selection: &BTreeMap<String, gnoblin_model::Value>, name: &str) -> Option<u32> {
    match selection.get(name)? {
        gnoblin_model::Value::Uint(raw) => u32::try_from(*raw).ok(),
        _ => None,
    }
}

/// One boolean value from a recorded selection.
fn recorded_flag(selection: &BTreeMap<String, gnoblin_model::Value>, name: &str) -> bool {
    matches!(selection.get(name), Some(gnoblin_model::Value::Bool(true)))
}

/// A string, as the variant a result carries.
pub(crate) fn text(value: &str) -> OwnedVariant {
    OwnedVariant::try_from(zbus::Variant::from(value)).expect("a string is a value")
}

/// The accent colour's shape, for `PickColor`.
fn color(red: f64, green: f64, blue: f64) -> OwnedVariant {
    let structure = zbus::Structure::from((red, green, blue));
    OwnedVariant::try_from(zbus::Variant::Structure(structure)).expect("three doubles are a value")
}

/// One stream's properties, as the dictionary the frontend forwards: `source_type` is a `u`, and `position` and
/// `size` are `(ii)` pairs, the types the interface declares for them.
pub(crate) fn properties(properties: &BTreeMap<String, StreamProperty>) -> BTreeMap<String, OwnedVariant> {
    properties
        .iter()
        .filter_map(|(name, property)| {
            let value = match *property {
                StreamProperty::Uint(raw) => zbus::Variant::from(raw),
                StreamProperty::Pair(first, second) => zbus::Variant::Structure(zbus::Structure::from((first, second))),
            };
            OwnedVariant::try_from(value).ok().map(|value| (name.clone(), value))
        })
        .collect()
}

/// The `streams` result: `a(ua{sv})`.
///
/// The array is built from `Value`s rather than from a Rust tuple so the signature is the
/// one the interface declares, which is what a frontend decodes with.
pub fn streams(streams: &[(u32, BTreeMap<String, OwnedVariant>)]) -> OwnedVariant {
    use zbus::{Array, Signature, Structure, Variant};
    let signature = Signature::try_from("(ua{sv})").expect("the streams signature is a constant");
    let mut array = Array::new(&signature);
    for (node, properties) in streams {
        let dictionary: std::collections::HashMap<String, OwnedVariant> = properties.clone().into_iter().collect();
        let entry = Structure::from((*node, dictionary));
        if let Err(error) = array.append(Variant::Structure(entry)) {
            log::warn!(%error, "a stream could not be added to the reply");
        }
    }
    OwnedVariant::try_from(Variant::Array(array)).expect("an array of streams is a value")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(pairs: &[(&str, zbus::Variant<'_>)]) -> Options {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    OwnedVariant::try_from(value.clone()).expect("a plain value"),
                )
            })
            .collect()
    }

    /// A View's node id carries an epoch in its high half, so it is a `t` and survives whole.
    const VIEW: u64 = (1 << 32) | 0x102;

    #[test]
    fn a_virtual_source_with_a_view_is_recorded_and_casts_that_view() {
        let selection = narrow(
            false,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_VIRTUAL)),
                (OPTION_VIEW, zbus::Variant::from(VIEW)),
            ]),
        );
        assert_eq!(recorded_uint(&selection, "types"), Some(SOURCE_VIRTUAL));
        assert_eq!(selection.get(OPTION_VIEW), Some(&gnoblin_model::Value::Uint(VIEW)));
        assert_eq!(cast_target(&selection), Target::View(NodeId::new(VIEW)));
    }

    #[test]
    fn a_monitor_source_casts_the_output_it_names_or_the_first_and_ignores_a_view() {
        let first = narrow(
            false,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_MONITOR)),
                (OPTION_VIEW, zbus::Variant::from(VIEW)),
            ]),
        );
        assert!(!first.contains_key(OPTION_VIEW), "a monitor share records no View");
        assert_eq!(
            cast_target(&first),
            Target::Output(OutputId::new(gnoblin_client::wire::OUTPUT_FIRST))
        );
        let named = narrow(
            false,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_MONITOR)),
                ("output", zbus::Variant::from(2_u32)),
            ]),
        );
        assert_eq!(cast_target(&named), Target::Output(OutputId::new(2)));
    }

    #[test]
    fn a_sandboxed_caller_is_never_given_a_view() {
        let selection = narrow(
            true,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_VIRTUAL)),
                (OPTION_VIEW, zbus::Variant::from(VIEW)),
            ]),
        );
        assert_eq!(
            recorded_uint(&selection, "types"),
            Some(0),
            "the session says what was allowed: no virtual source"
        );
        assert!(!selection.contains_key(OPTION_VIEW));
        assert!(matches!(cast_target(&selection), Target::Output(_)));
    }

    #[test]
    fn a_view_id_given_as_a_u32_is_read_whole() {
        let selection = narrow(
            false,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_VIRTUAL)),
                (OPTION_VIEW, zbus::Variant::from(0x102_u32)),
            ]),
        );
        assert_eq!(cast_target(&selection), Target::View(NodeId::new(0x102)));
    }

    /// A window's compositor identity.
    const WINDOW: u64 = 0x2_0000_0007;

    #[test]
    fn a_window_source_that_names_a_window_is_recorded_and_casts_that_window() {
        let selection = narrow(
            false,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_WINDOW)),
                (OPTION_WINDOW, zbus::Variant::from(WINDOW)),
            ]),
        );
        assert_eq!(recorded_uint(&selection, "types"), Some(SOURCE_WINDOW));
        assert_eq!(selection.get(OPTION_WINDOW), Some(&gnoblin_model::Value::Uint(WINDOW)));
        assert_eq!(cast_target(&selection), Target::Window(WindowId::new(WINDOW)));
        assert_eq!(unnamed_source(&selection), None);
    }

    #[test]
    fn a_window_source_that_names_no_window_is_refused_before_anyone_is_asked() {
        let selection = narrow(false, &options(&[("types", zbus::Variant::from(SOURCE_WINDOW))]));
        let problem = unnamed_source(&selection).expect("a window-only selection must name its window");
        assert!(problem.contains(OPTION_WINDOW), "{problem}");
        // The app that is content with a monitor too gets the monitor, as before.
        let either = narrow(
            false,
            &options(&[("types", zbus::Variant::from(SOURCE_MONITOR | SOURCE_WINDOW))]),
        );
        assert_eq!(unnamed_source(&either), None);
        assert!(matches!(cast_target(&either), Target::Output(_)));
    }

    #[test]
    fn a_selection_naming_a_view_and_a_window_is_refused() {
        let selection = narrow(
            false,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_VIRTUAL | SOURCE_WINDOW)),
                (OPTION_VIEW, zbus::Variant::from(VIEW)),
                (OPTION_WINDOW, zbus::Variant::from(WINDOW)),
            ]),
        );
        assert!(unnamed_source(&selection).is_some_and(|problem| problem.contains("name one source")));
    }

    #[test]
    fn a_window_is_only_recorded_for_a_window_source_and_never_for_a_sandboxed_caller() {
        let monitor = narrow(
            false,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_MONITOR)),
                (OPTION_WINDOW, zbus::Variant::from(WINDOW)),
            ]),
        );
        assert!(
            !monitor.contains_key(OPTION_WINDOW),
            "a monitor share records no window, so nobody is asked about one"
        );
        let sandboxed = narrow(
            true,
            &options(&[
                ("types", zbus::Variant::from(SOURCE_WINDOW)),
                (OPTION_WINDOW, zbus::Variant::from(WINDOW)),
            ]),
        );
        assert!(
            !sandboxed.contains_key(OPTION_WINDOW),
            "a sandboxed caller may not share another app's window"
        );
        assert!(matches!(cast_target(&sandboxed), Target::Output(_)));
    }

    /// The interface declares `source_type` a `u`, and `position` and `size` `(ii)`: the encoder gives exactly
    /// those, not a `t` and not a list.
    #[test]
    fn stream_properties_carry_the_types_the_interface_declares() {
        let mut stream = BTreeMap::new();
        stream.insert("source_type".to_owned(), StreamProperty::Uint(SOURCE_WINDOW));
        stream.insert("position".to_owned(), StreamProperty::Pair(-40, 8));
        stream.insert("size".to_owned(), StreamProperty::Pair(640, 480));
        let encoded = properties(&stream);
        assert_eq!(encoded["source_type"].value_signature().to_string(), "u");
        assert_eq!(u32::try_from(&encoded["source_type"]).ok(), Some(SOURCE_WINDOW));
        for (name, want) in [("position", (-40_i32, 8_i32)), ("size", (640, 480))] {
            assert_eq!(encoded[name].value_signature().to_string(), "(ii)", "{name}");
            let zbus::Variant::Structure(pair) = &*encoded[name] else {
                panic!("{name} is a structure");
            };
            let fields: Vec<i32> = pair
                .fields()
                .iter()
                .map(|field| i32::try_from(field).expect("an i32"))
                .collect();
            assert_eq!(fields, [want.0, want.1], "{name}");
        }
    }
}
