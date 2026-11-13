//! The consent the portal asserts, and what the compositor lets it unlock.
//!
//! The portal process asks the user. The compositor enforces: it holds the EIS endpoint
//! and the capture table, and neither does anything for an app until the user's answer
//! has reached it here. Two flows, one rule: **no recorded consent, nothing opens.**
//!
//! | Flow | Portal's call | What the compositor then does |
//! | --- | --- | --- |
//! | Remote desktop, yes | `remote.grant {session, devices}` | records the approval and lets the endpoint advertise exactly those devices to that session's connection |
//! | Remote desktop, no | `remote.refuse {session}` | records the refusal; the session never gets a socket |
//! | Remote desktop, connect | `remote.connect {session}` | opens a one-shot EIS socket for the approved session and answers its path; any other session is refused |
//! | Remote desktop, end | `remote.end {session}` | ends the session's EIS connection after releasing what it held |
//! | Screen cast, yes | `cast.grant {session, source}` | records the approval of that one source: an output, a View named by its node id, or a window named by its identity (refused `no-consent` unless the user was asked about that very window by a `portal.ui` request of this connection and allowed it) |
//! | Screen cast, start | `cast.start {session}` | starts the PipeWire stream of the approved output, View or window (the View's picture at the View's size; the window's rectangle at the window's size, following it) and answers its node and where it sits; a session with no approval is refused, and so is one whose output, View or window is not there (`no-output`, `no-view`, `no-window`) |
//! | Screen cast, end | `cast.end {session}` | ends the stream |
//!
//! Every member is the owner's ([`super::server`]'s one owner predicate): the portal is
//! the session owner's own process, and a connection that is not the owner is refused
//! `denied` before this module sees the call.
//!
//! # Who holds a session
//!
//! A session name is the portal's session handle, chosen by the app's frontend. The
//! connection that recorded the first answer for a name holds it: another connection
//! naming it is refused `session-taken` when it tries to decide it and `session-unknown`
//! when it tries to use it, so one connection cannot connect or end what another was
//! asked about. A session ends when its holder ends it, and when the holder's connection
//! goes away, because a portal that crashed took the user's question with it.
//!
//! # Failure modes
//!
//! Each is a test in `tests/eis.rs` (remote desktop) or `tests/cast.rs` (screen cast).
//!
//! 1. **A start with no consent.** `cast.start` and `remote.connect` for a session nobody
//!    answered for are refused `no-consent`, and no stream or socket is made.
//! 2. **A refusal that opens.** A refused remote session is refused `consent-refused` at
//!    `remote.connect`, and its decision is not replaced by a later grant.
//! 3. **One session's answer reaching another.** A grant is per session; a second session
//!    and the endpoint's own socket stay default deny.
//! 4. **A session that outlives its holder.** Ending the session or losing the holder's
//!    connection ends the EIS connection and the stream.

use std::collections::BTreeMap;

use gnoblin_capture::{SessionId, Target};
use gnoblin_client::wire::{self, CastSource, CastStream as WireCast};
use gnoblin_input_eis::{Caps, Region};
use gnoblin_types::{ClientId, NodeId, OutputId, WindowId};
use gnoblin_wayland::{CastRefusal, ControlHandle, SessionSnapshot};

use crate::eis::{EisGate, SessionError};

/// `types` bit of the RemoteDesktop portal: a keyboard.
const DEVICE_KEYBOARD: u32 = 1;
/// `types` bit of the RemoteDesktop portal: a pointer.
const DEVICE_POINTER: u32 = 2;
/// `types` bit of the RemoteDesktop portal: a touch screen.
const DEVICE_TOUCHSCREEN: u32 = 4;

/// A call the compositor refuses: the wire code the portal matches on, and why.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    /// The stable wire code.
    pub code: &'static str,
    /// What happened, for a log and for the portal's error sentence.
    pub message: String,
}

impl Refusal {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn unknown(session: &str) -> Self {
        Self::new(
            wire::code::SESSION_UNKNOWN,
            format!("this connection holds no consent for `{session}`"),
        )
    }

    fn taken(session: &str) -> Self {
        Self::new(
            wire::code::SESSION_TAKEN,
            format!("`{session}` is held by another connection or was already decided"),
        )
    }
}

