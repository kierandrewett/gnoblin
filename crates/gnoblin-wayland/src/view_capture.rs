//! `View:capture(path)`: one PNG of a View's picture, written where the shell names.
//!
//! The shell calls `capture` on a View node with the path as the call's text. The compositor answers on the call's
//! token with the path it wrote, or with an error code and an empty result. The call is a still and not a stream:
//! streams of a View are the `gnoblin_view_source_v1` protocol's, for the clients that hold the `view` role, and
//! the portal's. A still takes the same road a stream does, so no rule can be skipped:
//!
//! - it is a session in the capture table (`Target::View`, owned by the compositor), so the privacy indicator's
//!   count of open sessions ([`crate::SessionSnapshot::capture_sessions`]) is one higher from before the picture is
//!   read until after the file is written, and a reader that waits for the count sees the capture run;
//! - the lock's answer is asked at the copy ([`gnoblin_capture::Table::capture`]), so a locked or embargoed session
//!   writes nothing and answers `locked`;
//! - the picture is the one the composer drew for the View on the frame in progress, so it holds exactly what the
//!   View shows, and a `secure` node in its source was never drawn into it.
//!
//! The path is the shell's, and the shell is trusted, but a path is still checked: it must be absolute, free of `.` and
//! `..`, name a `.png` file whose directory exists inside the session's runtime directory (the directory of the
//! Wayland socket), and name no directory or link. The file is written to a name of its own and renamed over the
//! target with mode `0600`, so a half-written picture is never seen under the target's name and a link at the target
//! is replaced, never followed.
//!
//! A View has no cursor: `capture` takes no cursor flag, and the picture never shows one.
//!
//! Every capture leaves an audit record: the View, the path and the outcome, at `info`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use gnoblin_capture::still;
use gnoblin_capture::{BufferId, CaptureOutcome, CaptureSource, Copied, Format, PixelFormat, Refusal, Target};
use gnoblin_lock::CaptureStream;
use gnoblin_render::transaction::TreeCall;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{ClientId, NodeId, Rect};

use crate::{State, log};

/// The owner of a still's capture session: the compositor itself. Connection identities start at 1.
const STILL_OWNER: ClientId = ClientId::new(0);

/// The most bytes a PNG may take. A View is a few megapixels at the largest, so this only stops a runaway picture.
const MAX_PNG_BYTES: usize = 128 << 20;

/// The error codes the shell is answered with.
mod code {
    /// The session is locked, or the lock embargoes capture.
    pub const LOCKED: &str = "locked";
    /// The View has no picture (no source, or a source that cannot be drawn), or stills have nowhere to go.
    pub const UNAVAILABLE: &str = "unavailable";
    /// The path is not one the compositor will write.
    pub const BAD_VALUE: &str = "bad-value";
    /// The picture does not fit the limit.
    pub const TOO_LARGE: &str = "too-large";
    /// The file could not be written.
    pub const IO: &str = "io-error";
}

/// The picture of a View, offered to the capture table as the one target it can copy.
struct StillSource<'a> {
    /// The View the session captures.
    view: NodeId,
    /// What it drew.
    picture: &'a Image,
    /// The encoding the copy made, when it ran.
    encoded: Option<Result<Vec<u8>, still::Refusal>>,
}

impl CaptureSource for StillSource<'_> {
    fn bounds(&self, target: Target) -> Option<Rect> {
        (target == Target::View(self.view)).then(|| Rect::from_size(self.picture.size()))
    }

    fn copy(&mut self, target: Target, region: Rect, _format: Format) -> Copied {
        if target != Target::View(self.view) || region != Rect::from_size(self.picture.size()) {
            return Copied::Nothing;
        }
        let encoded = still::encode_png(
            self.picture.size(),
            self.picture.stride(),
            self.picture.data(),
            MAX_PNG_BYTES,
        );
        let copied = if encoded.is_ok() {
            Copied::Copied
        } else {
            Copied::Nothing
        };
        self.encoded = Some(encoded);
        copied
    }
}

