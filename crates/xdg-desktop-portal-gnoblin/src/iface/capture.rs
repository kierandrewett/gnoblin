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
//! Not real: the frames. [`crate::backends::CaptureBackend`] is the compositor's call, and
//! the default refuses with a sentence naming the missing schema member. A screen cast
//! therefore fails at `Start` with `org.freedesktop.portal.Error.NotSupported` after the
//! user has been asked, and the stream list is never fabricated.
//!
//! # The narrower option set
//!
//! A sandboxed caller may not share another app's window and may not receive cursor
//! metadata, because both hand a sandboxed app something about the desktop it did not
//! ask the user for. [`narrow`] is the whole rule, and it runs before the selection is
//! recorded, so the session says what was allowed rather than what was asked.

use std::collections::BTreeMap;

use gnoblin_capture::Target;
use gnoblin_types::OutputId;
use zbus::OwnedVariant;
use zbus::message::Header;
use zbus::{Connection, ObjectPath, fdo};

use super::{Portal, Reply, await_consent, export_request, refused, remove, sender, unsupported, variant};
use crate::backends::{ScreenshotOptions, StreamRequest};
use crate::consent::{ConsentOutcome, Interface, NewSession, PortalCapability};

/// `types` bit: an existing monitor.
pub const SOURCE_MONITOR: u32 = 1;
/// `types` bit: one application window.
pub const SOURCE_WINDOW: u32 = 2;
/// `types` bit: a new virtual monitor.
pub const SOURCE_VIRTUAL: u32 = 4;
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
        let outcome = await_consent(&self.portal, &path, &caller, &path, PortalCapability::Capture).await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => {
                let options = ScreenshotOptions {
                    parent_window,
                    interactive: flag(&options, "interactive"),
                    modal: flag(&options, "modal"),
                };
                let shot = {
                    let mut shared = self.portal.lock();
                    shared.capture.screenshot(&options)
                };
                match shot {
                    Ok(uri) => {
                        let mut results = BTreeMap::new();
                        results.insert("uri".to_owned(), text(&uri));
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
        gnoblin_log::info!(app = %app_id, response = reply.0, "a screenshot request finished");
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
        let outcome = await_consent(&self.portal, &path, &caller, &path, PortalCapability::Capture).await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => {
                let _ = parent_window;
                let picked = {
                    let mut shared = self.portal.lock();
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
    #[zbus(property)]
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
        gnoblin_log::info!(app = %app_id, session = %path, "a screen cast session was created");
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
            shared
                .guard
                .configure(&caller, &path, selection.clone())
                .map_err(refused)?;
            selection
        };
        let _ = remove(&self.portal.connection, handle.as_str()).await;
        gnoblin_log::info!(app = %app_id,
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
        let _ = parent_window;
        let _ = options;
        let outcome = await_consent(&self.portal, handle.as_str(), &caller, &path, PortalCapability::Capture).await;
        let reply = match outcome {
            Ok(ConsentOutcome::Granted) => {
                let request = {
                    let shared = self.portal.lock();
                    let session = shared
                        .guard
                        .session(&caller, &path)
                        .ok_or_else(|| refused(crate::consent::Refusal::UnknownSession { path: path.clone() }))?;
                    StreamRequest {
                        // The picker is the provider's, and this backend's prompt answers
                        // allow/deny, so the target is the output the selection named or
                        // the first one. A window selection would need the picker's answer
                        // to carry an identity, which the request layer's answer does not:
                        // stated here rather than guessed at from a monitor number.
                        target: Target::Output(OutputId::new(u64::from(
                            recorded_uint(&session.options, "output").unwrap_or(1),
                        ))),
                        cursor_mode: recorded_uint(&session.options, "cursor_mode").unwrap_or(CURSOR_EMBEDDED),
                        multiple: recorded_flag(&session.options, "multiple"),
                    }
                };
                let opened = {
                    let mut shared = self.portal.lock();
                    let opened = shared.capture.open_stream(&request);
                    if let Ok(stream) = &opened {
                        let _ = shared.guard.record_stream(&path, stream.id);
                    }
                    opened
                };
                match opened {
                    Ok(stream) => {
                        {
                            let mut shared = self.portal.lock();
                            shared.guard.start(&caller, &path).map_err(refused)?;
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
        gnoblin_log::info!(app = %app_id, session = %path, response = reply.0, "a screen cast start finished");
        Ok(reply)
    }

    /// The source types this build can offer.
    #[zbus(property)]
    fn available_source_types(&self) -> u32 {
        // A virtual monitor needs a compositor call the schema has no member for, so it is
        // not advertised: a frontend that offers it would create an expectation this
        // backend cannot meet.
        SOURCE_MONITOR | SOURCE_WINDOW
    }

    /// The cursor modes this build can offer.
    #[zbus(property)]
    fn available_cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED | CURSOR_METADATA
    }

    /// The interface version.
    #[zbus(property)]
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
        SOURCE_MONITOR | SOURCE_WINDOW
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
    let value = options.get(name)?;
    if let Ok(raw) = u64::try_from(value) {
        return u32::try_from(raw).ok();
    }
    i64::try_from(value).ok().and_then(|raw| u32::try_from(raw).ok())
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

/// One stream's properties, as the dictionary the frontend forwards.
fn properties(properties: &BTreeMap<String, gnoblin_model::Value>) -> BTreeMap<String, OwnedVariant> {
    let mut out = BTreeMap::new();
    for (name, value) in properties {
        if let Some(value) = variant(value) {
            out.insert(name.clone(), value);
        }
    }
    out
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
            gnoblin_log::warn!(%error, "a stream could not be added to the reply");
        }
    }
    OwnedVariant::try_from(Variant::Array(array)).expect("an array of streams is a value")
}
