//! The input call, served by the compositor's `remote.*` members.
//!
//! The portal asks the user; the compositor enforces the answer. This module is the whole of
//! what the portal does about an answer: it tells the compositor what the user said
//! ([`EisBackend::grant`], [`EisBackend::refuse`]), asks for the descriptor of an approved
//! session ([`EisBackend::connect`]) and tells it when the session is over
//! ([`EisBackend::end`]).
//!
//! The descriptor is not made here. The compositor opens a one-shot socket for the session
//! and answers its path; this module connects to it and hands the connected stream to the
//! frontend as the app's descriptor, so the app speaks libei straight to the compositor's
//! EIS server. The compositor refuses the path for a session it was not told the user
//! approved, so a portal that skipped `grant` has no descriptor to hand out.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;

use crate::backends::{EisBackend, Unavailable};
use crate::host::{ConsentAnswer, ConsentCall, IpcHost};
use crate::log;

/// Where a consent call goes: the compositor's connection, or a fake.
pub trait ConsentSink: Send + Sync {
    /// Make one call of the compositor and answer what it said.
    ///
    /// # Errors
    ///
    /// The compositor's refusal, with its code, or a connection that closed.
    fn call(&self, call: ConsentCall) -> Result<ConsentAnswer, Unavailable>;
}

impl ConsentSink for IpcHost {
    fn call(&self, call: ConsentCall) -> Result<ConsentAnswer, Unavailable> {
        self.consent(call)
    }
}

/// The portal's input backend: every call is a `remote.*` member of the compositor.
pub struct EisRelay<S: ConsentSink> {
    sink: S,
}

impl<S: ConsentSink> EisRelay<S> {
    /// A relay that makes its calls of `sink`.
    pub fn new(sink: S) -> Self {
        EisRelay { sink }
    }
}

impl<S: ConsentSink> EisBackend for EisRelay<S> {
    fn grant(&self, session: &str, devices: u32) -> Result<(), Unavailable> {
        self.sink
            .call(ConsentCall::RemoteGrant {
                session: session.to_owned(),
                devices,
            })
            .map(drop)
    }

    fn refuse(&self, session: &str) {
        let told = self.sink.call(ConsentCall::RemoteRefuse {
            session: session.to_owned(),
        });
        if let Err(error) = told {
            log::warn!(session = %session, %error, "the compositor was not told the user refused");
        }
    }

    fn connect(&self, session: &str, _app_id: &str) -> Result<OwnedFd, Unavailable> {
        let answer = self.sink.call(ConsentCall::RemoteConnect {
            session: session.to_owned(),
        })?;
        let ConsentAnswer::Socket(path) = answer else {
            return Err(Unavailable::refused(format!(
                "the compositor answered `remote.connect` with {answer:?}, not a socket"
            )));
        };
        let stream = UnixStream::connect(&path)
            .map_err(|error| Unavailable::refused(format!("the EIS socket `{path}` could not be reached: {error}")))?;
        Ok(OwnedFd::from(stream))
    }

    fn end(&self, session: &str) {
        let told = self.sink.call(ConsentCall::RemoteEnd {
            session: session.to_owned(),
        });
        if let Err(error) = told {
            // The compositor holds nothing for a session that was never approved, or one that
            // already ended with the connection that held it; neither is a problem to report.
            log::debug!(session = %session, %error, "the compositor held no EIS session to end");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A compositor that records every call and answers from a script.
    #[derive(Clone, Default)]
    struct Recorder {
        calls: Arc<Mutex<Vec<ConsentCall>>>,
        answer: Arc<Mutex<Option<Result<ConsentAnswer, Unavailable>>>>,
    }

    impl Recorder {
        fn answering(answer: Result<ConsentAnswer, Unavailable>) -> Self {
            let recorder = Recorder::default();
            *recorder.answer.lock().unwrap() = Some(answer);
            recorder
        }

        fn calls(&self) -> Vec<ConsentCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl ConsentSink for Recorder {
        fn call(&self, call: ConsentCall) -> Result<ConsentAnswer, Unavailable> {
            self.calls.lock().unwrap().push(call);
            self.answer.lock().unwrap().clone().unwrap_or(Ok(ConsentAnswer::Done))
        }
    }

    #[test]
    fn a_grant_tells_the_compositor_which_devices_the_user_approved() {
        let recorder = Recorder::default();
        EisRelay::new(recorder.clone())
            .grant("/session/1", 3)
            .expect("the compositor took it");
        assert_eq!(
            recorder.calls(),
            vec![ConsentCall::RemoteGrant {
                session: "/session/1".to_owned(),
                devices: 3
            }]
        );
    }

    #[test]
    fn a_grant_the_compositor_refuses_is_not_reported_as_one() {
        let recorder = Recorder::answering(Err(Unavailable::refused("no endpoint")));
        let refusal = EisRelay::new(recorder).grant("/session/1", 1).expect_err("refused");
        assert!(refusal.reason.contains("no endpoint"), "{refusal}");
    }

    #[test]
    fn a_refusal_and_an_end_are_reported_to_the_compositor() {
        let recorder = Recorder::default();
        let relay = EisRelay::new(recorder.clone());
        relay.refuse("/session/1");
        relay.end("/session/1");
        assert_eq!(
            recorder.calls(),
            vec![
                ConsentCall::RemoteRefuse {
                    session: "/session/1".to_owned()
                },
                ConsentCall::RemoteEnd {
                    session: "/session/1".to_owned()
                },
            ]
        );
    }

    #[test]
    fn a_connect_hands_back_a_stream_connected_to_the_sessions_socket() {
        let directory = tempfile_dir("connect");
        let path = directory.join("eis-1.sock");
        let listener = UnixListener::bind(&path).expect("the session's socket");
        let recorder = Recorder::answering(Ok(ConsentAnswer::Socket(path.display().to_string())));
        let descriptor = EisRelay::new(recorder)
            .connect("/session/1", "org.example.App")
            .expect("a descriptor");
        let (peer, _) = listener.accept().expect("the compositor's side accepts");
        drop(peer);
        drop(descriptor);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_connect_the_compositor_refuses_hands_back_no_descriptor() {
        // The failure mode: a descriptor for a session the user did not approve.
        let recorder = Recorder::answering(Err(Unavailable::refused("no-consent")));
        let refusal = EisRelay::new(recorder)
            .connect("/session/1", "org.example.App")
            .expect_err("no descriptor without consent");
        assert!(refusal.reason.contains("no-consent"), "{refusal}");
    }

    #[test]
    fn a_socket_that_cannot_be_reached_is_a_named_refusal() {
        let recorder = Recorder::answering(Ok(ConsentAnswer::Socket("/nonexistent/eis.sock".to_owned())));
        let refusal = EisRelay::new(recorder)
            .connect("/session/1", "org.example.App")
            .expect_err("nothing listens there");
        assert!(refusal.reason.contains("/nonexistent/eis.sock"), "{refusal}");
    }

    /// A directory under the system temp directory, empty and unique to one test.
    fn tempfile_dir(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!("gnoblin-portal-eis-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        directory
    }
}