/// The EIS capabilities the RemoteDesktop portal's device types stand for.
///
/// A pointer is the relative and absolute pointer with its buttons and scroll, because the
/// portal's "pointer" is one device to the app and EI's capabilities are finer than that.
fn capabilities(devices: u32) -> Caps {
    let mut capabilities = Caps::NONE;
    if devices & DEVICE_KEYBOARD != 0 {
        capabilities |= Caps::KEYBOARD;
    }
    if devices & DEVICE_POINTER != 0 {
        capabilities |= Caps::POINTER | Caps::POINTER_ABSOLUTE | Caps::BUTTON | Caps::SCROLL;
    }
    if devices & DEVICE_TOUCHSCREEN != 0 {
        capabilities |= Caps::TOUCH;
    }
    capabilities
}

/// What a refusal of the EIS gate is called on the wire.
fn session_refusal(error: &SessionError) -> Refusal {
    let code = match error {
        SessionError::NotRunning | SessionError::Socket(_) => wire::code::CAPTURE_UNAVAILABLE,
        SessionError::Decided => wire::code::SESSION_TAKEN,
        SessionError::NoConsent => wire::code::NO_CONSENT,
        SessionError::Refused => wire::code::CONSENT_REFUSED,
    };
    Refusal::new(code, error.to_string())
}

/// One remote desktop session this table was told about.
struct Remote {
    /// The connection that recorded its answer.
    holder: ClientId,
}

/// One screen cast session this table was told about.
struct Cast {
    /// The connection that recorded its answer.
    holder: ClientId,
    /// What the user approved: an output (or the first one the compositor has), or a View.
    source: CastSource,
    /// The stream, once it was started.
    started: Option<(SessionId, WireCast)>,
}

/// The sessions the portal recorded answers for, and what each unlocked.
pub(crate) struct Consent {
    eis: EisGate,
    remote: BTreeMap<String, Remote>,
    cast: BTreeMap<String, Cast>,
}

impl Consent {
    /// An empty table, deciding on `eis`.
    pub(crate) fn new(eis: EisGate) -> Self {
        Self {
            eis,
            remote: BTreeMap::new(),
            cast: BTreeMap::new(),
        }
    }

    /// `remote.grant`: the user approved `session` for `devices`.
    pub(crate) fn remote_grant(&mut self, holder: ClientId, session: &str, devices: u32) -> Result<(), Refusal> {
        let capabilities = capabilities(devices);
        if capabilities.is_empty() {
            return Err(Refusal::new(
                wire::code::NO_DEVICES,
                format!("the approval of `{session}` covers no device the compositor serves"),
            ));
        }
        if self.remote.contains_key(session) {
            return Err(Refusal::taken(session));
        }
        self.eis
            .grant_session(session, capabilities)
            .map_err(|error| session_refusal(&error))?;
        self.remote.insert(session.to_owned(), Remote { holder });
        Ok(())
    }

    /// `remote.refuse`: the user refused `session`.
    pub(crate) fn remote_refuse(&mut self, holder: ClientId, session: &str) -> Result<(), Refusal> {
        if self.remote.contains_key(session) {
            return Err(Refusal::taken(session));
        }
        self.eis
            .refuse_session(session)
            .map_err(|error| session_refusal(&error))?;
        self.remote.insert(session.to_owned(), Remote { holder });
        Ok(())
    }

    /// `remote.connect`: the one-shot EIS socket of an approved session.
    pub(crate) fn remote_connect(
        &mut self,
        holder: ClientId,
        session: &str,
        snapshot: &SessionSnapshot,
    ) -> Result<String, Refusal> {
        if self.remote.get(session).is_some_and(|held| held.holder != holder) {
            return Err(Refusal::unknown(session));
        }
        let regions = snapshot
            .outputs()
            .iter()
            .filter_map(|output| {
                snapshot
                    .output_rect(*output)
                    .map(|rect| Region { output: *output, rect })
            })
            .collect();
        let path = self
            .eis
            .connect_session(session, regions)
            .map_err(|error| session_refusal(&error))?;
        Ok(path.to_string_lossy().into_owned())
    }

    /// `remote.end`: `session` is over.
    pub(crate) fn remote_end(&mut self, holder: ClientId, session: &str) -> Result<(), Refusal> {
        match self.remote.get(session) {
            Some(held) if held.holder == holder => {
                self.remote.remove(session);
                self.eis.revoke_session(session);
                Ok(())
            }
            _ => Err(Refusal::unknown(session)),
        }
    }

    /// `cast.grant`: the user approved a cast of `source` for `session`. An output of
    /// [`wire::OUTPUT_FIRST`] names none, and the cast takes the first output when it starts. A View is named by its
    /// node id, and a window by its identity; both are only checked when the stream starts: one that is not there then
    /// is refused `no-view` or `no-window`. That the user was asked about a window is the caller's check
    /// ([`super::portal::PortalService::take_window_consent`]), made before this table hears of it.
    pub(crate) fn cast_grant(&mut self, holder: ClientId, session: &str, source: CastSource) -> Result<(), Refusal> {
        if self.cast.contains_key(session) {
            return Err(Refusal::taken(session));
        }
        self.cast.insert(
            session.to_owned(),
            Cast {
                holder,
                source,
                started: None,
            },
        );
        Ok(())
    }

