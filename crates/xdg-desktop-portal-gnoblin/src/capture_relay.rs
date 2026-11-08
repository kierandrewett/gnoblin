//! The screenshot call, served by the compositor's `capture.screen` member.
//!
//! The compositor sends one PNG of the output and never names a path. This module asks it
//! for the picture, has [`picture`] write it to the user's Pictures folder, and answers with
//! the file's `file://` URI. Only full-screen screenshots are served: an interactive one asks
//! the user to choose an area, which this portal does not offer, so it is refused by name.
//! Colour picking has no compositor member yet, so [`CaptureBackend::pick_color`] refuses.

use std::path::{Path, PathBuf};

use crate::backends::{
    CaptureBackend, ScreenshotError, ScreenshotOptions, Stream, StreamId, StreamRequest, Unavailable,
};
use crate::host::IpcHost;
use crate::picture;

/// Where the PNG of the whole output comes from: the compositor connection, or a fake.
pub trait ScreenSource: Send + Sync {
    /// One PNG of the whole output, or the reason the compositor did not give one.
    fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError>;
}

impl ScreenSource for IpcHost {
    fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError> {
        self.capture_screen()
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

/// The portal's capture backend: full-screen screenshots from a [`ScreenSource`].
pub struct CaptureRelay<S: ScreenSource> {
    source: S,
    /// The folder pictures are written to. `None` when the user has no home to name one in.
    folder: Option<PathBuf>,
}

impl<S: ScreenSource> CaptureRelay<S> {
    /// A relay that takes its pictures from `source` and stores them in `folder`.
    pub fn new(source: S, folder: Option<PathBuf>) -> Self {
        CaptureRelay { source, folder }
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

    fn open_stream(&self, _request: &StreamRequest) -> Result<Stream, Unavailable> {
        Err(Unavailable::missing("capture-open-stream"))
    }

    fn close_stream(&self, _stream: StreamId) {}
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
    }

    /// A compositor that must never be asked: an interactive request is refused first.
    struct NeverAsked;

    impl ScreenSource for NeverAsked {
        fn screen_png(&self) -> Result<Vec<u8>, ScreenshotError> {
            panic!("an interactive request must be refused before the compositor is asked");
        }
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
}