/// The file a still is written to: `requested` checked against the runtime directory `root`.
fn still_path(root: &Path, requested: &str) -> Result<PathBuf, &'static str> {
    let path = Path::new(requested);
    let clean = path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
        && path.extension().is_some_and(|extension| extension == "png");
    let (true, Some(name), Some(parent)) = (clean, path.file_name(), path.parent()) else {
        return Err(code::BAD_VALUE);
    };
    let parent = fs::canonicalize(parent).map_err(|_| code::BAD_VALUE)?;
    let root = fs::canonicalize(root).map_err(|_| code::UNAVAILABLE)?;
    if !parent.starts_with(&root) {
        return Err(code::BAD_VALUE);
    }
    let target = parent.join(name);
    match fs::symlink_metadata(&target) {
        Ok(existing) if !existing.file_type().is_file() => Err(code::BAD_VALUE),
        _ => Ok(target),
    }
}

/// Writes `png` to `target` through a file of its own, renamed over the target.
fn write_still(target: &Path, png: &[u8]) -> std::io::Result<()> {
    let name = target.file_name().and_then(|name| name.to_str()).unwrap_or("view");
    let staged = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)
        .and_then(|mut file| file.write_all(png).and_then(|()| file.sync_all()))
        .and_then(|()| fs::rename(&staged, target));
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    written
}

impl State {
    /// Answers the shell's `capture` calls on View nodes, in op order: each writes a PNG and answers its path, or answers
    /// the reason it did not.
    pub(crate) fn run_view_captures(&mut self, calls: Vec<TreeCall>) {
        for call in calls {
            let requested = call.text.expose().to_owned();
            match self.capture_view_still(call.node, &requested) {
                Ok(path) => {
                    log::info!(area = "capture", view = ?call.node, path = %path, "a View was captured");
                    self.tree.answer(call.token, path, String::new());
                }
                Err(error) => {
                    log::info!(area = "capture", view = ?call.node, path = %requested, error, "a View capture was refused");
                    self.tree.answer(call.token, String::new(), error.to_owned());
                }
            }
        }
    }

    /// One still of `view` at `requested`: the path written, or the error code.
    ///
    /// The capture session is open from before the picture is read until the file is written, and the snapshot is
    /// published at both edges, so the privacy indicator's signal precedes the pixels.
    fn capture_view_still(&mut self, view: NodeId, requested: &str) -> Result<String, &'static str> {
        if self.lock.state() != gnoblin_lock::State::Unlocked || !self.capture_allowed() {
            return Err(code::LOCKED);
        }
        let root = self.capture.still_root().ok_or(code::UNAVAILABLE)?.to_owned();
        let target = still_path(&root, requested)?;
        let picture: Arc<Image> = self
            .composer
            .view_pictures()
            .into_iter()
            .find_map(|(node, picture)| (node == view).then_some(picture))
            .ok_or(code::UNAVAILABLE)?;
        let size = picture.size();
        let stride = u32::try_from(picture.stride()).map_err(|_| code::TOO_LARGE)?;
        let format = Format::new(size, stride, PixelFormat::Argb8888).ok_or(code::UNAVAILABLE)?;