    /// What an approval of `source` captures now: the output it names, or the first one `snapshot` lists when it
    /// names none, or the View it names. [`None`] when it names no output and the session has none.
    fn target_of(source: CastSource, snapshot: &SessionSnapshot) -> Option<Target> {
        match source {
            CastSource::Output(output) if output != wire::OUTPUT_FIRST => Some(Target::Output(OutputId::new(output))),
            CastSource::Output(_) => snapshot.outputs().first().copied().map(Target::Output),
            CastSource::View(view) => Some(Target::View(NodeId::new(view))),
            CastSource::Window(window) => Some(Target::Window(WindowId::new(window))),
        }
    }

    /// `cast.start`: the PipeWire stream of an approved session.
    ///
    /// Starting twice answers the same node, so a portal that retries a start it did not
    /// hear the answer to does not make a second stream. An approval that named no output
    /// means the first one `snapshot` lists now. An approval of a View streams that View, and is refused when it
    /// is not in the scene.
    pub(crate) fn cast_start(
        &mut self,
        holder: ClientId,
        session: &str,
        snapshot: &SessionSnapshot,
        control: &ControlHandle,
    ) -> Result<WireCast, Refusal> {
        let Some(cast) = self.cast.get_mut(session) else {
            return Err(Refusal::new(
                wire::code::NO_CONSENT,
                format!("no consent was recorded for the screen cast `{session}`"),
            ));
        };
        if cast.holder != holder {
            return Err(Refusal::unknown(session));
        }
        if let Some((_, started)) = &cast.started {
            return Ok(*started);
        }
        let Some(target) = Self::target_of(cast.source, snapshot) else {
            return Err(Refusal::new(
                wire::code::CAPTURE_NO_OUTPUT,
                "there is no output to cast",
            ));
        };
        let stream = match control.start_cast(holder, target) {
            Some(Ok(stream)) => stream,
            Some(Err(CastRefusal::Locked)) => {
                return Err(Refusal::new(wire::code::CAPTURE_LOCKED, "the session is locked"));
            }
            Some(Err(CastRefusal::NoTarget)) => {
                return Err(match target {
                    Target::View(view) => Refusal::new(
                        wire::code::CAPTURE_NO_VIEW,
                        format!("the approved View {} is not in the scene", view.get()),
                    ),
                    Target::Window(window) => Refusal::new(
                        wire::code::CAPTURE_NO_WINDOW,
                        format!("the approved window {} is not shown", window.get()),
                    ),
                    Target::Output(_) => Refusal::new(
                        wire::code::CAPTURE_NO_OUTPUT,
                        format!("the approved target {target} is not there"),
                    ),
                });
            }
            Some(Err(CastRefusal::Unavailable(reason))) => {
                return Err(Refusal::new(wire::code::CAPTURE_UNAVAILABLE, reason));
            }
            None => {
                return Err(Refusal::new(
                    wire::code::CAPTURE_UNAVAILABLE,
                    "the compositor loop did not answer the stream start",
                ));
            }
        };
        let started = WireCast {
            node: stream.node,
            x: stream.origin.x,
            y: stream.origin.y,
            layout_width: stream.extent.width,
            layout_height: stream.extent.height,
            width: stream.size.width,
            height: stream.size.height,
        };
        cast.started = Some((stream.session, started));
        Ok(started)
    }

    /// `cast.end`: `session` is over, and its stream with it.
    pub(crate) fn cast_end(&mut self, holder: ClientId, session: &str, control: &ControlHandle) -> Result<(), Refusal> {
        match self.cast.get(session) {
            Some(held) if held.holder == holder => {
                if let Some((stream, _)) = &held.started {
                    control.stop_cast(*stream);
                }
                self.cast.remove(session);
                Ok(())
            }
            _ => Err(Refusal::unknown(session)),
        }
    }

    /// The holder's connection ended: everything it was asked about ends with it.
    pub(crate) fn departed(&mut self, holder: ClientId, control: &ControlHandle) {
        let remote: Vec<String> = self
            .remote
            .iter()
            .filter(|(_, held)| held.holder == holder)
            .map(|(session, _)| session.clone())
            .collect();
        for session in remote {
            self.remote.remove(&session);
            self.eis.revoke_session(&session);
        }
        let had_cast = self.cast.values().any(|held| held.holder == holder);
        self.cast.retain(|_, held| held.holder != holder);
        if had_cast {
            control.stop_casts_of(holder);
        }
    }

