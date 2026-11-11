//! The screenshot call, served by the compositor's `capture.screen` member.
//!
//! The compositor sends one PNG of the output and never names a path. This module asks it
//! for the picture, has [`picture`] write it to the user's Pictures folder, and answers with
//! the file's `file://` URI. Only full-screen screenshots are served: an interactive one asks
//! the user to choose an area, which this portal does not offer, so it is refused by name.
//! Colour picking has no compositor member yet, so [`CaptureBackend::pick_color`] refuses.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

use gnoblin_capture::Target;
use gnoblin_client::wire::{CastSource, CastStream};
use gnoblin_model::Value;

use crate::backends::{
    CaptureBackend, ScreenshotError, ScreenshotOptions, Stream, StreamId, StreamRequest, Unavailable,
};
use crate::host::{ConsentAnswer, ConsentCall, IpcHost};
use crate::iface::capture::{SOURCE_MONITOR, SOURCE_VIRTUAL};
use crate::log;
use crate::picture;

/// Where the compositor's pixels come from: its connection, or a fake.
pub trait ScreenSource: Send + Sync {
    /// One PNG of the whole output, or the reason the compositor did not give one.
    fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError>;

    /// Tell the compositor the user approved a cast of `source` for `session` and start its
    /// PipeWire stream. The compositor refuses the start when it holds no such approval.
    fn open_cast(&self, session: &str, source: CastSource) -> Result<CastStream, Unavailable>;

    /// The session ended: its stream ends with it.
    fn close_cast(&self, session: &str);
}

impl ScreenSource for IpcHost {
    fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError> {
        self.capture_screen()
    }

    fn open_cast(&self, session: &str, source: CastSource) -> Result<CastStream, Unavailable> {
        self.consent(ConsentCall::CastGrant {
            session: session.to_owned(),
            source,
        })?;
        let started = self.consent(ConsentCall::CastStart {
            session: session.to_owned(),
        });
        match started {
            Ok(ConsentAnswer::Stream(stream)) => Ok(stream),
            other => {
                // A grant with no stream behind it would hold the session's name for nothing, and
                // the start the app will retry would be refused as already decided.
                self.close_cast(session);
                match other {
                    Err(error) => Err(error),
                    Ok(answer) => Err(Unavailable::refused(format!(
                        "the compositor answered the stream start with {answer:?}"
                    ))),
                }
            }
        }
    }

    fn close_cast(&self, session: &str) {
        let ended = self.consent(ConsentCall::CastEnd {
            session: session.to_owned(),
        });
        if let Err(error) = ended {
            log::debug!(%error, session = %session, "the compositor held no cast to end");
        }
    }
}

/// Take one screenshot with `capture` and write it into `folder`, returning its URI.
///
/// The capture happens before the folder is touched, so a refused capture creates nothing.
///
/// # Errors
///
/// A refusal from `capture` passes through unchanged. A picture that cannot be stored is
/// [`ScreenshotError::Write`], which names the folder or file that failed.
pub fn screenshot_into(
    capture: impl FnOnce() -> Result<Vec<u8>, ScreenshotError>,
    folder: &Path,
) -> Result<String, ScreenshotError> {
    let png = capture()?;
    let path = picture::write_png(&png, folder).map_err(ScreenshotError::Write)?;
    Ok(picture::file_uri(&path))
}

/// The portal's capture backend: full-screen screenshots and screen casts from a [`ScreenSource`].
pub struct CaptureRelay<S: ScreenSource> {
    source: S,
    /// The folder pictures are written to. `None` when the user has no home to name one in.
    folder: Option<PathBuf>,
    /// The last stream number handed out.
    next_stream: AtomicU32,
    /// The portal session each open stream belongs to, so closing a stream ends its cast.
    streams: Mutex<BTreeMap<StreamId, String>>,
}

impl<S: ScreenSource> CaptureRelay<S> {
    /// A relay that takes its pictures and streams from `source` and stores pictures in `folder`.
    pub fn new(source: S, folder: Option<PathBuf>) -> Self {
        CaptureRelay {
            source,
            folder,
            next_stream: AtomicU32::new(0),
            streams: Mutex::new(BTreeMap::new()),
        }
    }
}