        let session = self
            .capture
            .table
            .open(STILL_OWNER, Target::View(view), CaptureStream::Requested, format);
        // The indicator's signal comes before the pixels are read.
        self.publish();
        let outcome = self.copy_still(session, view, &picture);
        let written = outcome.and_then(|png| write_still(&target, &png).map_err(|_| code::IO));
        self.capture.table.close(session);
        self.publish();
        written.map(|()| target.to_string_lossy().into_owned())
    }

    /// Takes the session's one frame through the table's rules and answers the PNG it copied.
    fn copy_still(
        &mut self,
        session: gnoblin_capture::SessionId,
        view: NodeId,
        picture: &Image,
    ) -> Result<Vec<u8>, &'static str> {
        let tick = self.capture.tick();
        let claim = self
            .capture
            .table
            .request(session, BufferId::new(0), tick)
            .map_err(|_| code::UNAVAILABLE)?;
        let mut source = StillSource {
            view,
            picture,
            encoded: None,
        };
        let capture = self.lock_capture(CaptureStream::Requested);
        match self.capture.table.capture(claim, tick, capture, &mut source) {
            CaptureOutcome::Copied(_) => match source.encoded {
                Some(Ok(png)) => Ok(png),
                _ => Err(code::UNAVAILABLE),
            },
            CaptureOutcome::Refused(Refusal::Locked | Refusal::Stopped(_)) => Err(code::LOCKED),
            CaptureOutcome::Refused(_) => match source.encoded {
                Some(Err(still::Refusal::TooLarge)) => Err(code::TOO_LARGE),
                _ => Err(code::UNAVAILABLE),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tempfile::TempDir {
        tempfile::tempdir().expect("a runtime directory")
    }

    #[test]
    fn a_png_inside_the_runtime_directory_is_accepted() {
        let root = runtime();
        let wanted = root.path().join("view.png");
        let got = still_path(root.path(), wanted.to_str().unwrap()).expect("accepted");
        assert_eq!(got, fs::canonicalize(root.path()).unwrap().join("view.png"));
    }

    #[test]
    fn a_path_that_is_relative_or_climbs_is_refused() {
        let root = runtime();
        let inside = root.path().join("sub");
        fs::create_dir(&inside).unwrap();
        for requested in [
            "view.png".to_owned(),
            "./view.png".to_owned(),
            format!("{}/sub/../view.png", root.path().display()),
            String::new(),
        ] {
            assert_eq!(
                still_path(root.path(), &requested),
                Err(code::BAD_VALUE),
                "{requested:?} is refused"
            );
        }
    }

    #[test]
    fn a_path_outside_the_runtime_directory_is_refused() {
        let root = runtime();
        let elsewhere = runtime();
        let wanted = elsewhere.path().join("view.png");
        assert_eq!(still_path(root.path(), wanted.to_str().unwrap()), Err(code::BAD_VALUE));
    }

    #[test]
    fn a_link_out_of_the_runtime_directory_is_not_followed() {
        let root = runtime();
        let elsewhere = runtime();
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("out")).unwrap();
        let through = root.path().join("out").join("view.png");
        assert_eq!(
            still_path(root.path(), through.to_str().unwrap()),
            Err(code::BAD_VALUE),
            "a directory link that leaves the runtime directory is refused"
        );
        let secret = elsewhere.path().join("secret.png");
        fs::write(&secret, b"mine").unwrap();
        let link = root.path().join("view.png");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert_eq!(
            still_path(root.path(), link.to_str().unwrap()),
            Err(code::BAD_VALUE),
            "a link at the target is refused"
        );
    }

    #[test]
    fn a_name_that_is_not_a_png_or_is_a_directory_is_refused() {
        let root = runtime();
        let text = root.path().join("view.txt");
        assert_eq!(still_path(root.path(), text.to_str().unwrap()), Err(code::BAD_VALUE));
        let directory = root.path().join("folder.png");
        fs::create_dir(&directory).unwrap();
        assert_eq!(
            still_path(root.path(), directory.to_str().unwrap()),
            Err(code::BAD_VALUE)
        );
        let missing = root.path().join("absent").join("view.png");
        assert_eq!(still_path(root.path(), missing.to_str().unwrap()), Err(code::BAD_VALUE));
    }

    #[test]
    fn a_still_replaces_the_file_in_one_step_and_stays_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = runtime();
        let target = root.path().join("view.png");
        fs::write(&target, b"old").unwrap();
        write_still(&target, b"new").expect("written");
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o600);
        let leftovers: Vec<_> = fs::read_dir(root.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no staging file is left behind");
    }
}