    /// Whether the table holds an answer for a remote desktop session, for a test.
    #[cfg(test)]
    fn holds_remote(&self, session: &str) -> bool {
        self.remote.contains_key(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portal_device_types_map_to_the_eis_capabilities() {
        assert_eq!(capabilities(DEVICE_KEYBOARD), Caps::KEYBOARD);
        assert_eq!(capabilities(DEVICE_TOUCHSCREEN), Caps::TOUCH);
        assert_eq!(
            capabilities(DEVICE_POINTER),
            Caps::POINTER | Caps::POINTER_ABSOLUTE | Caps::BUTTON | Caps::SCROLL,
            "the portal's pointer is the whole pointer"
        );
        assert_eq!(
            capabilities(DEVICE_KEYBOARD | DEVICE_POINTER | DEVICE_TOUCHSCREEN),
            Caps::ALL
        );
        assert!(capabilities(0).is_empty());
        assert!(
            capabilities(8).is_empty(),
            "a type the portal does not define is not a device"
        );
    }

    #[test]
    fn a_grant_that_approves_no_device_is_refused_and_records_nothing() {
        let mut consent = Consent::new(EisGate::default());
        let refusal = consent
            .remote_grant(ClientId::new(1), "/session/1", 0)
            .expect_err("no device is approved");
        assert_eq!(refusal.code, wire::code::NO_DEVICES);
        assert!(!consent.holds_remote("/session/1"));
    }

    #[test]
    fn a_grant_without_an_endpoint_is_unavailable_and_records_nothing() {
        // The default gate has no endpoint: nothing can be decided, so nothing is held.
        let mut consent = Consent::new(EisGate::default());
        let refusal = consent
            .remote_grant(ClientId::new(1), "/session/1", DEVICE_KEYBOARD)
            .expect_err("no endpoint is running");
        assert_eq!(refusal.code, wire::code::CAPTURE_UNAVAILABLE);
        assert!(!consent.holds_remote("/session/1"));
    }

    #[test]
    fn a_connect_for_a_session_nobody_answered_for_has_no_consent() {
        let mut consent = Consent::new(EisGate::default());
        let refusal = consent
            .remote_connect(ClientId::new(1), "/session/1", &SessionSnapshot::default())
            .expect_err("nothing was recorded");
        assert_eq!(refusal.code, wire::code::NO_CONSENT);
    }

    #[test]
    fn ending_a_session_the_connection_does_not_hold_is_refused() {
        let mut consent = Consent::new(EisGate::default());
        let refusal = consent
            .remote_end(ClientId::new(1), "/session/1")
            .expect_err("nothing is held");
        assert_eq!(refusal.code, wire::code::SESSION_UNKNOWN);
    }

    #[test]
    fn a_cast_grant_that_names_no_output_means_the_first_one() {
        let mut consent = Consent::new(EisGate::default());
        consent
            .cast_grant(ClientId::new(1), "/session/1", CastSource::Output(wire::OUTPUT_FIRST))
            .expect("recorded");
        consent
            .cast_grant(ClientId::new(1), "/session/2", CastSource::Output(7))
            .expect("recorded");
        assert_eq!(
            consent.cast["/session/1"].source,
            CastSource::Output(wire::OUTPUT_FIRST)
        );
        assert_eq!(consent.cast["/session/2"].source, CastSource::Output(7));
        assert_eq!(
            Consent::target_of(CastSource::Output(7), &SessionSnapshot::default()),
            Some(Target::Output(OutputId::new(7)))
        );
        assert_eq!(
            Consent::target_of(CastSource::Output(wire::OUTPUT_FIRST), &SessionSnapshot::default()),
            None,
            "a session with no output has no first one"
        );
    }

    #[test]
    fn a_cast_grant_that_names_a_view_is_a_view_target_and_never_an_output() {
        let view = (1 << 32) | 0x102;
        assert_eq!(
            Consent::target_of(CastSource::View(view), &SessionSnapshot::default()),
            Some(Target::View(NodeId::new(view)))
        );
    }

    #[test]
    fn a_cast_grant_that_names_a_window_is_a_window_target_and_never_an_output_or_a_view() {
        let target = Consent::target_of(CastSource::Window(5), &SessionSnapshot::default());
        assert_eq!(target, Some(Target::Window(WindowId::new(5))));
        assert_ne!(target, Some(Target::Output(OutputId::new(5))));
        assert_ne!(target, Some(Target::View(NodeId::new(5))));
    }
}