impl<S: ScreenSource> CaptureBackend for CaptureRelay<S> {
    fn screenshot(&self, options: &ScreenshotOptions) -> Result<String, ScreenshotError> {
        if options.interactive {
            return Err(ScreenshotError::Unavailable(Unavailable::refused(
                "an interactive screenshot (choosing an area) is not implemented in this portal; request a full-screen one",
            )));
        }
        let Some(folder) = &self.folder else {
            return Err(ScreenshotError::Write(Unavailable::refused(
                "no screenshot folder: HOME and XDG_PICTURES_DIR are both unset",
            )));
        };
        screenshot_into(|| self.source.screen_png(), folder)
    }

    fn pick_color(&self) -> Result<(f64, f64, f64), Unavailable> {
        Err(Unavailable::missing("pick-color"))
    }

    fn open_stream(&self, request: &StreamRequest) -> Result<Stream, Unavailable> {
        let (source, source_type) = match request.target {
            Target::Output(output) => (CastSource::Output(output.get()), SOURCE_MONITOR),
            Target::View(view) => (CastSource::View(view.get()), SOURCE_VIRTUAL),
            Target::Window(_) => {
                return Err(Unavailable::refused(
                    "only an output or a View can be shared in this build; a window is not served",
                ));
            }
        };
        let cast = self.source.open_cast(&request.session, source)?;
        let id = StreamId::new(self.next_stream.fetch_add(1, Ordering::Relaxed).saturating_add(1));
        self.streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, request.session.clone());
        let mut properties = BTreeMap::new();
        properties.insert("source_type".to_owned(), Value::Uint(u64::from(source_type)));
        Ok(Stream {
            id,
            node: cast.node,
            properties,
        })
    }

    fn close_stream(&self, stream: StreamId) {
        let session = self
            .streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&stream);
        if let Some(session) = session {
            self.source.close_cast(&session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n a picture";

    /// A folder under the system temp directory, empty and unique to one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gnoblin-portal-relay-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// A compositor that hands back a fixed picture.
    struct Fixed(Vec<u8>);

    impl ScreenSource for Fixed {
        fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError> {
            Ok(self.0.clone())
        }

        fn open_cast(&self, _session: &str, _source: CastSource) -> Result<CastStream, Unavailable> {
            Err(Unavailable::missing("cast.start"))
        }

        fn close_cast(&self, _session: &str) {}
    }

    /// A compositor that must never be asked: an interactive request is refused first.
    struct NeverAsked;

    impl ScreenSource for NeverAsked {
        fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError> {
            panic!("an interactive request must be refused before the compositor is asked");
        }

        fn open_cast(&self, _session: &str, _source: CastSource) -> Result<CastStream, Unavailable> {
            panic!("a screenshot never opens a cast");
        }

        fn close_cast(&self, _session: &str) {}
    }

    fn full_screen() -> ScreenshotOptions {
        ScreenshotOptions {
            parent_window: String::new(),
            interactive: false,
            modal: false,
        }
    }

    #[test]
    fn a_picture_from_the_compositor_is_written_and_its_uri_returned() {
        let dir = scratch("written");
        let uri = screenshot_into(|| Ok(PNG.to_vec()), &dir).expect("the picture is written");
        let written: Vec<PathBuf> = fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()).collect();
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(fs::read(&written[0]).unwrap(), PNG);
        assert_eq!(uri, picture::file_uri(&written[0]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_screen_request_through_the_relay_writes_the_picture() {
        // Failure mode: the relay answers without asking the compositor, or drops the bytes.
        let dir = scratch("relay");
        let relay = CaptureRelay::new(Fixed(PNG.to_vec()), Some(dir.clone()));
        let uri = relay.screenshot(&full_screen()).expect("the picture is written");
        assert!(uri.starts_with("file://"), "{uri}");
        let written: Vec<PathBuf> = fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()).collect();
        assert_eq!(written.len(), 1, "{written:?}");
        assert_eq!(fs::read(&written[0]).unwrap(), PNG);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unwritable_folder_is_a_named_write_failure() {
        let base = scratch("unwritable");
        fs::write(&base, b"a file where a folder should be").unwrap();
        let dir = base.join("Pictures");
        match screenshot_into(|| Ok(PNG.to_vec()), &dir) {
            Err(ScreenshotError::Write(reason)) => {
                assert!(reason.reason.contains(&dir.display().to_string()), "{reason}");
            }
            other => panic!("expected a named write failure, got {other:?}"),
        }
        let _ = fs::remove_file(&base);
    }

    #[test]
    fn a_relay_with_no_folder_is_a_named_write_failure() {
        // Failure mode: a missing home directory silently writes nowhere, or panics.
        let relay = CaptureRelay::new(Fixed(PNG.to_vec()), None);
        match relay.screenshot(&full_screen()) {
            Err(ScreenshotError::Write(reason)) => assert!(reason.reason.contains("XDG_PICTURES_DIR"), "{reason}"),
            other => panic!("expected a named write failure, got {other:?}"),
        }
    }

    #[test]
    fn a_locked_session_is_refused_and_writes_nothing() {
        let dir = scratch("locked");
        let refused = ScreenshotError::Refused(picture::refusal(
            gnoblin_client::code::CAPTURE_LOCKED,
            "the session is locked",
        ));
        match screenshot_into(|| Err(refused.clone()), &dir) {
            Err(ScreenshotError::Refused(reason)) => assert!(reason.reason.contains("locked"), "{reason}"),
            other => panic!("expected the refusal to pass through, got {other:?}"),
        }
        assert!(!dir.exists(), "a refused capture must not create the folder");
    }

    #[test]
    fn an_interactive_request_is_refused_by_name_and_takes_no_picture() {
        // Failure mode: an interactive request silently takes a full-screen picture instead.
        let dir = scratch("interactive");
        let relay = CaptureRelay::new(NeverAsked, Some(dir.clone()));
        let interactive = ScreenshotOptions {
            interactive: true,
            ..full_screen()
        };
        match relay.screenshot(&interactive) {
            Err(ScreenshotError::Unavailable(reason)) => assert!(reason.reason.contains("interactive"), "{reason}"),
            other => panic!("expected a named refusal, got {other:?}"),
        }
        assert!(!dir.exists());
    }

    #[test]
    fn an_empty_picture_is_a_write_failure_not_a_screenshot() {
        let dir = scratch("empty");
        assert!(matches!(
            screenshot_into(|| Ok(Vec::new()), &dir),
            Err(ScreenshotError::Write(_))
        ));
    }

    /// A compositor that records what it was told the user approved, and answers a stream.
    #[derive(Default)]
    struct Recording {
        sources: Mutex<Vec<CastSource>>,
    }

    impl ScreenSource for Recording {
        fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError> {
            panic!("a cast never takes a screenshot");
        }

        fn open_cast(&self, _session: &str, source: CastSource) -> Result<CastStream, Unavailable> {
            self.sources.lock().unwrap().push(source);
            Ok(CastStream {
                node: 42,
                x: 0,
                y: 0,
                width: 64,
                height: 48,
            })
        }

        fn close_cast(&self, _session: &str) {}
    }

    fn request(target: Target) -> StreamRequest {
        StreamRequest {
            session: "/session/1".to_owned(),
            target,
            cursor_mode: 2,
            multiple: false,
        }
    }

    #[test]
    fn a_view_target_is_approved_to_the_compositor_as_a_view_and_streams_as_a_virtual_source() {
        let relay = CaptureRelay::new(Recording::default(), None);
        let view = (1 << 32) | 0x102;
        let stream = relay
            .open_stream(&request(Target::View(gnoblin_types::NodeId::new(view))))
            .expect("a View is served");
        assert_eq!(stream.node, 42);
        assert_eq!(
            relay.source.sources.lock().unwrap().as_slice(),
            [CastSource::View(view)],
            "the full node id reaches the compositor, epoch included"
        );
        assert_eq!(
            stream.properties.get("source_type"),
            Some(&Value::Uint(u64::from(SOURCE_VIRTUAL)))
        );

        let output = relay
            .open_stream(&request(Target::Output(gnoblin_types::OutputId::new(3))))
            .expect("an output is served");
        assert_eq!(
            output.properties.get("source_type"),
            Some(&Value::Uint(u64::from(SOURCE_MONITOR)))
        );
        assert_eq!(
            relay.source.sources.lock().unwrap().last(),
            Some(&CastSource::Output(3))
        );
    }

    #[test]
    fn a_window_target_is_refused_before_the_compositor_is_asked() {
        let relay = CaptureRelay::new(Recording::default(), None);
        let refused = relay
            .open_stream(&request(Target::Window(gnoblin_types::WindowId::new(1))))
            .expect_err("a window is not served");
        assert!(refused.reason.contains("window"), "{refused}");
        assert!(relay.source.sources.lock().unwrap().is_empty());
    }
}
